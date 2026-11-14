// SPDX-License-Identifier: GPL-3.0-only
//! One-time setup for offline folders (a line in `/etc/fuse.conf`, the keeper's systemd
//! units, one restart of the Drive mount) and its teardown, without any UI.

use crate::config::Config;
use crate::{fuse_conf, pin, rc, units};

/// What's in place for offline folders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // independent yes/no checks, not a state machine
pub struct Checks {
    /// `user_allow_other` is in `/etc/fuse.conf`.
    pub fuse_conf: bool,
    /// The keeper/pin units and the mount drop-in are written and current.
    pub units: bool,
    pub keeper_running: bool,
    pub mounted: bool,
    /// The Drive mount is running with `allow_other` (it has been restarted since setup).
    pub mount_allows_other: bool,
}

impl Checks {
    #[must_use]
    pub fn probe(cfg: &Config) -> Self {
        Checks {
            fuse_conf: fuse_conf::allows_other(),
            units: units::installed(cfg),
            keeper_running: units::is_active(&units::keeper_unit_name()),
            mounted: rc::is_mounted(&cfg.mount_path),
            mount_allows_other: rc::mount_allows_other(&cfg.mount_path),
        }
    }

    #[must_use]
    pub fn complete(self) -> bool {
        self.fuse_conf && self.units && self.keeper_running && self.mount_allows_other
    }
}

/// The setup steps in order: title, explanation, done.
#[must_use]
pub fn setup_steps(c: Checks) -> [(&'static str, &'static str, bool); 3] {
    [
        (
            "Allow mounts inside the Drive folder",
            "Adds user_allow_other to /etc/fuse.conf. Asks for an administrator password once.",
            c.fuse_conf,
        ),
        (
            "Background sync service",
            "Installs and starts a user service that keeps offline folders in sync.",
            c.units && c.keeper_running,
        ),
        (
            "Restart the Drive mount",
            "Applies --allow-other to the Drive mount. Only you can open its files. Files open in \
             the Drive folder are interrupted once.",
            c.mount_allows_other,
        ),
    ]
}

/// What setup is about to change, for the consent dialog: one (title, detail) per change still
/// to be made, plus what stays true afterwards. Built from `c` so already-done steps aren't
/// listed.
#[must_use]
pub fn consent_items(c: Checks, cfg: &Config) -> Vec<(String, String)> {
    let mut items = vec![];
    if !c.fuse_conf {
        items.push((
            format!("Change a system setting: {}", fuse_conf::PATH),
            format!(
                "Adds the line “user_allow_other”, which lets your own programs mount with \
                 --allow-other. This needs an administrator password, which you'll be asked for \
                 next. The current file is first copied to {p}{infix}<date-time>.bak (if that \
                 copy fails, the file isn't touched). The line stays in place if you turn \
                 offline folders off later.",
                p = fuse_conf::PATH,
                infix = fuse_conf::backup_infix(),
            ),
        ));
    }
    if !c.units || !c.keeper_running {
        items.push((
            "Install background services".to_string(),
            format!(
                "Adds {keeper} and {pin} to {dir}, enables the first, and ties both to {svc} \
                 so they start and stop with the Drive mount.",
                keeper = units::keeper_unit_name(),
                pin = units::pin_unit_name("<folder>"),
                dir = units::unit_dir().display(),
                svc = cfg.service,
            ),
        ));
        items.push((
            "Change the Drive mount's settings".to_string(),
            format!(
                "Adds {} so the mount runs with --allow-other, --default-permissions and umask 077. \
                 Files in the Drive folder then show as owner-only (0600 / 0700), which keeps \
                 other users on this computer out.",
                units::dropin_path(&cfg.service).display()
            ),
        ));
    }
    if !c.mount_allows_other {
        items.push((
            format!("Restart {}", cfg.service),
            format!(
                "Happens once, now. Anything open in {} is interrupted; changes waiting to upload \
                 are kept and resume after the restart.",
                cfg.mount_path
            ),
        ));
    }
    items.push((
        "Afterwards".to_string(),
        format!(
            "Folders you choose to keep offline are downloaded to {} and use disk space there. \
             Turning offline folders off removes the services and mount settings again.",
            pin::data_dir().join("offline").display()
        ),
    ));
    items
}

