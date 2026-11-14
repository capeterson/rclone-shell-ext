// SPDX-License-Identifier: GPL-3.0-only
//! Opens one window the way the binary crate would: `smoke --menu PATH...` or `smoke --settings`.
use rclone_ui_core::identity::{set_identity, Identity};

fn main() {
    set_identity(Identity {
        slug: "rclone-plasma",
        app_id: "com.ca-peterson.RclonePlasma",
    });
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("--menu") => rclone_plasma_ui::run_details(&args[1..]),
        Some("--settings") => rclone_plasma_ui::run_settings(),
        _ => {
            eprintln!("usage: smoke --menu PATH... | --settings");
            2
        }
    };
    std::process::exit(code);
}
