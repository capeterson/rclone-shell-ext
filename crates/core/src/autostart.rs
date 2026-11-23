// SPDX-License-Identifier: GPL-3.0-only
//! Is the mount's systemd user unit set to start at login? If not, the Drive folder is stopped
//! after every restart until someone starts it by hand, so the front ends call [`spawn_check`]
//! once they are up and the user gets one notification per boot with a button that fixes it.
//!
//! systemd only: the answer comes from `systemctl --user is-enabled`. Without systemd (or with
//! a unit systemd can't classify) the check says nothing.

use crate::config::Config;
use rclone_ui_i18n::fl;
use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// What `systemctl --user is-enabled` says about starting the mount at login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginStart {
    /// Enabled: it starts at login.
    Enabled,
    /// Not enabled (or enabled with `--runtime` only, which a reboot forgets): it won't start
    /// at login, and `systemctl --user enable` would fix that.
    NotEnabled,
    /// Anything else (masked on purpose, static, generated, missing, no systemd): not ours to
    /// nag about.
    Unknown,
}

/// Classify the state word `systemctl is-enabled` prints.
#[must_use]
pub fn parse_is_enabled(output: &str) -> LoginStart {
    match output.trim() {
        "enabled" => LoginStart::Enabled,
        "disabled" | "enabled-runtime" => LoginStart::NotEnabled,
        _ => LoginStart::Unknown,
    }
}

/// What the user picked on the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Enable the unit and start it now.
    Enable,
    /// Never show this notification again.
    Dismiss,
    /// Closed it, or the notification server has no buttons.
    Nothing,
}

/// The outside world [`check_and_notify`] works against (systemd, the notification server, the
/// settings and a per-boot marker), so the decisions can be tested without any of them.
pub trait Env {
    /// The stdout of `systemctl --user is-enabled <service>` (it exits non-zero for most
    /// states, which is not an error here), or `None` if systemctl couldn't run.
    fn is_enabled(&self, service: &str) -> Option<String>;
    /// This boot's id; `None` if unknown (then the reminder isn't de-duplicated).
    fn boot_id(&self) -> Option<String>;
    /// The boot id the reminder was last shown in.
    fn notified_boot(&self) -> Option<String>;
    /// Remember that the reminder was shown in `boot`.
    fn set_notified_boot(&self, boot: &str);
    /// The current settings.
    fn config(&self) -> Config;
    /// Switch the reminder off for good.
    ///
    /// # Errors
    /// A message if the settings couldn't be saved.
    fn turn_reminder_off(&self) -> Result<(), String>;
    /// Show the reminder and wait for the user's choice.
    fn ask(&self, service: &str) -> Choice;
    /// Run `systemctl --user enable --now <service>`.
    ///
    /// # Errors
    /// systemctl's message if it failed.
    fn enable(&self, service: &str) -> Result<(), String>;
    /// Tell the user something went wrong.
    fn report(&self, text: &str);
}

/// Show the reminder if the mount's unit won't start at login, at most once per boot and never
/// after the user dismissed it; then do what the user picked. Blocks until the notification is
/// closed, so run it on its own thread ([`spawn_check`]).
pub fn check_and_notify(env: &dyn Env) {
    let cfg = env.config();
    if !cfg.login_start_reminder {
        return;
    }
    let state = env
        .is_enabled(&cfg.service)
        .map_or(LoginStart::Unknown, |o| parse_is_enabled(&o));
    if state != LoginStart::NotEnabled {
        return;
    }
    let boot = env.boot_id();
    if boot.is_some() && boot == env.notified_boot() {
        return;
    }
    // Remember before asking: the notification stays up until it's closed, and a restart of
    // the front end meanwhile shouldn't show a second one.
    if let Some(b) = &boot {
        env.set_notified_boot(b);
    }
    let failed = match env.ask(&cfg.service) {
        Choice::Enable => env.enable(&cfg.service).err(),
        Choice::Dismiss => env.turn_reminder_off().err(),
        Choice::Nothing => None,
    };
    if let Some(error) = failed {
        env.report(&fl!(
            "health-login-start-failed",
            service = cfg.service.as_str(),
            error = error
        ));
    }
}

