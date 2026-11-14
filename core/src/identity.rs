// SPDX-License-Identifier: GPL-3.0-only
//! Who is running: the names derived from the front end's identity (data and state
//! directories, systemd unit names, backup file names).

use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Short name for directories and unit names, e.g. `cosmic-rclone-ui`
    /// (`cosmic-rclone-ui-dev` for the side-by-side dev build).
    pub slug: &'static str,
    /// The application id, e.g. `com.ca-peterson.CosmicRcloneUi`.
    pub app_id: &'static str,
}

const DEFAULT: Identity = Identity {
    slug: "cosmic-rclone-ui",
    app_id: "com.ca-peterson.CosmicRcloneUi",
};

static IDENTITY: OnceLock<Identity> = OnceLock::new();

/// Set the process identity. Call once at start-up; later calls are ignored.
pub fn set_identity(id: Identity) {
    let _ = IDENTITY.set(id);
}

/// The process identity (the COSMIC applet's if none was set).
pub fn identity() -> &'static Identity {
    IDENTITY.get_or_init(|| DEFAULT)
}
