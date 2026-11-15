// SPDX-License-Identifier: GPL-3.0-only
//! `SettingsModel`: the `QObject` behind `Settings.qml`. The offline-folders toggle flow lives in
//! [`rclone_ui_view::settings`]; this file runs its worker steps on threads (the setup includes the
//! system password prompt) and mirrors the state into properties. Refreshes every [`REFRESH`].

use core::pin::Pin;
use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use rclone_ui_core::config::{self, Config};
use rclone_ui_core::offline_setup::{set_up, tear_down, turn_on, Checks};
use rclone_ui_core::pin::{self, OfflineState};
use rclone_ui_core::{keeper, share};
use rclone_ui_view::settings::{ConsentAction, SettingsState, SettingsView, ToggleAction};
use rclone_ui_view::REFRESH;
use std::time::{SystemTime, UNIX_EPOCH};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
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
        let state = SettingsState::new(config::store().load());
        let view = state.view(now());
        self.as_mut().rust_mut().get_mut().state = Some(state);
        self.as_mut().show(&view);

        let qt = self.qt_thread();
        std::thread::spawn(move || loop {
            // Probing can be slow (systemctl, rc); keep it off the Qt thread.
            let cfg = config::store().load();
            let checks = Checks::probe(&cfg);
            let state = OfflineState::load();
            let alive = post(&qt, move |s| {
                s.checks = checks;
                s.state = state;
                // While a setup runs, or its consent is open, our copy is the truth.
                if !s.busy && !s.consent {
                    s.cfg = cfg;
                }
            });
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
}
