// SPDX-License-Identifier: GPL-3.0-only
//! The Plasma binary's own modes (windows, the daemon, `--status --json`). Everything else is
//! the shared command line in `rclone-shell-ext`.

use crate::{daemon, snapshot};
use rclone_ui_core::config::store;

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
    if let Some(i) = args.iter().position(|a| a == "--menu" || a == "--details") {
        return Some(rclone_shell_ext_plasma_ui::run_details(&args[i + 1..]));
    }
    if has(args, "--settings") {
        return Some(rclone_shell_ext_plasma_ui::run_settings());
    }
    if has(args, "--daemon") {
        return Some(daemon::run());
    }
    if has(args, "--status") && has(args, "--json") {
        status_json(args);
        return Some(0);
    }
    // `--status`, `--share-url`, `--pin`, `--migrate`, ... are the shared command line.
    rclone_shell_ext_cli::run(args)
}

pub fn usage() {
    eprintln!(
        "usage: rclone-shell-ext-plasma MODE\n\
         \x20 --daemon                 run the D-Bus service\n\
         \x20 --status --json [--rc-addr HOST:PORT]\n\
         \x20 --menu PATH...           details window (also --details)\n\
         \x20 --settings               settings window\n\
         plus the shared modes of `rclone-shell-ext`:\n{}",
        rclone_shell_ext_cli::USAGE
    );
}

/// `--status --json`: the daemon's snapshot document. Plain `--status` is the shared mode.
fn status_json(args: &[String]) {
    let mut cfg = store().load();
    if let Some(addr) = arg(args, "--rc-addr") {
        addr.clone_into(&mut cfg.rc_addr);
    }
    let doc = snapshot::one_shot(&cfg);
    println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
}
