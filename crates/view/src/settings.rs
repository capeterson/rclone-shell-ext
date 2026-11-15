// SPDX-License-Identifier: GPL-3.0-only
//! What the settings window shows and the offline-folders toggle flow, as plain Rust.
//!
//! Turning offline folders on is a small state machine: the switch asks for consent first
//! (nothing is changed yet), "Continue" runs the setup on a worker thread (which includes the
//! system password prompt), and a failure undoes everything so the switch goes back off.

use crate::ago;
use rclone_ui_core::config::Config;
use rclone_ui_core::offline_setup::{consent_items, setup_steps, Checks};
use rclone_ui_core::pin::{self, OfflineState, Phase, PinError};
use rclone_ui_i18n::fl;
use serde_json::json;

/// What the switch should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAction {
    /// The switch is locked right now.
    Ignore,
    /// Explain first; nothing is changed until the user continues.
    ShowConsent,
    /// Run the teardown on a worker.
    TearDown,
}

/// What the consent dialog's answer should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentAction {
    /// Nothing was saved or changed, so the switch simply stays off.
    Cancel,
    /// Offline folders are on but setup is incomplete: run the remaining steps.
    SetUp,
    /// Turn offline folders on (saves the setting, then sets up; undone on failure).
    TurnOn,
}

#[derive(Debug, Clone)]
pub struct SettingsState {
    pub cfg: Config,
    pub checks: Checks,
    pub state: OfflineState,
    /// A setup or teardown is running; buttons are disabled.
    pub busy: bool,
    pub error: Option<String>,
    /// Something worth telling after a successful setup (where the backup went).
    pub note: Option<String>,
    /// The consent dialog is showing; nothing has been changed yet.
    pub consent: bool,
}

/// One row of the pins list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRow {
    pub rel: String,
    /// The keeper's phase, e.g. "Downloading for offline use".
    pub phase: String,
    pub icon: String,
    /// A line about the folder: progress, last sync, and the last error if any.
    pub label: String,
    pub error: String,
    pub can_repair: bool,
    /// Still being cleaned up after removal: no buttons.
    pub removing: bool,
}

impl SettingsState {
    pub fn new(cfg: Config) -> Self {
        SettingsState {
            cfg,
            checks: Checks::default(),
            state: OfflineState::default(),
            busy: false,
            error: None,
            note: None,
            consent: false,
        }
    }

    /// Folders must all be removed before offline folders can be turned off.
    pub fn can_turn_off(&self) -> bool {
        self.cfg.pins.is_empty() && !self.busy
    }

    pub fn toggle_allowed(&self) -> bool {
        !self.busy && !self.consent && (!self.cfg.offline_enabled || self.can_turn_off())
    }

    /// The switch reads "on" while the consent dialog is open too (it is the pending answer).
    pub fn toggle_checked(&self) -> bool {
        self.cfg.offline_enabled || self.consent
    }

    pub fn description(&self) -> String {
        if self.cfg.offline_enabled && !self.cfg.pins.is_empty() {
            fl!("settings-offline-description-locked")
        } else {
            fl!("settings-offline-description")
        }
    }

    /// The user flipped the switch.
    pub fn toggle_offline(&mut self, on: bool) -> ToggleAction {
        if !self.toggle_allowed() {
            return ToggleAction::Ignore;
        }
        self.error = None;
        if on {
            self.consent = true;
            ToggleAction::ShowConsent
        } else {
            self.busy = true;
            self.cfg.offline_enabled = false;
            ToggleAction::TearDown
        }
    }

    /// The "Set up" button, which picks up after something undid part of the setup.
    pub fn request_setup(&mut self) {
        self.error = None;
        self.consent = true;
    }

    /// The consent dialog was answered.
    pub fn answer_consent(&mut self, go: bool) -> ConsentAction {
        self.consent = false;
        if !go {
            return ConsentAction::Cancel;
        }
        self.busy = true;
        if self.cfg.offline_enabled {
            ConsentAction::SetUp
        } else {
            self.cfg.offline_enabled = true;
            ConsentAction::TurnOn
        }
    }

    /// A setup or teardown finished. `cfg` is the saved setting afterwards: a failed turn-on
    /// switches itself back off, and that must show now.
    pub fn finish(&mut self, result: Result<Option<String>, String>, cfg: Config) {
        self.busy = false;
        (self.note, self.error) = match result {
            Ok(note) => (note, None),
            Err(e) => (None, Some(e)),
        };
        self.cfg = cfg;
    }

