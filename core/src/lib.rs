// SPDX-License-Identifier: GPL-3.0-only
//! Toolkit-free logic for the rclone UI front ends (COSMIC applet, KDE Plasma, ...).
//!
//! Two seams let a front end plug in: [`config::set_store`] (where settings live) and
//! [`identity::set_identity`] (the names used for units, directories and backups).

pub mod config;
pub mod conflict;
pub mod drive;
pub mod fuse_conf;
pub mod history;
pub mod identity;
pub mod keeper;
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
