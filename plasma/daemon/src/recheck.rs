// SPDX-License-Identifier: GPL-3.0-only
//! When to look at a cached path again: settle rechecks after transfer activity, the periodic
//! round-robin safety net, and the per-folder throttle. All pure, with injected `Instant`s.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Follow-up delays after a path saw transfer or history activity. rclone writes the VFS
/// metadata shortly after a transfer ends, so the first look may be too early.
pub const SETTLE_DELAYS: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(10)];
/// Entries for the same path due this close together count as one.
const DEDUPE_WINDOW: Duration = Duration::from_millis(1500);

pub const SWEEP_EVERY: Duration = Duration::from_secs(30);
pub const SWEEP_BATCH: usize = 200;

/// A folder is recomputed at most this often (it walks the cache tree).
pub const FOLDER_MIN_INTERVAL: Duration = Duration::from_secs(10);

/// Paths waiting for a follow-up recheck, each with the time it is due.
#[derive(Default)]
pub struct SettleQueue {
    items: Vec<(Instant, String)>,
}

impl SettleQueue {
    /// Schedule each path at `now + delay` for every settle delay, skipping near-duplicates.
    pub fn schedule<'a>(&mut self, paths: impl IntoIterator<Item = &'a String>, now: Instant) {
        for p in paths {
            for d in SETTLE_DELAYS {
                let due = now + d;
                let dup = self.items.iter().any(|(t, q)| {
                    q == p && (if *t > due { *t - due } else { due - *t }) < DEDUPE_WINDOW
                });
                if !dup {
                    self.items.push((due, p.clone()));
                }
            }
        }
    }

    /// Remove and return the distinct paths that are due at `now`, earliest first.
    pub fn drain_due(&mut self, now: Instant) -> Vec<String> {
        let mut due: Vec<(Instant, String)> = Vec::new();
        self.items.retain(|(t, p)| {
            if *t <= now {
                due.push((*t, p.clone()));
                false
            } else {
                true
            }
        });
        due.sort();
        let mut seen = HashSet::new();
        due.into_iter()
            .filter_map(|(_, p)| seen.insert(p.clone()).then_some(p))
            .collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// Take up to `max` paths from `all` (sorted in place for a stable order) starting at
/// `*cursor`, wrapping around, and advance the cursor. Every path is visited once per
/// `ceil(len / max)` calls.
pub fn round_robin(all: &mut [String], cursor: &mut usize, max: usize) -> Vec<String> {
    if all.is_empty() || max == 0 {
        return Vec::new();
    }
    all.sort_unstable();
    let n = all.len();
    let start = *cursor % n;
    let take = max.min(n);
    *cursor = (start + take) % n;
    (0..take).map(|i| all[(start + i) % n].clone()).collect()
}

/// Remembers when each folder was last computed.
#[derive(Default)]
pub struct FolderThrottle {
    last: HashMap<String, Instant>,
}

impl FolderThrottle {
    /// `Ok` (and records `now`) if the folder may be computed; otherwise `Err` with the time
    /// it becomes allowed.
    pub fn admit(&mut self, path: &str, now: Instant) -> Result<(), Instant> {
        if let Some(t) = self.last.get(path) {
            let next = *t + FOLDER_MIN_INTERVAL;
            if next > now {
                return Err(next);
            }
        }
        if self.last.len() > 2000 {
            self.last.retain(|_, t| *t + FOLDER_MIN_INTERVAL > now);
        }
        self.last.insert(path.to_string(), now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> String {
        x.to_string()
    }

    #[test]
    fn settle_schedules_two_followups_and_dedupes() {
        let t0 = Instant::now();
        let mut q = SettleQueue::default();
        let p = vec![s("/a"), s("/b")];
        q.schedule(&p, t0);
        assert_eq!(q.len(), 4);
        q.schedule(&p, t0);
        assert_eq!(q.len(), 4, "same trigger twice adds nothing");
        assert_eq!(q.drain_due(t0 + Duration::from_millis(2900)).len(), 0);
        assert_eq!(q.drain_due(t0 + Duration::from_secs(3)), ["/a", "/b"]);
        assert_eq!(q.len(), 2);
        assert_eq!(q.drain_due(t0 + Duration::from_secs(9)).len(), 0);
        assert_eq!(q.drain_due(t0 + Duration::from_secs(10)), ["/a", "/b"]);
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn settle_drain_is_distinct_and_late_triggers_extend() {
        let t0 = Instant::now();
        let mut q = SettleQueue::default();
        q.schedule(&[s("/a")], t0);
        q.schedule(&[s("/a")], t0 + Duration::from_secs(2));
        assert_eq!(q.len(), 4, "a trigger 2 s later is a separate window");
        // Everything is overdue at once: still one entry.
        assert_eq!(q.drain_due(t0 + Duration::from_secs(60)), ["/a"]);
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn round_robin_covers_all_and_wraps() {
        let mut all: Vec<String> = ["d", "a", "c", "b", "e"].map(s).to_vec();
        let mut cur = 0;
        assert_eq!(round_robin(&mut all, &mut cur, 2), ["a", "b"]);
        assert_eq!(round_robin(&mut all, &mut cur, 2), ["c", "d"]);
        assert_eq!(round_robin(&mut all, &mut cur, 2), ["e", "a"]);
        assert_eq!(round_robin(&mut all, &mut cur, 2), ["b", "c"]);
        let mut cur = 0;
        assert_eq!(
            round_robin(&mut all, &mut cur, 99).len(),
            5,
            "capped at len"
        );
        assert_eq!(round_robin(&mut [], &mut cur, 5).len(), 0);
        let mut cur = 7;
        assert_eq!(
            round_robin(&mut all, &mut cur, 1),
            ["c"],
            "stale cursor wraps"
        );
    }

    #[test]
    fn folder_throttle() {
        let t0 = Instant::now();
        let mut th = FolderThrottle::default();
        assert!(th.admit("/f", t0).is_ok());
        assert_eq!(
            th.admit("/f", t0 + Duration::from_secs(4)),
            Err(t0 + FOLDER_MIN_INTERVAL)
        );
        assert!(th.admit("/g", t0 + Duration::from_secs(4)).is_ok());
        assert!(th.admit("/f", t0 + FOLDER_MIN_INTERVAL).is_ok());
    }
}
