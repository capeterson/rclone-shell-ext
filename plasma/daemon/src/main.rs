// SPDX-License-Identifier: GPL-3.0-only
mod cli;
mod daemon;
mod emblems;
mod kconfig;
mod recheck;
mod setconfig;
mod snapshot;

use rclone_ui_core::config::set_store;
use rclone_ui_core::identity::{set_identity, Identity};

fn main() {
    set_identity(Identity {
        slug: "rclone-plasma",
        app_id: "com.ca-peterson.RclonePlasma",
    });
    set_store(Box::new(kconfig::KConfigStore::new(
        kconfig::KConfigStore::default_path(),
    )));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    cli::usage();
    std::process::exit(2);
}