/// Run [`check_and_notify`] against the real system on a background thread.
pub fn spawn_check() {
    std::thread::spawn(|| check_and_notify(&System));
}

/// The real [`Env`]: `systemctl --user`, `notify-send`, the shared config store and a marker
/// file in the state directory.
struct System;

fn marker() -> PathBuf {
    crate::pin::state_dir().join("login-start-reminded")
}

fn notify_args(title: &str, body: &str) -> Vec<String> {
    vec![
        format!("--app-name={}", fl!("common-app-name")),
        format!("--icon={}", crate::identity::identity().app_id),
        title.to_string(),
        body.to_string(),
    ]
}

impl Env for System {
    fn is_enabled(&self, service: &str) -> Option<String> {
        let out = Command::new("systemctl")
            .args(["--user", "is-enabled", service])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn boot_id(&self) -> Option<String> {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn notified_boot(&self) -> Option<String> {
        std::fs::read_to_string(marker())
            .ok()
            .map(|s| s.trim().to_string())
    }

    fn set_notified_boot(&self, boot: &str) {
        let _ = crate::config::write_atomic(&marker(), boot);
    }

    fn config(&self) -> Config {
        crate::config::store().load()
    }

    fn turn_reminder_off(&self) -> Result<(), String> {
        crate::config::store()
            .save(&mut |c| c.login_start_reminder = false)
            .map(|_| ())
    }

    fn ask(&self, service: &str) -> Choice {
        let mut args = notify_args(
            &fl!("health-login-start-off-title"),
            &fl!("health-login-start-off-body", service = service),
        );
        // `-A` waits for a click and prints the chosen action's name (libnotify 0.7.10+).
        args.insert(
            0,
            format!("--action=enable={}", fl!("health-login-start-enable")),
        );
        args.insert(
            1,
            format!("--action=dismiss={}", fl!("health-login-start-dismiss")),
        );
        let Ok(mut child) = Command::new("notify-send")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return Choice::Nothing;
        };
        let mut picked = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = out.read_to_string(&mut picked);
        }
        let ok = child.wait().is_ok_and(|s| s.success());
        if !ok {
            // An older notify-send without `--action`: still tell the user, without buttons.
            let _ = Command::new("notify-send").args(&args[2..]).status();
            return Choice::Nothing;
        }
        match picked.trim() {
            "enable" => Choice::Enable,
            "dismiss" => Choice::Dismiss,
            _ => Choice::Nothing,
        }
    }

    fn enable(&self, service: &str) -> Result<(), String> {
        crate::units::systemctl(&["enable", "--now", service])
    }

