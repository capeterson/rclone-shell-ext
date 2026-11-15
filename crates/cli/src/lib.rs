// SPDX-License-Identifier: GPL-3.0-only
//! The command-line modes every front end shares: the offline-folder keeper and pin units the
//! systemd services run, `--pins`, `--status`, `--share-url` and `--migrate`. The
//! `rclone-shell-ext` binary is a thin wrapper around [`run`]; the front-end binaries call
//! [`run`] too for flags that aren't theirs, so there is exactly one implementation.

use rclone_ui_core::config::store;
use rclone_ui_core::{keeper, migrate, pin, rc, share, units};

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

/// Start-up common to every binary, after it has set the identity and installed the config
/// store: moves an old install over once (see [`rclone_ui_core::migrate`]). Does nothing for
/// the unit modes and for `--migrate` itself, which does it on request.
pub fn startup(args: &[String]) {
    if !is_unit_mode(args) && !has(args, "--migrate") {
        migrate::run_if_needed();
    }
}

/// What the `rclone-shell-ext` binary prints when it is given no mode.
pub const USAGE: &str = "usage: rclone-shell-ext MODE\n\
    \x20 --status [--rc-addr HOST:PORT]\n\
    \x20 --share-url PATH | --share-open PATH\n\
    \x20 --pin PATH | --unpin PATH | --pins\n\
    \x20 --offline-repair FOLDER | --offline-remove\n\
    \x20 --offline-keeper | --pin-mount ID | --pin-unmount ID   (run by the systemd units)\n\
    \x20 --migrate [--dry-run]    move an old cosmic-rclone-ui / rclone-plasma install over";

/// Run the mode the arguments ask for. `None` means none of the shared modes matched.
#[must_use]
pub fn run(args: &[String]) -> Option<i32> {
    if has(args, "--migrate") {
        return Some(report(migrate::command(has(args, "--dry-run"))));
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
    println!("{st:#?}\nhealth: {:?}", st.health());
    println!("recent: {} entries", recent.len());
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
            keeper::set_pinned(p, true).map(|r| format!("Keeping {r} available offline")),
        ));
    }
    if let Some(p) = arg(args, "--unpin") {
        return Some(report(
            keeper::set_pinned(p, false).map(|r| format!("No longer keeping {r} offline")),
        ));
    }
    // `--offline-repair FOLDER` asks the keeper to rebuild a folder's sync baseline.
    if let Some(rel) = arg(args, "--offline-repair") {
        return Some(report(
            keeper::request_repair(rel)
                .map(|()| format!("Repair of {rel} requested"))
                .map_err(|e| e.to_string()),
        ));
    }
    // `--offline-remove` takes out the units and mount drop-in (used when uninstalling). It
    // refuses while folders are kept offline, so local edits are never dropped unseen.
    if has(args, "--offline-remove") {
        let cfg = store().load();
        if !cfg.pins.is_empty() {
            return Some(report(Err(format!(
                "{} folder(s) are still kept offline; remove them in Settings first",
                cfg.pins.len()
            ))));
        }
        return Some(report(
            units::remove(&cfg)
                .and_then(|()| store().save(&mut |c| c.offline_enabled = false).map(|_| ()))
                .map(|()| "Offline folder services removed".into()),
        ));
    }
    // `--pins` lists offline folders and their state.
    if has(args, "--pins") {
        let cfg = store().load();
        let state = pin::OfflineState::load();
        let on = if cfg.offline_enabled { "on" } else { "off" };
        println!("offline folders: {on}");
        for rel in &cfg.pins {
            match state.pin(rel) {
                Some(p) => println!(
                    "  {rel}: {}{}",
                    p.phase.label(),
                    p.error
                        .as_deref()
                        .map_or_else(String::new, |e| format!(" ({e})"))
                ),
                None => println!("  {rel}: waiting for the keeper"),
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
    fn unit_modes_are_recognised() {
        assert!(is_unit_mode(&a(&["--offline-keeper"])));
        assert!(is_unit_mode(&a(&["--pin-mount", "x"])));
        assert!(!is_unit_mode(&a(&["--status"])));
        assert!(!is_unit_mode(&a(&["--migrate", "--dry-run"])));
    }

    #[test]
    fn no_mode_means_none() {
        assert_eq!(run(&a(&["--settings"])), None);
        assert_eq!(arg(&a(&["--pin", "x"]), "--pin"), Some("x"));
    }
}
