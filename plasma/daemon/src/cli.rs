// SPDX-License-Identifier: GPL-3.0-only
//! Command-line modes, ported from the COSMIC applet's `main.rs`.

use crate::{daemon, snapshot};
use rclone_ui_core::config::store;
use rclone_ui_core::{keeper, pin, rc, share, units};

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

/// Run the mode the arguments ask for. `None` means there was no mode: the caller prints usage.
pub fn run(args: &[String]) -> Option<i32> {
    if let Some(i) = args.iter().position(|a| a == "--menu") {
        return Some(rclone_plasma_ui::run_details(&args[i + 1..]));
    }
    if has(args, "--settings") {
        return Some(rclone_plasma_ui::run_settings());
    }
    if has(args, "--daemon") {
        return Some(daemon::run());
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

pub fn usage() {
    eprintln!(
        "usage: rclone-plasma MODE\n\
         \x20 --daemon                 run the D-Bus service\n\
         \x20 --status [--json] [--rc-addr HOST:PORT]\n\
         \x20 --menu PATH...           details window\n\
         \x20 --settings               settings window\n\
         \x20 --share-url PATH | --share-open PATH\n\
         \x20 --pin PATH | --unpin PATH | --pins\n\
         \x20 --offline-repair FOLDER | --offline-remove | --offline-keeper\n\
         \x20 --pin-mount ID | --pin-unmount ID"
    );
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

fn status_cmd(args: &[String]) {
    let mut cfg = store().load();
    if let Some(addr) = arg(args, "--rc-addr") {
        addr.clone_into(&mut cfg.rc_addr);
    }
    if has(args, "--json") {
        let doc = snapshot::one_shot(&cfg);
        println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
        return;
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
    if let Some(rel) = arg(args, "--offline-repair") {
        return Some(report(
            keeper::request_repair(rel)
                .map(|()| format!("Repair of {rel} requested"))
                .map_err(|e| e.to_string()),
        ));
    }
    // Refuses while folders are kept offline, so local edits are never dropped unseen.
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
                        .as_ref()
                        .map_or_else(String::new, |e| format!(" ({})", e.label()))
                ),
                None => println!("  {rel}: waiting for the keeper"),
            }
        }
        return Some(0);
    }
    None
}