    fn report(&self, text: &str) {
        let _ = Command::new("notify-send")
            .args(notify_args(&fl!("health-login-start-off-title"), text))
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct Fake {
        is_enabled: Option<&'static str>,
        boot: Option<&'static str>,
        notified: RefCell<Option<String>>,
        cfg: RefCell<Config>,
        choice: Choice,
        enable_result: Result<(), String>,
        asked: Cell<u32>,
        enabled: RefCell<Vec<String>>,
        reports: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(is_enabled: &'static str) -> Self {
            Self {
                is_enabled: Some(is_enabled),
                boot: Some("boot-1"),
                notified: RefCell::new(None),
                cfg: RefCell::new(Config::default()),
                choice: Choice::Nothing,
                enable_result: Ok(()),
                asked: Cell::new(0),
                enabled: RefCell::new(Vec::new()),
                reports: RefCell::new(Vec::new()),
            }
        }
    }

    impl Env for Fake {
        fn is_enabled(&self, _: &str) -> Option<String> {
            self.is_enabled.map(|s| format!("{s}\n"))
        }
        fn boot_id(&self) -> Option<String> {
            self.boot.map(str::to_string)
        }
        fn notified_boot(&self) -> Option<String> {
            self.notified.borrow().clone()
        }
        fn set_notified_boot(&self, boot: &str) {
            *self.notified.borrow_mut() = Some(boot.to_string());
        }
        fn config(&self) -> Config {
            self.cfg.borrow().clone()
        }
        fn turn_reminder_off(&self) -> Result<(), String> {
            self.cfg.borrow_mut().login_start_reminder = false;
            Ok(())
        }
        fn ask(&self, _: &str) -> Choice {
            self.asked.set(self.asked.get() + 1);
            self.choice
        }
        fn enable(&self, service: &str) -> Result<(), String> {
            self.enabled.borrow_mut().push(service.to_string());
            self.enable_result.clone()
        }
        fn report(&self, text: &str) {
            self.reports.borrow_mut().push(text.to_string());
        }
    }

    #[test]
    fn only_units_that_wont_start_at_login_are_reported() {
        assert_eq!(parse_is_enabled("enabled\n"), LoginStart::Enabled);
        assert_eq!(parse_is_enabled("disabled\n"), LoginStart::NotEnabled);
        // `--runtime` enablement is gone after a reboot.
        assert_eq!(parse_is_enabled("enabled-runtime"), LoginStart::NotEnabled);
        // Masked is a deliberate choice; static/not-found are not fixable by enabling.
        for s in ["masked", "static", "not-found", "generated", ""] {
            assert_eq!(parse_is_enabled(s), LoginStart::Unknown, "{s}");
        }
    }

    #[test]
    fn a_disabled_mount_is_reported_once_per_boot() {
        let env = Fake::new("disabled");
        check_and_notify(&env);
        check_and_notify(&env);
        assert_eq!(env.asked.get(), 1);
        *env.notified.borrow_mut() = Some("boot-0".into());
        check_and_notify(&env);
        assert_eq!(env.asked.get(), 2, "a new boot reminds again");
    }

    #[test]
    fn nothing_is_shown_when_enabled_unknown_or_dismissed() {
        for state in ["enabled", "masked"] {
            let env = Fake::new(state);
            check_and_notify(&env);
            assert_eq!(env.asked.get(), 0, "{state}");
        }
        let env = Fake {
            is_enabled: None,
            ..Fake::new("disabled")
        };
        check_and_notify(&env);
        assert_eq!(env.asked.get(), 0, "no systemctl");
        let env = Fake::new("disabled");
        env.cfg.borrow_mut().login_start_reminder = false;
        check_and_notify(&env);
        assert_eq!(env.asked.get(), 0, "dismissed");
    }

    #[test]
    fn enable_enables_the_configured_service() {
        let env = Fake {
            choice: Choice::Enable,
            ..Fake::new("disabled")
        };
        env.cfg.borrow_mut().service = "my-mount.service".into();
        check_and_notify(&env);
        assert_eq!(*env.enabled.borrow(), ["my-mount.service"]);
        assert!(env.reports.borrow().is_empty());
    }

    #[test]
    fn a_failed_enable_is_reported() {
        let env = Fake {
            choice: Choice::Enable,
            enable_result: Err("Access denied".into()),
            ..Fake::new("disabled")
        };
        check_and_notify(&env);
        let reports = env.reports.borrow();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].contains("Access denied"), "{}", reports[0]);
    }

    #[test]
    fn dismiss_turns_the_reminder_off_for_good() {
        let env = Fake {
            choice: Choice::Dismiss,
            ..Fake::new("disabled")
        };
        check_and_notify(&env);
        assert!(!env.cfg.borrow().login_start_reminder);
        assert!(env.enabled.borrow().is_empty());
        *env.notified.borrow_mut() = None;
        check_and_notify(&env);
        assert_eq!(env.asked.get(), 1);
    }
}
