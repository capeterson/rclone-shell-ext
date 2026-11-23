// SPDX-License-Identifier: GPL-3.0-only
//! `SettingsModel`: the `QObject` behind `Settings.qml`. The offline-folders toggle flow lives in
//! [`rclone_ui_view::settings`]; this file runs its worker steps on threads (the setup includes the
//! system password prompt) and mirrors the state into properties. Refreshes every [`REFRESH`].
//! The Backups and Restore sections show [`rclone_ui_view::backup`] rows; adding, removing and
//! listing backups run on threads too (systemctl, rclone).

use core::pin::Pin;
use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};
use rclone_ui_core::backup as core_backup;
use rclone_ui_core::config::{self, Config};
use rclone_ui_core::offline_setup::{set_up, tear_down, turn_on, Checks};
use rclone_ui_core::pin::{self, OfflineState};
use rclone_ui_core::{keeper, share};
use rclone_ui_view::backup;
use rclone_ui_view::settings::{ConsentAction, SettingsState, SettingsView, ToggleAction};
use rclone_ui_view::REFRESH;
use std::time::{SystemTime, UNIX_EPOCH};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// The offline-folders section is relevant (the setting is on).
        #[qproperty(bool, offline_enabled)]
        #[qproperty(bool, toggle_checked)]
        #[qproperty(bool, toggle_enabled)]
        #[qproperty(QString, description)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, consent_visible)]
        #[qproperty(QString, consent_json)]
        #[qproperty(QString, consent_go_label)]
        #[qproperty(bool, show_setup)]
        #[qproperty(QString, setup_steps_json)]
        #[qproperty(QString, setup_hint)]
        #[qproperty(bool, show_pins)]
        #[qproperty(bool, no_pins)]
        #[qproperty(QString, pins_json)]
        #[qproperty(QString, notices_json)]
        #[qproperty(bool, context_menu)]
        #[qproperty(QString, error)]
        #[qproperty(QString, note)]
        // Backups: the section's description (names the remote folder), `backup::rows_json`,
        // and whether an add or remove is running.
        #[qproperty(QString, backup_description)]
        #[qproperty(QString, backups_json)]
        #[qproperty(bool, backup_busy)]
        // Restore: `backup::restore_rows_json`, the list is being fetched, and it was fetched
        // at least once (so an empty list means there are none).
        #[qproperty(QString, restore_json)]
        #[qproperty(bool, restore_loading)]
        #[qproperty(bool, restore_loaded)]
        type SettingsModel = super::SettingsModelRust;

        /// Load the settings and start the periodic refresh.
        #[qinvokable]
        fn start(self: Pin<&mut SettingsModel>);

        /// The offline switch was flipped.
        #[qinvokable]
        #[cxx_name = "toggleOffline"]
        fn toggle_offline(self: Pin<&mut SettingsModel>, on: bool);

        /// The "Set up" button (setup was left incomplete).
        #[qinvokable]
        #[cxx_name = "requestSetup"]
        fn request_setup(self: Pin<&mut SettingsModel>);

        /// The consent dialog's answer: Continue (true) or Cancel (false).
        #[qinvokable]
        #[cxx_name = "answerConsent"]
        fn answer_consent(self: Pin<&mut SettingsModel>, go: bool);

        /// Ask the keeper to repair a folder that needs attention.
        #[qinvokable]
        fn repair(self: Pin<&mut SettingsModel>, rel: &QString);

        /// Stop keeping a folder offline.
        #[qinvokable]
        fn remove(self: Pin<&mut SettingsModel>, rel: &QString);

        /// Open a folder kept offline in the file manager.
        #[qinvokable]
        #[cxx_name = "openPin"]
        fn open_pin(self: &SettingsModel, rel: &QString);

        /// Show or hide the Dolphin right-click entry (saved in the settings).
        #[qinvokable]
        #[cxx_name = "saveContextMenu"]
        fn save_context_menu(self: Pin<&mut SettingsModel>, on: bool);

        /// Start backing up a folder picked in the folder dialog.
        #[qinvokable]
        #[cxx_name = "addBackup"]
        fn add_backup(self: Pin<&mut SettingsModel>, folder: &QUrl);

        /// Stop backing up a folder (the copy in Drive is kept).
        #[qinvokable]
        #[cxx_name = "stopBackup"]
        fn stop_backup(self: Pin<&mut SettingsModel>, source: &QString);

        /// List every machine's backups in Drive (in the background).
        #[qinvokable]
        #[cxx_name = "findBackups"]
        fn find_backups(self: Pin<&mut SettingsModel>);

        /// Restore the backup at `destination` into a folder picked in the folder dialog, and
        /// with `link` keep backing that folder up.
        #[qinvokable]
        #[cxx_name = "restoreBackup"]
        fn restore_backup(
            self: Pin<&mut SettingsModel>,
            destination: &QString,
            folder: &QUrl,
            link: bool,
        );
    }

    impl cxx_qt::Threading for SettingsModel {}
}

