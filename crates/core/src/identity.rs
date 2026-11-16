// SPDX-License-Identifier: GPL-3.0-only
//! Who is running: the names derived from the front end's identity (data and state
//! directories, systemd unit names, backup file names).
//!
//! There is no default: a binary must call [`set_identity`] before anything asks for the
//! identity, so a forgotten call fails loudly instead of quietly using someone else's names.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// The shared application id, the same for every front end.
pub const APP_ID: &str = "com.ca-peterson.rclone-shell-ext";
/// The shared slug: directory names, unit names and the shared binary's name.
pub const SLUG: &str = "rclone-shell-ext";
/// The side-by-side dev build's application id.
pub const DEV_APP_ID: &str = "com.ca-peterson.rclone-shell-ext-dev";
/// The side-by-side dev build's slug.
pub const DEV_SLUG: &str = "rclone-shell-ext-dev";

/// Prefix of the status icons and Dolphin emblems in the icon theme. The icons are shared by
/// the release and dev builds, so this does not follow the dev identity.
pub const ICON_PREFIX: &str = "rclone-shell-ext";

/// The name of the status icon for `state` (`synced`, `uploading`, ...).
#[must_use]
pub fn status_icon(state: &str) -> String {
    format!("{ICON_PREFIX}-{state}")
}

/// The name of the Dolphin emblem for `state` (`synced`, `offline`, ...).
#[must_use]
pub fn emblem_icon(state: &str) -> String {
    format!("{ICON_PREFIX}-emblem-{state}")
}

/// The COSMIC front end's desktop id (panel applet entry and `.desktop` file).
pub const COSMIC_DESKTOP_ID: &str = "com.ca-peterson.rclone-shell-ext.Cosmic";
/// The dev build's COSMIC desktop id.
pub const DEV_COSMIC_DESKTOP_ID: &str = "com.ca-peterson.rclone-shell-ext-dev.Cosmic";
/// The Plasma front end's desktop id (the settings launcher).
pub const PLASMA_DESKTOP_ID: &str = "com.ca-peterson.rclone-shell-ext.Plasma";

/// The Plasma daemon's D-Bus name for a normal install (the same as [`APP_ID`]).
pub const DBUS_NAME: &str = APP_ID;
/// The dev build's D-Bus name, so a dev and a release daemon never fight over one name.
pub const DEV_DBUS_NAME: &str = DEV_APP_ID;
/// The Plasma daemon's object path (D-Bus paths cannot contain `-`).
pub const DBUS_PATH: &str = "/com/ca_peterson/rclone_shell_ext";
/// The Plasma daemon's interface (interface names cannot contain `-`).
pub const DBUS_INTERFACE: &str = "com.ca_peterson.RcloneShellExt1";
/// The organisation name Qt applications set (settings and cache directories).
pub const ORGANIZATION: &str = "ca-peterson";

/// The names one process works under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Short name for directories and unit names, e.g. `rclone-shell-ext`
    /// (`rclone-shell-ext-dev` for the side-by-side dev build).
    pub slug: &'static str,
    /// The application id, e.g. `com.ca-peterson.rclone-shell-ext`.
    pub app_id: &'static str,
}

impl Identity {
    /// The identity of a normal install.
    pub const RELEASE: Identity = Identity {
        slug: SLUG,
        app_id: APP_ID,
    };

    /// The identity of the side-by-side dev build.
    pub const DEV: Identity = Identity {
        slug: DEV_SLUG,
        app_id: DEV_APP_ID,
    };

    /// The identity a build asks for through its `APP_ID_OVERRIDE` environment variable
    /// (`None` is a normal build).
    ///
    /// # Panics
    /// If the override names something other than [`DEV_APP_ID`]; a typo in a build recipe
    /// should stop the build, not install under a made-up name.
    #[must_use]
    pub fn from_override(app_id_override: Option<&str>) -> Identity {
        match app_id_override {
            None => Identity::RELEASE,
            Some(DEV_APP_ID) => Identity::DEV,
            Some(other) => panic!("APP_ID_OVERRIDE must be {DEV_APP_ID}, not {other}"),
        }
    }

    /// The D-Bus name the Plasma daemon owns: [`DBUS_NAME`], or [`DEV_DBUS_NAME`] for the dev
    /// build.
    #[must_use]
    pub fn dbus_name(&self) -> &'static str {
        if self.is_dev() {
            DEV_DBUS_NAME
        } else {
            DBUS_NAME
        }
    }

    /// Is this the dev build's identity?
    #[must_use]
    pub fn is_dev(&self) -> bool {
        self.slug == DEV_SLUG
    }
}

static IDENTITY: OnceLock<Identity> = OnceLock::new();

/// Set the process identity. Call once at start-up; later calls are ignored.
pub fn set_identity(id: Identity) {
    let _ = IDENTITY.set(id);
}

fn require(slot: &OnceLock<Identity>) -> &Identity {
    slot.get().expect(
        "rclone-ui-core: identity() called before set_identity(); \
         every binary must set its identity first thing in main (tests: use_test_identity())",
    )
}

/// The process identity.
///
/// # Panics
/// If [`set_identity`] was never called (the `test-identity` feature, for tests, falls back to
/// the release identity instead).
pub fn identity() -> &'static Identity {
    #[cfg(any(test, feature = "test-identity"))]
    IDENTITY.get_or_init(|| {
        mark_test();
        Identity::RELEASE
    });
    require(&IDENTITY)
}

/// The process identity if one was set, without panicking (for code that only peeks, such as
/// `migrate::pending`, which tests of other crates reach before they set one).
pub fn try_identity() -> Option<&'static Identity> {
    IDENTITY.get()
}

/// For tests (here and in other crates): set the release identity, so code that names units
/// and directories works without a `main`. Does nothing if an identity is already set.
pub fn use_test_identity() {
    mark_test();
    let _ = IDENTITY.set(Identity::RELEASE);
}

/// Set once a test identity was used: code that would touch the real home directory (the
/// migration's `pending()`, `guard()`) then behaves as if nothing needed doing.
static TEST_RUN: AtomicBool = AtomicBool::new(false);

fn mark_test() {
    TEST_RUN.store(true, Ordering::Relaxed);
}

/// Is this process a test run (a test identity was used)? Never true in a shipped binary.
pub(crate) fn is_test_run() -> bool {
    TEST_RUN.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "before set_identity()")]
    fn unset_identity_panics_with_a_clear_message() {
        require(&OnceLock::new());
    }

    #[test]
    fn override_maps_to_the_dev_identity() {
        assert_eq!(Identity::from_override(None), Identity::RELEASE);
        assert_eq!(Identity::from_override(Some(DEV_APP_ID)), Identity::DEV);
        assert!(Identity::DEV.is_dev() && !Identity::RELEASE.is_dev());
    }

    #[test]
    fn dev_and_release_daemons_own_different_bus_names() {
        assert_eq!(Identity::RELEASE.dbus_name(), DBUS_NAME);
        assert_eq!(Identity::DEV.dbus_name(), DEV_DBUS_NAME);
        assert_ne!(DBUS_NAME, DEV_DBUS_NAME);
    }

    #[test]
    #[should_panic(expected = "APP_ID_OVERRIDE must be")]
    fn unknown_override_is_refused() {
        let _ = Identity::from_override(Some("com.example.Other"));
    }
}