    /// What the consent dialog lists: only the steps not already done.
    pub fn consent_items(&self) -> Vec<(String, String)> {
        consent_items(self.checks, &self.cfg)
    }

    pub fn consent_go_label(&self) -> String {
        if self.checks.fuse_conf {
            fl!("common-continue")
        } else {
            fl!("settings-consent-continue-password")
        }
    }

    /// Setup is on but incomplete: the checklist shows instead of the folders.
    pub fn show_setup(&self) -> bool {
        self.cfg.offline_enabled && !self.checks.complete()
    }

    /// Hint under "Setup isn't complete".
    pub fn setup_hint(&self) -> String {
        if self.busy {
            fl!("settings-setup-hint-busy")
        } else if self.checks.mounted {
            fl!("settings-setup-hint-remaining")
        } else {
            fl!("settings-setup-hint-mount-down")
        }
    }

    pub fn pin_rows(&self, now: i64) -> Vec<PinRow> {
        let mut rows: Vec<PinRow> = self
            .cfg
            .pins
            .iter()
            .map(|rel| {
                let p = self.state.pin(rel);
                let phase = p.map_or(Phase::Downloading, |p| p.phase);
                let mut label = pin::describe(p, None);
                if let Some(t) = p
                    .and_then(|p| p.last_sync)
                    .filter(|_| !p.is_some_and(|p| p.syncing))
                {
                    label = fl!("settings-pin-synced", label = label, when = ago(now, t));
                }
                PinRow {
                    rel: rel.clone(),
                    phase: phase.label(),
                    icon: phase.icon().to_string(),
                    label,
                    error: p
                        .and_then(|p| p.error.as_ref().map(PinError::label))
                        .unwrap_or_default(),
                    can_repair: phase == Phase::Attention,
                    removing: false,
                }
            })
            .collect();
        // Folders that were removed but are still being cleaned up.
        rows.extend(
            self.state
                .pins
                .iter()
                .filter(|p| !self.cfg.pins.contains(&p.rel))
                .map(|p| PinRow {
                    rel: p.rel.clone(),
                    phase: Phase::Removing.label(),
                    icon: Phase::Removing.icon().to_string(),
                    label: Phase::Removing.label(),
                    error: p.error.as_ref().map(PinError::label).unwrap_or_default(),
                    can_repair: false,
                    removing: true,
                }),
        );
        rows
    }

    /// The last few notices, newest first, with when they happened.
    pub fn notices(&self, now: i64) -> Vec<(String, String)> {
        if !self.cfg.offline_enabled {
            return Vec::new();
        }
        self.state
            .notices
            .iter()
            .rev()
            .take(5)
            .map(|n| (n.text.label(), ago(now, n.at)))
            .collect()
    }

    pub fn view(&self, now: i64) -> SettingsView {
        let items: Vec<_> = self
            .consent_items()
            .into_iter()
            .map(|(t, d)| json!({ "title": t, "detail": d }))
            .collect();
        let steps: Vec<_> = setup_steps(self.checks)
            .into_iter()
            .map(|(t, d, done)| json!({ "title": t, "detail": d, "done": done }))
            .collect();
        let pins: Vec<_> = self
            .pin_rows(now)
            .into_iter()
            .map(|r| {
                json!({
                    "rel": r.rel, "phase": r.phase, "icon": r.icon, "label": r.label,
                    "error": r.error, "canRepair": r.can_repair, "removing": r.removing,
                })
            })
            .collect();
        let notices: Vec<_> = self
            .notices(now)
            .into_iter()
            .map(|(t, w)| json!({ "text": t, "when": w }))
            .collect();
        SettingsView {
            offline_enabled: self.cfg.offline_enabled,
            toggle_checked: self.toggle_checked(),
            toggle_enabled: self.toggle_allowed(),
            description: self.description(),
            busy: self.busy,
            consent_visible: self.consent,
            consent_json: serde_json::Value::Array(items).to_string(),
            consent_go_label: self.consent_go_label(),
            show_setup: self.show_setup(),
            setup_steps_json: serde_json::Value::Array(steps).to_string(),
            setup_hint: self.setup_hint(),
            show_pins: self.cfg.offline_enabled && !self.show_setup(),
            no_pins: self.cfg.pins.is_empty(),
            pins_json: serde_json::Value::Array(pins).to_string(),
            notices_json: serde_json::Value::Array(notices).to_string(),
            context_menu: self.cfg.context_menu,
            error: self.error.clone().unwrap_or_default(),
            note: self.note.clone().unwrap_or_default(),
        }
    }
}

