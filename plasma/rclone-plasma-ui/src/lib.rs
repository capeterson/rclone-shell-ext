// SPDX-License-Identifier: GPL-3.0-only
//! Kirigami windows for rclone-plasma. The binary crate calls these entry points after it has
//! installed the config store and identity (`rclone_ui_core::config::set_store`,
//! `rclone_ui_core::identity::set_identity`). Each runs a Qt event loop and returns an exit code.

/// `rclone-plasma --menu PATH...`: the details window for a mounted file or folder.
pub fn run_details(paths: &[String]) -> i32 {
    let _ = paths;
    eprintln!("details window not implemented yet");
    1
}

/// `rclone-plasma --settings`: the settings window (offline folders setup, integrations).
pub fn run_settings() -> i32 {
    eprintln!("settings window not implemented yet");
    1
}
