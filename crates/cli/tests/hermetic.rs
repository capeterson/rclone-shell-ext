// SPDX-License-Identifier: GPL-3.0-only
//! A test run (anything that used a test identity) must never act on the machine's real old
//! install: `migrate::pending()` and `guard()` say "nothing to do" even when the XDG roots show
//! an old install. This is its own test binary because it sets environment variables.

use rclone_ui_core::{identity, migrate};

#[test]
fn test_runs_ignore_an_old_install_in_the_xdg_roots() {
    let root = std::env::temp_dir().join(format!("rcs-hermetic-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    let (config, data, state) = (root.join("c"), root.join("d"), root.join("s"));
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("rclone-plasmarc"), "[General]\n").unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &config);
    std::env::set_var("XDG_DATA_HOME", &data);
    std::env::set_var("XDG_STATE_HOME", &state);
    std::env::remove_var("RCLONE_SHELL_EXT_NO_MIGRATE");

    // The roots do show an old install...
    assert_eq!(
        migrate::pending_in(&config, &data, &state),
        migrate::MigrationStatus::Pending
    );
    // ...but a test run never looks at the process's roots.
    identity::use_test_identity();
    assert_eq!(migrate::pending(), migrate::MigrationStatus::NotNeeded);
    assert!(migrate::guard().is_ok());
    std::fs::remove_dir_all(&root).ok();
}