#[allow(clippy::struct_excessive_bools)] // one per bound property
pub struct SettingsModelRust {
    offline_enabled: bool,
    toggle_checked: bool,
    toggle_enabled: bool,
    description: QString,
    busy: bool,
    consent_visible: bool,
    consent_json: QString,
    consent_go_label: QString,
    show_setup: bool,
    setup_steps_json: QString,
    setup_hint: QString,
    show_pins: bool,
    no_pins: bool,
    pins_json: QString,
    notices_json: QString,
    context_menu: bool,
    error: QString,
    note: QString,
    backup_description: QString,
    backups_json: QString,
    backup_busy: bool,
    restore_json: QString,
    restore_loading: bool,
    restore_loaded: bool,
    /// Only touched on the Qt thread.
    state: Option<SettingsState>,
}

impl Default for SettingsModelRust {
    fn default() -> Self {
        let empty = QString::default;
        SettingsModelRust {
            offline_enabled: false,
            toggle_checked: false,
            toggle_enabled: false,
            description: empty(),
            busy: false,
            consent_visible: false,
            consent_json: QString::from("[]"),
            consent_go_label: empty(),
            show_setup: false,
            setup_steps_json: QString::from("[]"),
            setup_hint: empty(),
            show_pins: false,
            no_pins: true,
            pins_json: QString::from("[]"),
            notices_json: QString::from("[]"),
            context_menu: true,
            error: empty(),
            note: empty(),
            backup_description: empty(),
            backups_json: QString::from("[]"),
            backup_busy: false,
            restore_json: QString::from("[]"),
            restore_loading: false,
            restore_loaded: false,
            state: None,
        }
    }
}

type Model = qobject::SettingsModel;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Run `f` on the state on the Qt thread, then refresh the bound properties. `false` once the
/// model is gone.
fn post(qt: &CxxQtThread<Model>, f: impl FnOnce(&mut SettingsState) + Send + 'static) -> bool {
    qt.queue(move |model| model.update(f)).is_ok()
}

/// Run a setup step on a worker; when it finishes, re-read the saved settings (a failed
/// turn-on switches itself back off) and show the outcome.
fn run_worker(
    qt: CxxQtThread<Model>,
    work: impl FnOnce() -> Result<Option<String>, String> + Send + 'static,
) {
    std::thread::spawn(move || {
        let result = work();
        let cfg = config::store().load();
        let checks = Checks::probe(&cfg);
        let state = OfflineState::load();
        post(&qt, move |s| {
            s.finish(result, cfg);
            s.checks = checks;
            s.state = state;
        });
    });
}

/// The Backups rows for `cfg`, with the keeper's last recorded status.
fn backup_rows(cfg: &Config) -> String {
    // No status yet (the keeper never ran) or unreadable: the rows say "Not backed up yet".
    let status = core_backup::status().unwrap_or_default();
    backup::rows_json(&backup::rows(cfg, &status, now()))
}

/// A new backup action replaces the outcome of the last change.
fn clear_messages(s: &mut SettingsState) {
    s.error = None;
    s.note = None;
}

/// Add or remove a backup on a worker (it installs or removes the backup unit with
/// systemctl), then show the new list and any failure.
fn run_backup_worker(
    qt: CxxQtThread<Model>,
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) {
    std::thread::spawn(move || {
        let result = work();
        let cfg = config::store().load();
        let rows = backup_rows(&cfg);
        let _ = qt.queue(move |mut model| {
            model.as_mut().set_backup_busy(false);
            model
                .as_mut()
                .set_backups_json(QString::from(rows.as_str()));
            model.update(move |s| {
                if let Err(code) = result {
                    s.error = Some(backup::error(&code));
                }
                s.apply_config(cfg);
            });
        });
    });
}