/// Run the setup steps that aren't done yet, in order. Returns a note for the user (where the
/// `/etc/fuse.conf` backup went), if there is one.
pub fn set_up(cfg: &Config) -> Result<Option<String>, String> {
    let c = Checks::probe(cfg);
    let mut note = None;
    if !c.fuse_conf {
        note = fuse_conf::enable()?
            .map(|b| format!("The previous {} was saved as {b}", fuse_conf::PATH));
    }
    if !c.units || !c.keeper_running {
        units::install(cfg)?;
    }
    if !c.mount_allows_other {
        units::systemctl(&["restart", &cfg.service])?;
    }
    Ok(note)
}

/// Switch offline folders on and run the setup. If it can't finish (say the password prompt was
/// dismissed), undo it so the switch goes back off with the reason shown.
pub fn turn_on(cfg: &Config) -> Result<Option<String>, String> {
    crate::config::store().save(&mut |c| c.offline_enabled = true)?;
    set_up(cfg).map_err(|e| {
        let _ = tear_down(cfg);
        format!("Offline folders weren't turned on: {e}")
    })
}

/// Turn offline folders off: everything the setup added except the `/etc/fuse.conf` line.
pub fn tear_down(cfg: &Config) -> Result<Option<String>, String> {
    units::remove(cfg)?;
    crate::config::store()
        .save(&mut |c| c.offline_enabled = false)
        .map(|_| None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_is_complete_only_when_every_piece_is_in_place() {
        let all = Checks {
            fuse_conf: true,
            units: true,
            keeper_running: true,
            mounted: true,
            mount_allows_other: true,
        };
        assert!(all.complete());
        for broken in [
            Checks {
                fuse_conf: false,
                ..all
            },
            Checks {
                units: false,
                ..all
            },
            Checks {
                keeper_running: false,
                ..all
            },
            Checks {
                mount_allows_other: false,
                ..all
            },
        ] {
            assert!(!broken.complete(), "{broken:?}");
        }
    }

    #[test]
    fn setup_steps_track_the_checks_in_order() {
        let c = Checks {
            fuse_conf: true,
            units: true,
            keeper_running: false,
            ..Checks::default()
        };
        let steps = setup_steps(c);
        assert_eq!(steps.map(|s| s.2), [true, false, false]);
        assert!(steps[0].1.contains("/etc/fuse.conf"));
        assert!(steps[2].1.contains("Only you can open"));
    }

    #[test]
    fn consent_lists_every_change_on_a_fresh_system() {
        let cfg = Config::default();
        let items = consent_items(Checks::default(), &cfg);
        let titles: Vec<&str> = items.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Change a system setting: /etc/fuse.conf",
                "Install background services",
                "Change the Drive mount's settings",
                "Restart rclone-gdrive.service",
                "Afterwards",
            ]
        );
        let fuse = &items[0].1;
        assert!(fuse.contains("administrator password"), "{fuse}");
        assert!(
            fuse.contains("/etc/fuse.conf.cosmic-rclone-ui-<date-time>.bak"),
            "{fuse}"
        );
        assert!(items[2]
            .1
            .contains("--allow-other, --default-permissions and umask 077"));
        assert!(items[3].1.contains("~/GoogleDrive"));
    }

    #[test]
    fn consent_leaves_out_steps_already_done() {
        let c = Checks {
            fuse_conf: true,
            units: true,
            keeper_running: true,
            mounted: true,
            mount_allows_other: false,
        };
        let items = consent_items(c, &Config::default());
        let titles: Vec<&str> = items.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(titles, ["Restart rclone-gdrive.service", "Afterwards"]);
    }
}
