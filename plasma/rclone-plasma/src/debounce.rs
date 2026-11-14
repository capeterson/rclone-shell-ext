// SPDX-License-Identifier: GPL-3.0-only
//! The recent-history debounce from the COSMIC applet's `sync_history`: the first fill is
//! immediate; later changes show up `DELAY` after they were first seen.

use rclone_ui_core::rc::CompletedTransfer;
use std::time::{Duration, Instant};

pub const DELAY: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct History {
    shown: Vec<CompletedTransfer>,
    pending_since: Option<Instant>,
}

impl History {
    pub fn shown(&self) -> &[CompletedTransfer] {
        &self.shown
    }

    /// Feed the newest history; returns true if what is shown changed.
    pub fn update(&mut self, latest: &[CompletedTransfer], now: Instant) -> bool {
        if latest == self.shown {
            self.pending_since = None;
            return false;
        }
        if self.shown.is_empty() {
            self.shown = latest.to_vec();
            self.pending_since = None;
            return true;
        }
        match self.pending_since {
            None => {
                self.pending_since = Some(now);
                false
            }
            Some(t) if now.duration_since(t) >= DELAY => {
                self.shown = latest.to_vec();
                self.pending_since = None;
                true
            }
            Some(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: &str) -> CompletedTransfer {
        CompletedTransfer {
            name: n.into(),
            size: 1,
            bytes: 1,
            at: None,
            error: None,
            direction: None,
            times: 1,
            kind: None,
        }
    }

    #[test]
    fn first_fill_is_immediate_then_debounced() {
        let t0 = Instant::now();
        let mut h = History::default();
        assert!(!h.update(&[], t0));
        assert!(h.update(&[item("a")], t0));
        assert_eq!(h.shown().len(), 1);
        let two = [item("b"), item("a")];
        assert!(!h.update(&two, t0 + Duration::from_secs(1)));
        assert!(
            !h.update(&two, t0 + Duration::from_secs(2)),
            "2s after first seen"
        );
        assert!(h.update(&two, t0 + Duration::from_secs(3)));
        assert_eq!(h.shown().len(), 2);
    }

    #[test]
    fn reverting_cancels_pending() {
        let t0 = Instant::now();
        let mut h = History::default();
        h.update(&[item("a")], t0);
        assert!(!h.update(&[item("b")], t0));
        assert!(!h.update(&[item("a")], t0 + Duration::from_secs(1)));
        assert!(
            !h.update(&[item("b")], t0 + Duration::from_secs(5)),
            "timer restarted"
        );
        assert!(h.update(&[item("b")], t0 + Duration::from_secs(7)));
    }
}
