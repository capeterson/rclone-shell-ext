// SPDX-License-Identifier: GPL-3.0-only
//! `rclone-shell-ext`: the shared binary the systemd units run, and the command line for
//! everything that doesn't need a window.

use rclone_ui_core::identity::{set_identity, Identity};

fn main() {
    // `APP_ID_OVERRIDE` at build time makes the side-by-side dev build (see the justfiles).
    set_identity(Identity::from_override(option_env!("APP_ID_OVERRIDE")));
    rclone_ui_core::config::install_default_store();
    let args: Vec<String> = std::env::args().skip(1).collect();
    rclone_shell_ext_cli::startup(&args);
    if let Some(code) = rclone_shell_ext_cli::run(&args) {
        std::process::exit(code);
    }
    eprintln!("{}", rclone_shell_ext_cli::usage());
    std::process::exit(2);
}
