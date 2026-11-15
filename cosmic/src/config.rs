// SPDX-License-Identifier: GPL-3.0-only
//! COSMIC side of the settings: the plain [`Config`] and its shared `config.toml` store live in
//! `rclone-ui-core`; here are the conveniences the UI code uses, and the change watch.

pub use rclone_ui_core::config::{expand, Config};
use rclone_ui_core::config::{store, TomlStore};
use std::time::{Duration, SystemTime};

/// How often the windows and the applet look at the settings file for changes made elsewhere
/// (the Plasma front end, `--pin`, a text editor).
pub const WATCH_EVERY: Duration = Duration::from_secs(2);

/// `Config::load()` / `Config::save(..)` for the UI code, through the shared store.
pub trait ConfigExt: Sized {
    /// Load the user's saved settings, falling back to defaults for anything unreadable.
    fn load() -> Self;
    /// Change the saved settings: `f` edits a fresh copy, which is written back. Running
    /// windows pick the change up through their [`ConfigWatch`].
    fn save(f: impl FnOnce(&mut Config)) -> Result<Config, String>;
}

impl ConfigExt for Config {
    fn load() -> Self {
        store().load()
    }

    fn save(f: impl FnOnce(&mut Config)) -> Result<Config, String> {
        let mut f = Some(f);
        store().save(&mut |c| {
            if let Some(f) = f.take() {
                f(c);
            }
        })
    }
}

/// Notices when `config.toml` changes (its modification time moves) so a window can reload.
/// The first poll only records the file's time.
#[derive(Debug, Default)]
pub struct ConfigWatch {
    state: Option<(TomlStore, Option<SystemTime>)>,
}

impl ConfigWatch {
    /// The settings if the file changed since the last poll.
    pub fn poll(&mut self) -> Option<Config> {
        let (file, seen) = self.state.get_or_insert_with(|| {
            let file = TomlStore::new(TomlStore::default_path());
            let seen = file.mtime();
            (file, seen)
        });
        let now = file.changed_since(*seen)?;
        *seen = now;
        Some(store().load())
    }
}
