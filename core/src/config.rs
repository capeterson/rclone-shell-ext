// SPDX-License-Identifier: GPL-3.0-only
//! Settings shared by every front end, and the seam through which a front end stores them.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock, PoisonError};

/// User-editable settings. Defaults match the rclone Google Drive setup this was written for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Address of rclone's `--rc` HTTP server.
    pub rc_addr: String,
    /// File holding `RCLONE_RC_USER=` / `RCLONE_RC_PASS=` lines (`~` is expanded).
    pub env_file: String,
    /// systemd *user* unit that runs `rclone mount`.
    pub service: String,
    /// Where the remote is mounted (`~` is expanded).
    pub mount_path: String,
    /// rclone's `--cache-dir` (`~` is expanded); read for per-file cache status.
    pub cache_dir: String,
    /// Add an "rclone…" entry to the COSMIC Files right-click menu.
    pub context_menu: bool,
    /// Offline folders are switched on (set up from the settings window).
    pub offline_enabled: bool,
    /// Folders kept available offline, relative to the mount root (no leading slash).
    pub pins: Vec<String>,
    /// How often each offline folder is synced with the remote when nothing changes locally.
    pub sync_interval_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rc_addr: "127.0.0.1:5572".into(),
            env_file: "~/.config/rclone/gdrive-rc.env".into(),
            service: "rclone-gdrive.service".into(),
            mount_path: "~/GoogleDrive".into(),
            cache_dir: "~/.cache/rclone".into(),
            context_menu: true,
            offline_enabled: false,
            pins: Vec::new(),
            sync_interval_secs: 300,
        }
    }
}

/// Where a front end keeps the settings (COSMIC config, KDE config, a file, ...).
pub trait ConfigStore: Send + Sync {
    /// The saved settings, falling back to defaults for anything unreadable.
    fn load(&self) -> Config;
    /// Change the saved settings: `f` edits a fresh copy, which is written back and returned.
    ///
    /// # Errors
    /// A message if the settings couldn't be written.
    fn save(&self, f: &mut dyn FnMut(&mut Config)) -> Result<Config, String>;
}

/// Keeps settings in memory only; used when no store was installed (and in tests).
#[derive(Debug, Default)]
pub struct MemoryStore(Mutex<Config>);

impl MemoryStore {
    #[must_use]
    pub fn new(cfg: Config) -> Self {
        Self(Mutex::new(cfg))
    }
}

impl ConfigStore for MemoryStore {
    fn load(&self) -> Config {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn save(&self, f: &mut dyn FnMut(&mut Config)) -> Result<Config, String> {
        let mut cfg = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut cfg);
        Ok(cfg.clone())
    }
}

static STORE: OnceLock<Box<dyn ConfigStore>> = OnceLock::new();

/// Install the process-wide store. Call once at start-up; later calls are ignored.
pub fn set_store(store: Box<dyn ConfigStore>) {
    let _ = STORE.set(store);
}

/// The process-wide store (an in-memory one with defaults if none was installed).
pub fn store() -> &'static dyn ConfigStore {
    STORE
        .get_or_init(|| Box::new(MemoryStore::default()))
        .as_ref()
}

/// Expand a leading `~/` (or a bare `~`) using `$HOME`.
pub fn expand(path: &str) -> String {
    expand_with(path, std::env::var("HOME").ok().as_deref())
}

fn expand_with(path: &str, home: Option<&str>) -> String {
    match (home, path) {
        (Some(home), "~") => home.to_string(),
        (Some(home), p) => p
            .strip_prefix("~/")
            .map_or_else(|| p.to_string(), |rest| format!("{home}/{rest}")),
        (None, p) => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde_prefix() {
        assert_eq!(expand_with("~/Drive", Some("/home/u")), "/home/u/Drive");
        assert_eq!(expand_with("~", Some("/home/u")), "/home/u");
    }

    #[test]
    fn leaves_other_paths_alone() {
        assert_eq!(expand_with("/mnt/x", Some("/home/u")), "/mnt/x");
        assert_eq!(expand_with("~other/x", Some("/home/u")), "~other/x");
        assert_eq!(expand_with("a/~/b", Some("/home/u")), "a/~/b");
    }

    #[test]
    fn no_home_means_no_expansion() {
        assert_eq!(expand_with("~/Drive", None), "~/Drive");
    }

    #[test]
    fn defaults_are_stable() {
        let c = Config::default();
        assert_eq!(c.rc_addr, "127.0.0.1:5572");
        assert_eq!(c.service, "rclone-gdrive.service");
    }
}
