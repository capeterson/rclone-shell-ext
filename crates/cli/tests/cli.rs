// SPDX-License-Identifier: GPL-3.0-only
//! Runs the real binary, with every XDG root pointed at a scratch directory so the user's real
//! config, data and systemd units are never touched (and no migration can run).

use std::{
    path::PathBuf,
    process::{Command, Output},
};

struct Sandbox {
    home: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let home = std::env::temp_dir().join(format!("rcs-cli-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&home).ok();
        std::fs::create_dir_all(&home).unwrap();
        Self { home }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rclone-shell-ext"))
            .args(args)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("RCLONE_SHELL_EXT_NO_MIGRATE", "1")
            .output()
            .unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.home).ok();
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// We can't assert on health here (it depends on the host's systemd and /proc/mounts), only that
/// the snapshot path runs and prints a verdict.
#[test]
fn status_mode_prints_a_health_verdict() {
    let sb = Sandbox::new("status");
    let out = sb.run(&["--status", "--rc-addr", "127.0.0.1:1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("health: "), "{stdout}");
}

#[test]
fn no_mode_prints_usage_and_fails() {
    let sb = Sandbox::new("usage");
    let out = sb.run(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("usage: rclone-shell-ext"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn pins_lists_nothing_on_a_fresh_config() {
    let sb = Sandbox::new("pins");
    let out = sb.run(&["--pins"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "offline folders: off\n"
    );
}

#[test]
fn share_url_refuses_paths_outside_the_mount() {
    let sb = Sandbox::new("outside");
    let out = sb.run(&["--share-url", "/definitely/not/in/the/mount"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("Not inside the rclone mount"),
        "{}",
        stderr(&out)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

#[test]
fn share_url_reports_an_unmounted_remote() {
    let sb = Sandbox::new("unmounted");
    // Inside the default mount path (~/GoogleDrive), which is not an rclone mount here.
    let path = sb.home.join("GoogleDrive/file.txt");
    let out = sb.run(&["--share-url", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not mounted"), "{}", stderr(&out));
}

#[test]
fn migrate_dry_run_with_nothing_old_changes_nothing() {
    let sb = Sandbox::new("migrate");
    let out = sb.run(&["--migrate", "--dry-run"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("No old install found"), "{text}");
    assert!(!sb.home.join("state").exists(), "a dry run must not write");
}
