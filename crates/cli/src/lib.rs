// SPDX-License-Identifier: GPL-3.0-only
//! The command-line modes every front end shares: the offline-folder keeper and pin units the
//! systemd services run, `--pins`, `--status`, `--share-url` and `--migrate` and `--plasma-panel-rename`. The
//! `rclone-shell-ext` binary is a thin wrapper around [`run`]; the front-end binaries call
//! [`run`] too for flags that aren't theirs, so there is exactly one implementation.

use rclone_ui_core::config::store;
use rclone_ui_core::migrate::plasma_panel;
use rclone_ui_core::{keeper, migrate, pin, rc, share, units};
use rclone_ui_i18n::fl;

/// The value after `flag`, if present.
fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// The modes the systemd units run. They must not start a migration: the migration stops
/// those very units.
#[must_use]
pub fn is_unit_mode(args: &[String]) -> bool {
    ["--offline-keeper", "--pin-mount", "--pin-unmount"]
        .iter()
        .any(|f| has(args, f))
}

/// The command-line modes that change offline folders: they migrate an old install first, in the
/// foreground. Read-only modes (`--status`, `--pins`, `--share-url`, `--share-open`) never wait
/// on a migration, and neither does `--offline-remove`, which uninstall must always be able to
/// run.
#[must_use]
pub fn is_cli_mode(args: &[String]) -> bool {
    ["--pin", "--unpin", "--offline-repair"]
        .iter()
        .any(|f| has(args, f))
}

/// Start-up common to every binary, after it has set the identity and installed the config
/// store. For the command-line modes it moves an old install over first (see
/// [`rclone_ui_core::migrate`]); on failure it prints why on stderr and exits non-zero. It does
/// nothing for the unit modes (the migration stops those very units), for `--migrate` itself,
/// and for the long-running front ends (the daemon and the applet), which never block their
/// start-up on it: they call [`rclone_ui_core::migrate::spawn_background`] once they are up.
pub fn startup(args: &[String]) {
    if is_cli_mode(args) && !is_unit_mode(args) && !has(args, "--migrate") {
        // run_if_needed has already printed the reason.
        if migrate::run_if_needed().is_err() {
            std::process::exit(1);
        }
    }
}

/// What the `rclone-shell-ext` binary prints when it is given no mode.
#[must_use]
pub fn usage() -> String {
    fl!("cli-usage")
}

/// Run the mode the arguments ask for. `None` means none of the shared modes matched.
#[must_use]
pub fn run(args: &[String]) -> Option<i32> {
    if has(args, "--migrate") {
        return Some(report(
            migrate::command(has(args, "--dry-run")).map_err(|e| e.message),
        ));
    }
    if has(args, "--plasma-panel-rename") {
        let path = rclone_ui_core::config::xdg_config_home().join(plasma_panel::APPLETSRC);
        return Some(report(panel_rename(
            &path,
            has(args, "--dry-run"),
            &plasma_panel::plasmashell_running,
        )));
    }
    if let Some(code) = offline_cli(args) {
        return Some(code);
    }
    if let Some(path) = arg(args, "--share-url") {
        return Some(share_cmd(path, false));
    }
    if let Some(path) = arg(args, "--share-open") {
        return Some(share_cmd(path, true));
    }
    if has(args, "--status") {
        status_cmd(args);
        return Some(0);
    }
    None
}

/// `--plasma-panel-rename`: see [`plasma_panel::apply`]. `running` is injected for tests.
fn panel_rename(
    path: &std::path::Path,
    dry_run: bool,
    running: &dyn Fn() -> bool,
) -> Result<String, String> {
    let shown = path.display().to_string();
    match plasma_panel::apply(path, dry_run, running) {
        Ok(plasma_panel::Outcome::Nothing(_)) => Ok(fl!("cli-panel-nothing", path = shown)),
        Ok(plasma_panel::Outcome::Changed(_)) if dry_run => {
            Ok(fl!("cli-panel-would-change", path = shown))
        }
        Ok(plasma_panel::Outcome::Changed(_)) => Ok(fl!("cli-panel-changed", path = shown)),
        Err(None) => Err(fl!("cli-panel-running", path = shown)),
        Err(Some(error)) => Err(fl!("cli-panel-failed", path = shown, error = error)),
    }
}

