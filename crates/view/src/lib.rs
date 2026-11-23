// SPDX-License-Identifier: GPL-3.0-only
//! Toolkit-free presentation model behind the windows and panels: what to show and which buttons apply. Everything
//! here is plain Rust so it can be unit tested without a Qt event loop.

pub mod backup;
pub mod details;
pub mod history;
pub mod panel;
pub mod settings;

use rclone_ui_i18n::fl;
use std::time::Duration;

/// A value that is on its way, arrived, or failed.
#[derive(Debug, Clone, Default)]
pub enum Load<T> {
    #[default]
    Pending,
    Done(Result<T, String>),
}

impl<T> Load<T> {
    pub fn ok(&self) -> Option<&T> {
        match self {
            Load::Done(Ok(v)) => Some(v),
            _ => None,
        }
    }

    pub fn err(&self) -> Option<&str> {
        match self {
            Load::Done(Err(e)) => Some(e),
            _ => None,
        }
    }

    /// Neither arrived nor failed yet.
    pub fn waiting(&self) -> bool {
        matches!(self, Load::Pending)
    }
}

/// How long the window re-reads transfer counts and the keeper's progress.
pub const REFRESH: Duration = Duration::from_secs(2);

/// Show the loading screen at least this long, so a fast load doesn't flash it.
pub const MIN_LOADING: Duration = Duration::from_millis(450);

/// How much longer the loading screen must stay up after `elapsed` since it appeared.
pub fn remaining_loading(elapsed: Duration) -> Duration {
    MIN_LOADING.saturating_sub(elapsed)
}

/// "5 minutes ago" style text for a Unix timestamp, relative to `now` (both in seconds).
pub fn ago(now: i64, then: i64) -> String {
    let secs = (now - then).max(0);
    let (minutes, hours, days): (i64, i64, i64) = (secs / 60, secs / 3600, secs / 86400);
    match secs {
        0..=59 => fl!("common-ago-now"),
        60..=3599 => fl!("common-ago-minutes", count = minutes),
        3600..=86399 => fl!("common-ago-hours", count = hours),
        _ => fl!("common-ago-days", count = days),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_screen_has_a_minimum_time() {
        assert_eq!(remaining_loading(Duration::ZERO), MIN_LOADING);
        assert_eq!(
            remaining_loading(Duration::from_millis(100)),
            Duration::from_millis(350)
        );
        assert_eq!(remaining_loading(MIN_LOADING), Duration::ZERO);
        assert_eq!(remaining_loading(MIN_LOADING * 3), Duration::ZERO);
    }

    #[test]
    fn load_accessors() {
        let pending: Load<u8> = Load::Pending;
        assert!(pending.waiting() && pending.ok().is_none() && pending.err().is_none());
        let ok: Load<u8> = Load::Done(Ok(1));
        assert_eq!(ok.ok(), Some(&1));
        assert!(!ok.waiting());
        let bad: Load<u8> = Load::Done(Err("x".into()));
        assert_eq!(bad.err(), Some("x"));
        assert!(!bad.waiting());
    }

    #[test]
    fn ago_text() {
        assert_eq!(ago(100, 100), "just now");
        assert_eq!(ago(100, 200), "just now");
        assert_eq!(ago(160, 100), "1 minute ago");
        assert_eq!(ago(100 + 3 * 60, 100), "3 minutes ago");
        assert_eq!(ago(100 + 3600, 100), "1 hour ago");
        assert_eq!(ago(100 + 2 * 86400, 100), "2 days ago");
    }
}
