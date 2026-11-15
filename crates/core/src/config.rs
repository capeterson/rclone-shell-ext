// SPDX-License-Identifier: GPL-3.0-only
//! Settings shared by every front end: one `config.toml` that all of them read and write.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

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

/// Where the settings are kept (the shared TOML file, or memory in tests).
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

/// `$XDG_CONFIG_HOME` (default `~/.config`).
pub fn xdg_config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(expand("~/.config")), PathBuf::from)
}

/// The settings file: `<config home>/<slug>/config.toml`.
pub fn config_path_in(config_home: &Path, slug: &str) -> PathBuf {
    config_home.join(slug).join("config.toml")
}

/// Settings kept in `$XDG_CONFIG_HOME/<slug>/config.toml`. Every front end uses this one file.
///
/// Reading never fails: a missing or unparsable file gives the defaults, and a field with an
/// unknown name, or a value of the wrong type, is ignored on its own (the other fields keep
/// what the file says). Writing replaces the file atomically (temp file, then rename), so a
/// reader never sees half a file.
#[derive(Debug)]
pub struct TomlStore {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl TomlStore {
    /// A store at `path`.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_lock: Mutex::new(()),
        }
    }

    /// The settings file for the process identity, under `$XDG_CONFIG_HOME`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        config_path_in(&xdg_config_home(), crate::identity::identity().slug)
    }

    /// Where the file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// When the file was last modified, or `None` if it doesn't exist. Front ends that show
    /// settings compare this against the last value they saw to notice outside changes.
    #[must_use]
    pub fn mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok()
    }

    /// Has the file changed since `seen` (a previous [`mtime`](Self::mtime))? Returns the new
    /// value to remember when it has.
    #[must_use]
    pub fn changed_since(&self, seen: Option<SystemTime>) -> Option<Option<SystemTime>> {
        let now = self.mtime();
        (now != seen).then_some(now)
    }

    fn read(&self) -> Config {
        parse_toml(&std::fs::read_to_string(&self.path).unwrap_or_default())
    }
}

impl ConfigStore for TomlStore {
    fn load(&self) -> Config {
        self.read()
    }

    fn save(&self, f: &mut dyn FnMut(&mut Config)) -> Result<Config, String> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut cfg = self.read();
        f(&mut cfg);
        let text = toml::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        write_atomic(&self.path, &text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(cfg)
    }
}

/// The settings in `text`, with the default for every field that is missing or invalid.
pub fn parse_toml(text: &str) -> Config {
    let Ok(table) = toml::from_str::<toml::value::Table>(text) else {
        return Config::default();
    };
    let Ok(toml::Value::Table(mut merged)) = toml::Value::try_from(Config::default()) else {
        return Config::default();
    };
    // Take each key from the file only if the whole struct still reads with it.
    for (key, value) in table {
        if !merged.contains_key(&key) {
            continue;
        }
        let Some(old) = merged.insert(key.clone(), value) else {
            continue;
        };
        if toml::Value::Table(merged.clone())
            .try_into::<Config>()
            .is_err()
        {
            merged.insert(key, old);
        }
    }
    toml::Value::Table(merged).try_into().unwrap_or_default()
}

/// Write `text` to `path` through a temp file in the same directory and a rename.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Install the shared `config.toml` store for the process identity. Every binary calls this
/// right after [`crate::identity::set_identity`].
pub fn install_default_store() {
    set_store(Box::new(TomlStore::new(TomlStore::default_path())));
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

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rcs-toml-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = temp_dir("missing");
        let s = TomlStore::new(config_path_in(&dir, "x"));
        assert_eq!(s.load(), Config::default());
        assert_eq!(s.mtime(), None);
    }

    #[test]
    fn save_and_load_round_trip_with_atomic_write() {
        let dir = temp_dir("rt");
        let path = config_path_in(&dir, "x");
        let s = TomlStore::new(path.clone());
        let saved = s
            .save(&mut |c| {
                c.rc_addr = "10.0.0.1:9".into();
                c.offline_enabled = true;
                c.pins = vec!["A".into(), "b \\ \"c\"".into()];
                c.sync_interval_secs = 42;
            })
            .unwrap();
        assert_eq!(s.load(), saved);
        assert_eq!(TomlStore::new(path.clone()).load(), saved);
        // No temp files are left behind.
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["config.toml"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_and_unknown_fields_fall_back_one_by_one() {
        let c = parse_toml(
            "rc_addr = \"x:1\"\ncontext_menu = \"maybe\"\nsync_interval_secs = -4\n\
             mystery = 1\npins = [\"a\", \"b\"]\n",
        );
        assert_eq!(c.rc_addr, "x:1");
        assert!(c.context_menu);
        assert_eq!(c.sync_interval_secs, 300);
        assert_eq!(c.pins, ["a", "b"]);
        assert_eq!(parse_toml("not = [valid"), Config::default());
        assert_eq!(parse_toml(""), Config::default());
    }

    #[test]
    fn change_detection_follows_the_file() {
        let dir = temp_dir("mtime");
        let s = TomlStore::new(config_path_in(&dir, "x"));
        let seen = s.mtime();
        assert_eq!(s.changed_since(seen), None);
        s.save(&mut |c| c.context_menu = false).unwrap();
        let now = s.changed_since(seen).expect("the file appeared");
        assert!(now.is_some());
        assert_eq!(s.changed_since(now), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn defaults_are_stable() {
        let c = Config::default();
        assert_eq!(c.rc_addr, "127.0.0.1:5572");
        assert_eq!(c.service, "rclone-gdrive.service");
    }
}