fn share_cmd(path: &str, open: bool) -> i32 {
    let cfg = store().load();
    match share::resolve(&cfg, &[path.to_string()]) {
        Ok(t) => {
            let url = share::sharing_url(&t);
            if open {
                share::open_url(&url);
            } else {
                println!("{url}");
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// `--status [--rc-addr HOST:PORT]` prints one status snapshot and exits (handy for debugging).
fn status_cmd(args: &[String]) {
    let mut cfg = store().load();
    if let Some(addr) = arg(args, "--rc-addr") {
        addr.clone_into(&mut cfg.rc_addr);
    }
    let mut st = rc::fetch(&cfg);
    let recent = std::mem::take(&mut st.recent);
    println!(
        "{st:#?}\n{}",
        fl!("cli-status-health", health = format!("{:?}", st.health()))
    );
    println!("{}", fl!("cli-status-recent", count = recent.len()));
    for t in recent.iter().take(10) {
        println!("  {t:?}");
    }
}

fn report(r: Result<String, String>) -> i32 {
    match r {
        Ok(msg) => {
            if !msg.is_empty() {
                println!("{msg}");
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Offline-folder commands that don't open a window. Returns the exit code if one ran.
fn offline_cli(args: &[String]) -> Option<i32> {
    // Run by the systemd units this app installs (see core's `units.rs`).
    if has(args, "--offline-keeper") {
        keeper::run();
    }
    if let Some(id) = arg(args, "--pin-mount") {
        return Some(report(keeper::pin_mount(id).map(|()| String::new())));
    }
    if let Some(id) = arg(args, "--pin-unmount") {
        keeper::pin_unmount(id);
        return Some(0);
    }
    // `--pin PATH` / `--unpin PATH` keep a folder offline or stop doing so.
    if let Some(p) = arg(args, "--pin") {
        return Some(report(
            keeper::set_pinned(p, true).map(|r| fl!("cli-pin-keeping", folder = r)),
        ));
    }
    if let Some(p) = arg(args, "--unpin") {
        return Some(report(
            keeper::set_pinned(p, false).map(|r| fl!("cli-pin-stopped", folder = r)),
        ));
    }
    // `--offline-repair FOLDER` asks the keeper to rebuild a folder's sync baseline.
    if let Some(rel) = arg(args, "--offline-repair") {
        return Some(report(
            keeper::request_repair(rel)
                .map(|()| fl!("cli-repair-requested", folder = rel))
                .map_err(|e| e.to_string()),
        ));
    }
    // `--offline-remove` takes out the units and mount drop-in (used when uninstalling). It
    // refuses while folders are kept offline, so local edits are never dropped unseen.
    if has(args, "--offline-remove") {
        let cfg = store().load();
        if !cfg.pins.is_empty() {
            return Some(report(Err(fl!(
                "cli-offline-still-pinned",
                count = cfg.pins.len()
            ))));
        }
        return Some(report(
            units::remove(&cfg)
                .and_then(|()| store().save(&mut |c| c.offline_enabled = false).map(|_| ()))
                .map(|()| fl!("cli-offline-removed")),
        ));
    }
    // `--pins` lists offline folders and their state.
    if has(args, "--pins") {
        let cfg = store().load();
        let state = pin::OfflineState::load();
        println!(
            "{}",
            if cfg.offline_enabled {
                fl!("cli-pins-header-on")
            } else {
                fl!("cli-pins-header-off")
            }
        );
        for rel in &cfg.pins {
            match state.pin(rel) {
                Some(p) => match &p.error {
                    Some(e) => println!(
                        "  {}",
                        fl!(
                            "cli-pin-line-error",
                            folder = rel.as_str(),
                            phase = p.phase.label(),
                            error = e.label()
                        )
                    ),
                    None => println!(
                        "  {}",
                        fl!(
                            "cli-pin-line",
                            folder = rel.as_str(),
                            phase = p.phase.label()
                        )
                    ),
                },
                None => println!("  {}", fl!("cli-pin-waiting", folder = rel.as_str())),
            }
        }
        return Some(0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn panel_rename_refuses_while_plasmashell_runs() {
        rclone_ui_i18n::init();
        let d = std::env::temp_dir().join(format!("rse-cli-panel-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join(plasma_panel::APPLETSRC);
        std::fs::write(&f, "[A]\nplugin=com.ca-peterson.rcloneplasma\n").unwrap();
        let err = panel_rename(&f, false, &|| true).unwrap_err();
        assert!(err.contains("plasmashell is running"), "{err}");
        assert!(std::fs::read_to_string(&f)
            .unwrap()
            .contains("rcloneplasma"));
        assert!(panel_rename(&f, true, &|| true)
            .unwrap()
            .starts_with("Would rename"));
        assert!(panel_rename(&f, false, &|| false)
            .unwrap()
            .starts_with("Renamed"));
        assert!(panel_rename(&f, false, &|| true)
            .unwrap()
            .starts_with("No old"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn still_pinned_message_is_plural_aware() {
        rclone_ui_i18n::init();
        assert!(fl!("cli-offline-still-pinned", count = 1).starts_with("1 folder is still"));
        assert!(fl!("cli-offline-still-pinned", count = 3).starts_with("3 folders are still"));
    }

    #[test]
    fn unit_modes_are_recognised() {
        assert!(is_unit_mode(&a(&["--offline-keeper"])));
        assert!(is_unit_mode(&a(&["--pin-mount", "x"])));
        assert!(!is_unit_mode(&a(&["--status"])));
        assert!(!is_unit_mode(&a(&["--migrate", "--dry-run"])));
    }

    #[test]
    fn only_command_line_modes_migrate_at_startup() {
        assert!(is_cli_mode(&a(&["--pin", "x"])));
        assert!(is_cli_mode(&a(&["--unpin", "x"])));
        assert!(is_cli_mode(&a(&["--offline-repair"])));
        // Read-only modes must not block on a migration.
        for args in [
            &["--status"][..],
            &["--pins"],
            &["--share-url", "x"],
            &["--share-open", "x"],
        ] {
            assert!(!is_cli_mode(&a(args)), "{args:?}");
        }
        // The daemon, the applet, windows, uninstall and the migration itself are not.
        for args in [
            &["--daemon"][..],
            &[],
            &["--settings"],
            &["--offline-remove"],
            &["--migrate"],
            &["--offline-keeper"],
        ] {
            assert!(!is_cli_mode(&a(args)), "{args:?}");
        }
    }

    #[test]
    fn no_mode_means_none() {
        assert_eq!(run(&a(&["--settings"])), None);
        assert_eq!(arg(&a(&["--pin", "x"]), "--pin"), Some("x"));
    }
}
