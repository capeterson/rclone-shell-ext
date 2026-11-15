// SPDX-License-Identifier: GPL-3.0-only
//! `DetailsModel`: the `QObject` behind `Details.qml`. Work runs in stages on plain threads and the
//! results are queued back onto the Qt thread; the loading screen stays up until every stage has
//! landed and for at least [`MIN_LOADING`]. After that the transfer counts and pin state refresh
//! every [`REFRESH`].

use core::pin::Pin;
use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use rclone_ui_core::config;
use rclone_ui_core::keeper;
use rclone_ui_core::share;
use rclone_ui_core::status;
use rclone_ui_view::details::{offline_snapshot, parent_dir, DetailsState, DetailsView};
use rclone_ui_view::{remaining_loading, REFRESH};
use std::sync::Mutex;
use std::time::Instant;

/// The paths this process was started with (`run_details`); `load` falls back to them when QML
/// passes none.
static INITIAL_PATHS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn set_initial_paths(paths: &[String]) {
    *INITIAL_PATHS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = paths.to_vec();
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, loading)]
        #[qproperty(QString, error)]
        #[qproperty(QString, name)]
        #[qproperty(QString, path)]
        #[qproperty(bool, is_dir)]
        #[qproperty(QString, icon_name)]
        #[qproperty(QString, sync_label)]
        #[qproperty(QString, sync_icon)]
        #[qproperty(QString, sync_detail)]
        #[qproperty(QString, drive_modified)]
        #[qproperty(QString, cache_state)]
        #[qproperty(QString, size)]
        #[qproperty(QString, version)]
        #[qproperty(QString, uploading)]
        #[qproperty(QString, downloading)]
        #[qproperty(QString, cached)]
        #[qproperty(QString, cached_label)]
        #[qproperty(bool, shared)]
        #[qproperty(QString, sharing_summary)]
        #[qproperty(QString, sharing_note)]
        #[qproperty(QString, sharing_note_icon)]
        #[qproperty(QString, permissions_json)]
        #[qproperty(QString, manage_label)]
        #[qproperty(bool, manage_enabled)]
        #[qproperty(bool, open_is_menu)]
        #[qproperty(bool, offline_enabled)]
        #[qproperty(QString, offline_title)]
        #[qproperty(QString, offline_caption)]
        #[qproperty(QString, offline_action)]
        #[qproperty(QString, pinned_phase_label)]
        #[qproperty(QString, offline_error)]
        type DetailsModel = super::DetailsModelRust;

        /// Start loading. An empty list means the paths this process was started with.
        #[qinvokable]
        fn load(self: Pin<&mut DetailsModel>, paths: &QStringList);

        /// Open the item's folder (the item itself for a folder, its parent for a file).
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: &DetailsModel);

        /// Open the file with its default app.
        #[qinvokable]
        #[cxx_name = "openFile"]
        fn open_file(self: &DetailsModel);

        /// Open the sharing page in the browser.
        #[qinvokable]
        #[cxx_name = "manageSharing"]
        fn manage_sharing(self: &DetailsModel);

        /// Turn "Keep offline" on or off for this folder.
        #[qinvokable]
        #[cxx_name = "keepOffline"]
        fn keep_offline(self: Pin<&mut DetailsModel>, on: bool);
    }

    impl cxx_qt::Threading for DetailsModel {}
}

#[allow(clippy::struct_excessive_bools)] // one per bound property
pub struct DetailsModelRust {
    loading: bool,
    error: QString,
    name: QString,
    path: QString,
    is_dir: bool,
    icon_name: QString,
    sync_label: QString,
    sync_icon: QString,
    sync_detail: QString,
    drive_modified: QString,
    cache_state: QString,
    size: QString,
    version: QString,
    uploading: QString,
    downloading: QString,
    cached: QString,
    cached_label: QString,
    shared: bool,
    sharing_summary: QString,
    sharing_note: QString,
    sharing_note_icon: QString,
    permissions_json: QString,
    manage_label: QString,
    manage_enabled: bool,
    open_is_menu: bool,
    offline_enabled: bool,
    offline_title: QString,
    offline_caption: QString,
    offline_action: QString,
    pinned_phase_label: QString,
    offline_error: QString,
    /// What has arrived so far (only touched on the Qt thread).
    state: Option<DetailsState>,
}

impl Default for DetailsModelRust {
    fn default() -> Self {
        let empty = QString::default;
        DetailsModelRust {
            loading: true,
            error: empty(),
            name: empty(),
            path: empty(),
            is_dir: false,
            icon_name: empty(),
            sync_label: empty(),
            sync_icon: empty(),
            sync_detail: empty(),
            drive_modified: empty(),
            cache_state: empty(),
            size: empty(),
            version: empty(),
            uploading: empty(),
            downloading: empty(),
            cached: empty(),
            cached_label: empty(),
            shared: false,
            sharing_summary: empty(),
            sharing_note: empty(),
            sharing_note_icon: empty(),
            permissions_json: empty(),
            manage_label: empty(),
            manage_enabled: false,
            open_is_menu: false,
            offline_enabled: false,
            offline_title: empty(),
            offline_caption: empty(),
            offline_action: QString::from("none"),
            pinned_phase_label: empty(),
            offline_error: empty(),
            state: None,
        }
    }
}

type Model = qobject::DetailsModel;

fn paths_of(list: &QStringList) -> Vec<String> {
    list.iter().map(String::from).collect()
}

/// Run `f` on the model's state on the Qt thread, then refresh the bound properties. `false`
/// once the model is gone, which is the worker threads' cue to stop.
fn post(qt: &CxxQtThread<Model>, f: impl FnOnce(&mut DetailsState) + Send + 'static) -> bool {
    qt.queue(move |model| model.update(f)).is_ok()
}