impl qobject::SettingsModel {
    fn update(mut self: Pin<&mut Self>, f: impl FnOnce(&mut SettingsState)) {
        let view = {
            let rust = self.as_mut().rust_mut().get_mut();
            let Some(state) = rust.state.as_mut() else {
                return;
            };
            f(state);
            state.view(now())
        };
        self.show(&view);
    }

    /// Run `f` on the state and return what it returned (`None` before `start`).
    fn with_state<T>(
        mut self: Pin<&mut Self>,
        f: impl FnOnce(&mut SettingsState) -> T,
    ) -> Option<T> {
        let mut out = None;
        self.as_mut().update(|s| out = Some(f(s)));
        out
    }

    fn show(mut self: Pin<&mut Self>, v: &SettingsView) {
        let q = |s: &str| QString::from(s);
        self.as_mut().set_offline_enabled(v.offline_enabled);
        self.as_mut().set_toggle_checked(v.toggle_checked);
        self.as_mut().set_toggle_enabled(v.toggle_enabled);
        self.as_mut().set_description(q(&v.description));
        self.as_mut().set_busy(v.busy);
        self.as_mut().set_consent_visible(v.consent_visible);
        self.as_mut().set_consent_json(q(&v.consent_json));
        self.as_mut().set_consent_go_label(q(&v.consent_go_label));
        self.as_mut().set_show_setup(v.show_setup);
        self.as_mut().set_setup_steps_json(q(&v.setup_steps_json));
        self.as_mut().set_setup_hint(q(&v.setup_hint));
        self.as_mut().set_show_pins(v.show_pins);
        self.as_mut().set_no_pins(v.no_pins);
        self.as_mut().set_pins_json(q(&v.pins_json));
        self.as_mut().set_notices_json(q(&v.notices_json));
        self.as_mut().set_context_menu(v.context_menu);
        self.as_mut().set_error(q(&v.error));
        self.as_mut().set_note(q(&v.note));
    }

    pub fn start(mut self: Pin<&mut Self>) {
        if self.rust().state.is_some() {
            return;
        }
        let cfg = config::store().load();
        let rows = backup_rows(&cfg);
        let state = SettingsState::new(cfg);
        let view = state.view(now());
        self.as_mut().rust_mut().get_mut().state = Some(state);
        self.as_mut().show(&view);
        self.as_mut()
            .set_backup_description(QString::from(backup::description().as_str()));
        self.as_mut().set_backups_json(QString::from(rows.as_str()));

        let qt = self.qt_thread();
        std::thread::spawn(move || loop {
            // Probing can be slow (systemctl, rc); keep it off the Qt thread.
            let cfg = config::store().load();
            let checks = Checks::probe(&cfg);
            let state = OfflineState::load();
            let rows = backup_rows(&cfg);
            let alive = qt
                .queue(move |mut model| {
                    model
                        .as_mut()
                        .set_backups_json(QString::from(rows.as_str()));
                    model.update(move |s| {
                        s.checks = checks;
                        s.state = state;
                        // Ignored while a setup runs or its consent is open: our copy is the truth.
                        s.apply_config(cfg);
                    });
                })
                .is_ok();
            if !alive {
                return;
            }
            std::thread::sleep(REFRESH);
        });
    }

    pub fn toggle_offline(mut self: Pin<&mut Self>, on: bool) {
        let qt = self.qt_thread();
        let action = self.as_mut().with_state(|s| {
            let a = s.toggle_offline(on);
            (a, s.cfg.clone())
        });
        if let Some((ToggleAction::TearDown, cfg)) = action {
            run_worker(qt, move || tear_down(&cfg));
        }
    }

    pub fn request_setup(self: Pin<&mut Self>) {
        self.with_state(SettingsState::request_setup);
    }