/// Everything the QML side binds to, as plain values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // one per bound property
pub struct SettingsView {
    pub offline_enabled: bool,
    pub toggle_checked: bool,
    pub toggle_enabled: bool,
    pub description: String,
    pub busy: bool,
    pub consent_visible: bool,
    pub consent_json: String,
    pub consent_go_label: String,
    pub show_setup: bool,
    pub setup_steps_json: String,
    pub setup_hint: String,
    pub show_pins: bool,
    pub no_pins: bool,
    pub pins_json: String,
    pub notices_json: String,
    pub context_menu: bool,
    pub error: String,
    pub note: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rclone_ui_core::pin::{Notice, NoticeText, PinState};

    fn state() -> SettingsState {
        SettingsState::new(Config::default())
    }

    #[test]
    fn turning_on_asks_for_consent_before_changing_anything() {
        let mut s = state();
        assert_eq!(s.toggle_offline(true), ToggleAction::ShowConsent);
        assert!(s.consent);
        assert!(!s.cfg.offline_enabled, "nothing has changed yet");
        assert!(
            s.toggle_checked(),
            "but the switch shows the pending answer"
        );
        assert!(
            !s.toggle_allowed(),
            "and is locked while the dialog is open"
        );
        assert_eq!(s.toggle_offline(false), ToggleAction::Ignore);
    }

    #[test]
    fn cancelling_leaves_the_switch_off() {
        let mut s = state();
        s.toggle_offline(true);
        assert_eq!(s.answer_consent(false), ConsentAction::Cancel);
        assert!(!s.consent && !s.busy && !s.cfg.offline_enabled);
        assert!(!s.toggle_checked());
    }

    #[test]
    fn continuing_turns_on_and_locks_the_ui() {
        let mut s = state();
        s.toggle_offline(true);
        assert_eq!(s.answer_consent(true), ConsentAction::TurnOn);
        assert!(s.busy && s.cfg.offline_enabled && !s.consent);
        assert!(!s.toggle_allowed());
    }

    #[test]
    fn a_failed_turn_on_goes_back_off_with_the_error() {
        let mut s = state();
        s.toggle_offline(true);
        s.answer_consent(true);
        // The core undid its changes, so the saved setting is off again.
        s.finish(Err("authentication cancelled".into()), Config::default());
        assert!(!s.busy && !s.cfg.offline_enabled && !s.toggle_checked());
        assert_eq!(s.error.as_deref(), Some("authentication cancelled"));
        assert_eq!(s.note, None);
        assert_eq!(s.view(0).error, "authentication cancelled");
    }

    #[test]
    fn a_successful_turn_on_keeps_the_note() {
        let mut s = state();
        s.toggle_offline(true);
        s.answer_consent(true);
        let on = Config {
            offline_enabled: true,
            ..Config::default()
        };
        s.finish(Ok(Some("backup at x".into())), on);
        assert!(s.cfg.offline_enabled && s.toggle_checked());
        assert_eq!(s.note.as_deref(), Some("backup at x"));
        assert_eq!(s.error, None);
    }

    #[test]
    fn set_up_picks_up_when_already_on() {
        let mut s = state();
        s.cfg.offline_enabled = true;
        s.request_setup();
        assert!(s.consent);
        assert_eq!(s.answer_consent(true), ConsentAction::SetUp);
        assert!(s.busy);
    }

    #[test]
    fn turning_off_needs_no_pins_and_tears_down() {
        let mut s = state();
        s.cfg.offline_enabled = true;
        s.cfg.pins = vec!["d".into()];
        assert!(!s.can_turn_off() && !s.toggle_allowed());
        assert!(s.description().starts_with("Stop keeping"));
        assert_eq!(s.toggle_offline(false), ToggleAction::Ignore);
        s.cfg.pins.clear();
        assert_eq!(s.toggle_offline(false), ToggleAction::TearDown);
        assert!(s.busy && !s.cfg.offline_enabled);
        s.finish(Ok(None), Config::default());
        assert!(!s.busy && !s.cfg.offline_enabled);
    }