impl qobject::DetailsModel {
    fn update(mut self: Pin<&mut Self>, f: impl FnOnce(&mut DetailsState)) {
        let view = {
            let rust = self.as_mut().rust_mut().get_mut();
            let Some(state) = rust.state.as_mut() else {
                return;
            };
            f(state);
            state.view()
        };
        self.show(&view);
    }

    fn show(mut self: Pin<&mut Self>, v: &DetailsView) {
        let q = |s: &str| QString::from(s);
        self.as_mut().set_error(q(&v.error));
        self.as_mut().set_name(q(&v.name));
        self.as_mut().set_path(q(&v.path));
        self.as_mut().set_is_dir(v.is_dir);
        self.as_mut().set_icon_name(q(&v.icon_name));
        self.as_mut().set_sync_label(q(&v.sync_label));
        self.as_mut().set_sync_icon(q(&v.sync_icon));
        self.as_mut().set_sync_detail(q(&v.sync_detail));
        self.as_mut().set_drive_modified(q(&v.drive_modified));
        self.as_mut().set_cache_state(q(&v.cache_state));
        self.as_mut().set_size(q(&v.size));
        self.as_mut().set_version(q(&v.version));
        self.as_mut().set_uploading(q(&v.uploading));
        self.as_mut().set_downloading(q(&v.downloading));
        self.as_mut().set_cached(q(&v.cached));
        self.as_mut().set_cached_label(q(&v.cached_label));
        self.as_mut().set_shared(v.shared);
        self.as_mut().set_sharing_summary(q(&v.sharing_summary));
        self.as_mut().set_sharing_note(q(&v.sharing_note));
        self.as_mut().set_sharing_note_icon(q(&v.sharing_note_icon));
        self.as_mut().set_permissions_json(q(&v.permissions_json));
        self.as_mut().set_manage_label(q(&v.manage_label));
        self.as_mut().set_manage_enabled(v.manage_enabled);
        self.as_mut().set_open_is_menu(v.open_is_menu);
        self.as_mut().set_offline_enabled(v.offline_enabled);
        self.as_mut().set_offline_title(q(&v.offline_title));
        self.as_mut().set_offline_caption(q(&v.offline_caption));
        self.as_mut().set_offline_action(q(&v.offline_action));
        self.as_mut()
            .set_pinned_phase_label(q(&v.pinned_phase_label));
        self.as_mut().set_offline_error(q(&v.offline_error));
    }

    pub fn load(mut self: Pin<&mut Self>, paths: &QStringList) {
        if self.rust().state.is_some() {
            return; // already loading
        }
        let mut paths = paths_of(paths);
        if paths.is_empty() {
            paths.clone_from(
                &INITIAL_PATHS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
        let cfg = config::store().load();
        let state = DetailsState::new(cfg.clone(), &paths);
        let view = state.view();
        self.as_mut().rust_mut().get_mut().state = Some(state);
        self.as_mut().set_loading(true);
        self.as_mut().show(&view);

        let qt = self.qt_thread();
        std::thread::spawn(move || run_stages(&qt, &cfg, &paths));
    }

    pub fn open_folder(&self) {
        let path = self.path().to_string();
        if *self.is_dir() {
            share::open_url(&path);
        } else {
            share::open_url(&parent_dir(&path));
        }
        // The opener is launched detached, so there is nothing left to wait for.
        std::process::exit(0);
    }

    pub fn open_file(&self) {
        share::open_url(&self.path().to_string());
        std::process::exit(0);
    }

    pub fn manage_sharing(&self) {
        if let Some(t) = self.rust().state.as_ref().and_then(|s| s.target.ok()) {
            share::open_url(&share::sharing_url(t));
        }
        std::process::exit(0);
    }

    pub fn keep_offline(self: Pin<&mut Self>, on: bool) {
        let path = self.path().to_string();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let result = keeper::set_pinned(&path, on);
            let snap = result.is_ok().then(|| offline_snapshot(&path));
            post(&qt, move |s| match result {
                Ok(_) => {
                    s.offline_error = None;
                    if let Some(snap) = snap {
                        s.apply_snapshot(snap);
                    }
                }
                Err(e) => s.offline_error = Some(e),
            });
        });
    }
}

/// The worker: resolve, then the rclone and Drive lookups side by side, then hold the loading
/// screen for its minimum time, then refresh until the window closes.
fn run_stages(qt: &CxxQtThread<Model>, cfg: &config::Config, paths: &[String]) {
    let started = Instant::now();
    let target = share::resolve(cfg, paths);
    let resolved = target.clone();
    if !post(qt, move |s| s.apply_target(resolved)) {
        return;
    }
    if let Ok(target) = &target {
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let r = status::local(cfg, target);
                post(qt, move |s| s.apply_local(r));
            });
            scope.spawn(|| {
                let r = rclone_ui_core::drive::fetch(cfg, target);
                post(qt, move |s| s.apply_drive(r));
            });
        });
    }
    // Every stage has landed (a failed resolve filled the others in already).
    std::thread::sleep(remaining_loading(started.elapsed()));
    if qt
        .queue(|mut model| model.as_mut().set_loading(false))
        .is_err()
    {
        return;
    }
    let path = paths.first().cloned().unwrap_or_default();
    loop {
        std::thread::sleep(REFRESH);
        let snap = offline_snapshot(&path);
        let local = target.as_ref().ok().map(|t| status::local(cfg, t));
        let alive = post(qt, move |s| {
            s.apply_snapshot(snap);
            if let Some(local) = local {
                s.apply_local(local);
            }
        });
        if !alive {
            return;
        }
    }
}