    pub fn answer_consent(mut self: Pin<&mut Self>, go: bool) {
        let qt = self.qt_thread();
        let action = self.as_mut().with_state(|s| {
            let a = s.answer_consent(go);
            (a, s.cfg.clone())
        });
        match action {
            Some((ConsentAction::SetUp, cfg)) => run_worker(qt, move || set_up(&cfg)),
            Some((ConsentAction::TurnOn, cfg)) => run_worker(qt, move || turn_on(&cfg)),
            _ => {}
        }
    }

    pub fn repair(self: Pin<&mut Self>, rel: &QString) {
        let rel = rel.to_string();
        self.with_state(|s| {
            if let Err(e) = keeper::request_repair(&rel) {
                s.error = Some(e.to_string());
            }
        });
    }

    pub fn remove(self: Pin<&mut Self>, rel: &QString) {
        let rel = rel.to_string();
        self.with_state(|s| {
            match config::store().save(&mut |c: &mut Config| c.pins.retain(|p| *p != rel)) {
                Ok(cfg) => s.cfg = cfg,
                Err(e) => s.error = Some(e),
            }
        });
    }

    #[allow(clippy::unused_self)] // an invokable, so it needs a receiver
    pub fn open_pin(&self, rel: &QString) {
        let cfg = config::store().load();
        share::open_url(&pin::mount_point(&cfg, &rel.to_string()).to_string_lossy());
    }

    pub fn save_context_menu(self: Pin<&mut Self>, on: bool) {
        self.with_state(|s| {
            match config::store().save(&mut |c: &mut Config| c.context_menu = on) {
                Ok(cfg) => s.cfg = cfg,
                Err(e) => s.error = Some(e),
            }
        });
    }

    /// Start an add or remove unless one is running.
    fn start_backup_work(
        mut self: Pin<&mut Self>,
        work: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
        if *self.backup_busy() {
            return;
        }
        self.as_mut().set_backup_busy(true);
        self.as_mut().with_state(clear_messages);
        run_backup_worker(self.qt_thread(), work);
    }

    pub fn add_backup(self: Pin<&mut Self>, folder: &QUrl) {
        // The folder dialog can hand back a remote (KIO) location; only local folders back up.
        let Some(source) = folder.to_local_file().map(|p| p.to_string()) else {
            self.with_state(|s| {
                clear_messages(s);
                s.error = Some(backup::error("invalid-source"));
            });
            return;
        };
        self.start_backup_work(move || core_backup::add(&source).map(drop));
    }

    pub fn stop_backup(self: Pin<&mut Self>, source: &QString) {
        let source = source.to_string();
        self.start_backup_work(move || core_backup::remove(&source).map(drop));
    }

    pub fn find_backups(mut self: Pin<&mut Self>) {
        if *self.restore_loading() {
            return;
        }
        self.as_mut().set_restore_loading(true);
        self.as_mut().with_state(clear_messages);
        let qt = self.qt_thread();
        // `rclone lsf` over every machine's folder: seconds, so never on the Qt thread.
        std::thread::spawn(move || {
            let rows = core_backup::list_remote()
                .map(|list| backup::restore_rows_json(&backup::restore_rows(&list)));
            let _ = qt.queue(move |mut model| {
                model.as_mut().set_restore_loading(false);
                match rows {
                    Ok(json) => {
                        model
                            .as_mut()
                            .set_restore_json(QString::from(json.as_str()));
                        model.as_mut().set_restore_loaded(true);
                    }
                    Err(code) => {
                        model.as_mut().set_restore_json(QString::from("[]"));
                        model.as_mut().set_restore_loaded(false);
                        model.update(|s| s.error = Some(backup::error(&code)));
                    }
                }
            });
        });
    }

    pub fn restore_backup(self: Pin<&mut Self>, destination: &QString, folder: &QUrl, link: bool) {
        let destination = destination.to_string();
        let target = folder.to_local_file().map(|p| p.to_string());
        self.with_state(|s| {
            clear_messages(s);
            // Only starts the separate restore process, which notifies when it is done.
            let started = target
                .ok_or_else(|| "invalid-source".to_string())
                .and_then(|t| core_backup::spawn_restore(&destination, &t, link).map(|()| t));
            match started {
                Ok(t) => s.note = Some(backup::restore_started(&t)),
                Err(code) => s.error = Some(backup::error(&code)),
            }
        });
    }
}