    #[test]
    fn the_consent_button_mentions_the_password_only_when_needed() {
        let mut s = state();
        assert_eq!(s.consent_go_label(), "Continue to password prompt");
        s.checks.fuse_conf = true;
        assert_eq!(s.consent_go_label(), "Continue");
    }

    #[test]
    fn consent_lists_only_what_is_left_to_do() {
        let mut s = state();
        let all = s.consent_items().len();
        s.checks = Checks {
            fuse_conf: true,
            units: true,
            keeper_running: true,
            mounted: true,
            mount_allows_other: true,
        };
        assert!(s.consent_items().len() < all);
    }

    #[test]
    fn setup_checklist_shows_only_while_incomplete() {
        let mut s = state();
        assert!(!s.show_setup(), "off: nothing to set up");
        s.cfg.offline_enabled = true;
        assert!(s.show_setup());
        assert!(!s.view(0).show_pins);
        s.checks = Checks {
            fuse_conf: true,
            units: true,
            keeper_running: true,
            mounted: true,
            mount_allows_other: true,
        };
        assert!(!s.show_setup());
        assert!(s.view(0).show_pins);
        assert_eq!(s.setup_hint(), "Runs the remaining steps above.");
        s.busy = true;
        assert!(s.setup_hint().contains("password"));
    }

    fn pinned(rel: &str, phase: Phase) -> PinState {
        PinState {
            rel: rel.into(),
            phase,
            ..PinState::default()
        }
    }

    #[test]
    fn pin_rows_carry_phase_label_and_error() {
        let mut s = state();
        s.cfg.offline_enabled = true;
        s.cfg.pins = vec!["a".into(), "b".into(), "c".into()];
        s.state.pins.push(PinState {
            error: Some(PinError::Other("boom".into())),
            ..pinned("a", Phase::Attention)
        });
        s.state.pins.push(PinState {
            last_sync: Some(100),
            ..pinned("b", Phase::Ready)
        });
        let rows = s.pin_rows(100 + 120);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].can_repair);
        assert_eq!(rows[0].error, "boom");
        assert_eq!(rows[0].phase, Phase::Attention.label());
        assert!(!rows[1].can_repair);
        assert!(
            rows[1].label.ends_with("synced 2 minutes ago"),
            "{}",
            rows[1].label
        );
        // No keeper report yet: waiting to start, shown as downloading.
        assert_eq!(rows[2].phase, Phase::Downloading.label());
        assert_eq!(rows[2].label, "Waiting to start…");
    }

    #[test]
    fn removed_folders_show_until_cleaned_up() {
        let mut s = state();
        s.cfg.pins = vec!["a".into()];
        s.state.pins.push(pinned("a", Phase::Ready));
        s.state.pins.push(PinState {
            error: Some(PinError::Other("busy".into())),
            ..pinned("gone", Phase::Removing)
        });
        let rows = s.pin_rows(0);
        assert_eq!(rows.len(), 2);
        assert!(rows[1].removing && !rows[1].can_repair);
        assert_eq!(rows[1].rel, "gone");
        assert_eq!(rows[1].error, "busy");
        let v: serde_json::Value = serde_json::from_str(&s.view(0).pins_json).unwrap();
        assert_eq!(v[1]["removing"], true);
        assert_eq!(v[0]["canRepair"], false);
    }

    #[test]
    fn notices_are_the_latest_five_newest_first() {
        let mut s = state();
        s.cfg.offline_enabled = true;
        for i in 0..7 {
            s.state.notices.push(Notice {
                at: i,
                rel: "d".into(),
                text: NoticeText::Other(format!("n{i}")),
            });
        }
        let n = s.notices(7);
        assert_eq!(n.len(), 5);
        assert_eq!(n[0].0, "n6");
        s.cfg.offline_enabled = false;
        assert_eq!(s.notices(7).len(), 0);
    }

    #[test]
    fn the_view_serialises_for_qml() {
        let mut s = state();
        s.cfg.offline_enabled = true;
        let v = s.view(0);
        assert!(v.toggle_checked && v.offline_enabled && v.no_pins);
        let steps: serde_json::Value = serde_json::from_str(&v.setup_steps_json).unwrap();
        assert_eq!(steps.as_array().unwrap().len(), 3);
        assert!(v.context_menu);
    }
}
