// SPDX-License-Identifier: GPL-3.0-only
//! Toolkit-free logic for the rclone UI front ends (COSMIC applet, KDE Plasma, ...).
//!
//! Two things a binary sets up first: [`identity::set_identity`] (the names used for units,
//! directories and backups; there is no default) and [`config::install_default_store`]
//! (the shared `config.toml`).

pub mod autostart;
pub mod bisync_name;
pub mod config;
pub mod conflict;
pub mod drive;
pub mod fuse_conf;
pub mod history;
pub mod identity;
pub mod keeper;
pub mod migrate;
pub mod mime_icon;
pub mod offline_setup;
pub mod pin;
pub mod rc;
pub mod share;
pub mod status;
#[cfg(test)]
mod testutil;
pub mod units;
pub mod version;
