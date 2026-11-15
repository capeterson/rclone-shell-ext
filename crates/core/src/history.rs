// SPDX-License-Identifier: GPL-3.0-only
//! Recent-transfer history helpers shared by the front ends.

use crate::pin::Change;
use crate::rc::{self, CompletedTransfer};

/// rclone's transfer history with offline-folder changes mixed in, newest first. rclone can't
/// say what kind of change a transfer was, so offline folders' come from the keeper.
#[must_use]
pub fn with_changes(recent: Vec<CompletedTransfer>, changes: &[Change]) -> Vec<CompletedTransfer> {
    if changes.is_empty() {
        return recent;
    }
    let mut all = recent;
    all.extend(changes.iter().map(|c| CompletedTransfer {
        name: c.path.clone(),
        size: c.size,
        bytes: c.size.max(0),
        at: jiff::Timestamp::from_second(c.at).ok(),
        error: None,
        direction: Some(c.direction),
        times: 1,
        kind: Some(c.kind),
    }));
    // Stable, so rclone's own order holds among equal times; unknown times go last.
    all.sort_by_key(|t| std::cmp::Reverse(t.at));
    all.truncate(rc::HISTORY_CAP);
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rc::{ChangeKind, Direction};

    fn ts(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    fn done(name: &str, size: i64, at: Option<&str>) -> CompletedTransfer {
        CompletedTransfer {
            name: name.into(),
            size,
            bytes: size.max(0),
            at: at.map(ts),
            error: None,
            direction: None,
            times: 1,
            kind: None,
        }
    }

    #[test]
    fn offline_changes_are_merged_into_history_by_time() {
        let recent = vec![
            done("new.bin", 1, Some("2026-11-02T12:00:00Z")),
            done("old.bin", 1, Some("2026-11-02T10:00:00Z")),
        ];
        let changes = [Change {
            at: ts("2026-11-02T11:00:00Z").as_second(),
            path: "Games/GCS/x.eqp".into(),
            size: 7,
            kind: ChangeKind::Updated,
            direction: Direction::Upload,
        }];
        let merged = with_changes(recent.clone(), &changes);
        let names: Vec<&str> = merged.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["new.bin", "Games/GCS/x.eqp", "old.bin"]);
        assert_eq!(merged[1].kind, Some(ChangeKind::Updated));
        assert_eq!(merged[1].direction, Some(Direction::Upload));
        assert_eq!(merged[1].size, 7);
        assert_eq!(with_changes(recent.clone(), &[]), recent);
    }
}
