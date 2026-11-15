// SPDX-License-Identifier: GPL-3.0-only
mod cli;
mod daemon;
mod debounce;
mod emblems;
mod recheck;
mod setconfig;
mod snapshot;

use rclone_ui_core::identity::{set_identity, Identity};

fn main() {
    set_identity(Identity::from_override(option_env!("APP_ID_OVERRIDE")));
    rclone_ui_core::config::install_default_store();
    let args: Vec<String> = std::env::args().skip(1).collect();
    rclone_shell_ext_cli::startup(&args);
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    cli::usage();
    std::process::exit(2);
}
