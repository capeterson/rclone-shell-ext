// SPDX-License-Identifier: GPL-3.0-only
//! COSMIC side of the settings: the plain [`Config`] lives in `rclone-ui-core`; here it is
//! stored with `cosmic_config`.

use cosmic::cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry};
use rclone_ui_core::config::ConfigStore;
pub use rclone_ui_core::config::{expand, Config};

/// Mirror of [`Config`] that carries the `CosmicConfigEntry` derive. Field names, types and
/// the version are what's on disk, so existing configs keep working.
#[derive(Debug, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct CosmicConfig {
    pub rc_addr: String,
    pub env_file: String,
    pub service: String,
    pub mount_path: String,
    pub cache_dir: String,
    pub context_menu: bool,
    pub offline_enabled: bool,
    pub pins: Vec<String>,
    pub sync_interval_secs: u64,
}

impl From<Config> for CosmicConfig {
    fn from(c: Config) -> Self {
        Self {
            rc_addr: c.rc_addr,
            env_file: c.env_file,
            service: c.service,
            mount_path: c.mount_path,
            cache_dir: c.cache_dir,
            context_menu: c.context_menu,
            offline_enabled: c.offline_enabled,
            pins: c.pins,
            sync_interval_secs: c.sync_interval_secs,
        }
    }
}

impl From<CosmicConfig> for Config {
    fn from(c: CosmicConfig) -> Self {
        Self {
            rc_addr: c.rc_addr,
            env_file: c.env_file,
            service: c.service,
            mount_path: c.mount_path,
            cache_dir: c.cache_dir,
            context_menu: c.context_menu,
            offline_enabled: c.offline_enabled,
            pins: c.pins,
            sync_interval_secs: c.sync_interval_secs,
        }
    }
}

impl Default for CosmicConfig {
    fn default() -> Self {
        Config::default().into()
    }
}

/// [`ConfigStore`] backed by the COSMIC config system.
pub struct CosmicStore {
    app_id: &'static str,
}

impl CosmicStore {
    #[must_use]
    pub fn new(app_id: &'static str) -> Self {
        Self { app_id }
    }
}

impl ConfigStore for CosmicStore {
    fn load(&self) -> Config {
        cosmic_config::Config::new(self.app_id, CosmicConfig::VERSION)
            .map(|ctx| match CosmicConfig::get_entry(&ctx) {
                Ok(c) | Err((_, c)) => c.into(),
            })
            .unwrap_or_default()
    }

    fn save(&self, f: &mut dyn FnMut(&mut Config)) -> Result<Config, String> {
        let ctx = cosmic_config::Config::new(self.app_id, CosmicConfig::VERSION)
            .map_err(|e| e.to_string())?;
        let mut cfg: Config = match CosmicConfig::get_entry(&ctx) {
            Ok(c) | Err((_, c)) => c.into(),
        };
        f(&mut cfg);
        CosmicConfig::from(cfg.clone())
            .write_entry(&ctx)
            .map_err(|e| e.to_string())?;
        Ok(cfg)
    }
}

/// `Config::load(APP_ID)` / `Config::save(APP_ID, ..)` for the UI code, through COSMIC config.
pub trait ConfigExt: Sized {
    /// Load the user's saved settings, falling back to defaults for anything unreadable.
    fn load(app_id: &'static str) -> Self;
    /// Change the saved settings: `f` edits a fresh copy, which is written back. Running apps
    /// pick the change up through their config watch.
    fn save(app_id: &'static str, f: impl FnOnce(&mut Config)) -> Result<Config, String>;
}

impl ConfigExt for Config {
    fn load(app_id: &'static str) -> Self {
        CosmicStore::new(app_id).load()
    }

    fn save(app_id: &'static str, f: impl FnOnce(&mut Config)) -> Result<Config, String> {
        let mut f = Some(f);
        CosmicStore::new(app_id).save(&mut |c| {
            if let Some(f) = f.take() {
                f(c);
            }
        })
    }
}
