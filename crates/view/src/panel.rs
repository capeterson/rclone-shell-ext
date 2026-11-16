// SPDX-License-Identifier: GPL-3.0-only
//! The panel popup's text, shared by the COSMIC applet and the Plasma daemon's `StatusJson`
//! (which feeds the plasmoid). Every function returns finished, translated text; the toolkits
//! only lay it out. Nothing here changes case or glues translated fragments together except
//! through a message (`panel-caption-join`), so translators control the sentence.

use crate::ago;
use rclone_ui_core::pin::{self, OfflineState};
use rclone_ui_core::rc::{self, human, ChangeKind, CompletedTransfer, Direction, Health, Status};
use rclone_ui_core::version::MIN_RCLONE;
use rclone_ui_i18n::fl;

/// The plain headline for a health state; `None` means the first poll hasn't finished.
pub fn headline(health: Option<Health>) -> String {
    match health {
        Some(Health::Healthy) => fl!("health-healthy"),
        Some(Health::Uploading) => fl!("health-uploading"),
        Some(Health::Downloading) => fl!("health-downloading"),
        Some(Health::Transferring) => fl!("health-transferring"),
        Some(Health::FileErrors) => fl!("health-file-errors"),
        Some(Health::CacheFull) => fl!("health-cache-full"),
        Some(Health::Unreachable) => fl!("health-unreachable"),
        Some(Health::NotMounted) => fl!("health-not-mounted"),
        Some(Health::ServiceStopped) => fl!("health-service-stopped"),
        None => fl!("health-checking"),
    }
}

/// The headline with a file count while files are moving or failing ("Uploading: 3 files").
pub fn health_label(st: &Status) -> String {
    let health = st.health();
    let transfers = || {
        if st.transfers.is_empty() {
            st.uploads_in_progress + st.uploads_queued
        } else {
            st.transfers.len() as u64
        }
    };
    match health {
        Health::Uploading if transfers() > 0 => {
            fl!("health-uploading-files", count = transfers())
        }
        Health::Downloading if transfers() > 0 => {
            fl!("health-downloading-files", count = transfers())
        }
        Health::Transferring if transfers() > 0 => {
            fl!("health-transferring-files", count = transfers())
        }
        Health::FileErrors if st.errored_files > 0 => {
            fl!("health-file-errors-files", count = st.errored_files)
        }
        h => headline(Some(h)),
    }
}

/// "1.0 MiB/s".
pub fn speed_text(bytes_per_second: f64) -> String {
    fl!("panel-speed", size = human(bytes_per_second))
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// The body lines of the status tooltip (excluding per-transfer rows), in display order.
pub fn detail_lines(st: &Status) -> Vec<String> {
    let mut lines = vec![fl!(
        "panel-detail-state",
        mounted = yes_no(st.mounted),
        running = yes_no(st.service_active)
    )];
    if let Some(e) = &st.rc_error {
        lines.push(fl!("panel-detail-rc-unreachable", error = e.as_str()));
    }
    if let Some(v) = &st.rclone_version {
        lines.push(fl!("panel-detail-rclone-version", version = v.as_str()));
    }
    lines.push(fl!(
        "panel-detail-uploads",
        active = st.uploads_in_progress,
        queued = st.uploads_queued
    ));
    lines.push(fl!(
        "panel-detail-cache",
        files = st.cache_files,
        size = human(st.cache_bytes as f64)
    ));
    if st.errored_files > 0 || st.out_of_space {
        lines.push(if st.out_of_space {
            fl!("panel-detail-errored-full", count = st.errored_files)
        } else {
            fl!("panel-detail-errored", count = st.errored_files)
        });
    }
    lines
}

/// "Last error: …".
pub fn last_error_line(error: &str) -> String {
    fl!("panel-last-error", error = error)
}

/// "Failed: …", for a history row whose last transfer failed.
pub fn failed_line(error: &str) -> String {
    fl!("panel-failed", error = error)
}

/// The warning shown when the running rclone is older than the minimum this app supports.
pub fn outdated_text(version: &str) -> String {
    fl!(
        "panel-outdated-rclone",
        version = version,
        minimum = MIN_RCLONE
    )
}

/// "Offline: 2 folders · all available offline", or a nudge to finish setup.
pub fn offline_line(pins: &[String], state: &OfflineState) -> String {
    if state.updated == 0 {
        fl!("panel-offline-setup")
    } else {
        fl!("panel-offline-summary", summary = pin::summary(pins, state))
    }
}

/// One line describing a transfer. `size` is negative when rclone doesn't know it.
pub fn transfer_line(t: &rc::Transfer) -> String {
    let done = human(t.bytes as f64);
    let speed = speed_text(t.speed);
    if t.size < 0 {
        fl!(
            "panel-transfer-line-unknown-size",
            name = t.name.as_str(),
            done = done,
            speed = speed
        )
    } else {
        fl!(
            "panel-transfer-line",
            name = t.name.as_str(),
            done = done,
            total = human(t.size as f64),
            speed = speed
        )
    }
}

/// "…and 3 more", for transfers that don't fit.
pub fn more_transfers(count: usize) -> String {
    fl!("panel-more-transfers", count = count)
}

/// "Show all (12)".
pub fn show_all(count: usize) -> String {
    fl!("panel-show-all", count = count)
}

/// The history page's summary line.
pub fn history_summary(count: usize) -> String {
    fl!("panel-history-summary", count = count)
}

/// Split a remote path into its folder (possibly empty) and file name.
pub fn split_path(name: &str) -> (&str, &str) {
    let trimmed = name.trim_end_matches('/');
    trimmed.rsplit_once('/').unwrap_or(("", trimmed))
}

/// Second line of a history row: `Folder · 1.2 MiB · 5 minutes ago · ×3`. Empty when nothing
/// is known.
pub fn recent_caption(t: &CompletedTransfer, now: jiff::Timestamp) -> String {
    let (dir, _) = split_path(&t.name);
    let mut parts: Vec<String> = vec![];
    if !dir.is_empty() {
        parts.push(dir.to_string());
    }
    if t.size >= 0 {
        parts.push(human(t.size as f64));
    }
    if let Some(at) = t.at {
        parts.push(ago(now.as_second(), at.as_second()));
    }
    if t.times > 1 {
        parts.push(fl!("panel-times", count = t.times));
    }
    let mut parts = parts.into_iter();
    let first = parts.next().unwrap_or_default();
    parts.fold(first, |head, tail| {
        fl!("panel-caption-join", head = head, tail = tail)
    })
}

/// The kind icon (a freedesktop name) and its tooltip: new, changed or deleted.
pub fn kind_icon(kind: ChangeKind) -> (&'static str, String) {
    match kind {
        ChangeKind::Created => ("document-new-symbolic", fl!("panel-kind-created")),
        ChangeKind::Updated => ("document-edit-symbolic", fl!("panel-kind-updated")),
        ChangeKind::Deleted => ("user-trash-symbolic", fl!("panel-kind-deleted")),
    }
}

/// The direction arrow's tooltip, worded for what happened to the file.
pub fn direction_tip(direction: Direction, kind: Option<ChangeKind>) -> String {
    match (direction, kind) {
        (Direction::Upload, Some(ChangeKind::Deleted)) => fl!("panel-tip-deleted-upload"),
        (Direction::Download, Some(ChangeKind::Deleted)) => fl!("panel-tip-deleted-download"),
        (Direction::Upload, _) => fl!("panel-tip-uploaded"),
        (Direction::Download, _) => fl!("panel-tip-downloaded"),
    }
}

/// Why the history list is empty, in words.
pub fn empty_history_text(st: &Status) -> String {
    if !st.service_active || st.rc_error.is_some() {
        fl!("panel-history-unavailable")
    } else {
        fl!("panel-history-empty")
    }
}

/// What a pin the keeper hasn't reported on yet is doing.
pub fn pin_waiting() -> String {
    fl!("panel-pin-waiting")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rclone_ui_core::identity::use_test_identity;

    fn ts(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    fn status() -> Status {
        use_test_identity();
        Status {
            service_active: true,
            mounted: true,
            rc_error: None,
            uploads_in_progress: 0,
            uploads_queued: 0,
            cache_files: 3,
            cache_bytes: 2048,
            errored_files: 0,
            out_of_space: false,
            speed: 0.0,
            last_error: None,
            transfers: vec![],
            recent: vec![],
            rclone_version: None,
        }
    }

    fn done(name: &str, size: i64, at: Option<&str>) -> CompletedTransfer {
        CompletedTransfer {
            name: name.into(),
            size,
            bytes: size,
            at: at.map(ts),
            error: None,
            direction: None,
            times: 1,
            kind: None,
        }
    }

    #[test]
    fn headline_covers_every_state() {
        use_test_identity();
        assert_eq!(headline(None), "Checking…");
        assert_eq!(headline(Some(Health::Healthy)), "Healthy");
        assert_eq!(headline(Some(Health::NotMounted)), "Not mounted");
        assert_eq!(headline(Some(Health::ServiceStopped)), "Service stopped");
        assert_eq!(
            headline(Some(Health::Transferring)),
            "Uploading and downloading"
        );
    }

    #[test]
    fn health_label_counts_files_with_plurals() {
        let mut st = status();
        st.uploads_in_progress = 1;
        assert_eq!(health_label(&st), "Uploading: 1 file");
        st.uploads_queued = 2;
        assert_eq!(health_label(&st), "Uploading: 3 files");
        st.service_active = false;
        assert_eq!(health_label(&st), "Service stopped");
        st.service_active = true;
        st.uploads_in_progress = 0;
        st.uploads_queued = 0;
        assert_eq!(health_label(&st), "Healthy");
        st.errored_files = 2;
        assert_eq!(health_label(&st), "Some files failed: 2 files");
    }

    #[test]
    fn detail_lines_idle() {
        let l = detail_lines(&status());
        assert_eq!(l[0], "Mount: up   Service: running");
        assert_eq!(l[1], "Uploads: 0 in progress, 0 queued");
        assert_eq!(l[2], "Cache: 3 files, 2.0 KiB");
        assert_eq!(l.len(), 3, "no error lines when healthy: {l:?}");
    }

    #[test]
    fn detail_lines_report_problems() {
        let mut s = status();
        s.mounted = false;
        s.service_active = false;
        s.rc_error = Some("timeout".into());
        s.errored_files = 2;
        s.out_of_space = true;
        let l = detail_lines(&s);
        assert_eq!(l[0], "Mount: down   Service: stopped");
        assert!(l.contains(&"rc unreachable: timeout".to_string()));
        assert!(l.contains(&"Errored files: 2 (cache out of space)".to_string()));
        s.out_of_space = false;
        assert!(detail_lines(&s).contains(&"Errored files: 2".to_string()));
    }

    #[test]
    fn outdated_text_names_both_versions() {
        use_test_identity();
        let t = outdated_text("v1.60.1");
        assert!(t.contains("v1.60.1") && t.contains(MIN_RCLONE), "{t}");
    }

    #[test]
    fn transfer_line_handles_unknown_size() {
        use_test_identity();
        let mut t = rc::Transfer {
            name: "a.bin".into(),
            size: -1,
            bytes: 1024,
            percentage: 0,
            speed: 0.0,
            src_fs: None,
            dst_fs: None,
        };
        assert_eq!(transfer_line(&t), "a.bin — 1.0 KiB / ? at 0.0 B/s");
        t.size = 2048;
        assert_eq!(transfer_line(&t), "a.bin — 1.0 KiB / 2.0 KiB at 0.0 B/s");
    }

    #[test]
    fn small_messages() {
        use_test_identity();
        assert_eq!(more_transfers(3), "…and 3 more");
        assert_eq!(show_all(12), "Show all (12)");
        assert_eq!(
            history_summary(5),
            "Last 5 files since rclone started · newest first"
        );
        assert_eq!(speed_text(1_048_576.0), "1.0 MiB/s");
        assert_eq!(last_error_line("x"), "Last error: x");
        assert_eq!(failed_line("x"), "Failed: x");
        assert_eq!(pin_waiting(), "Waiting for the keeper");
    }

    #[test]
    fn split_path_separates_folder_and_file() {
        assert_eq!(split_path("a/b/c.txt"), ("a/b", "c.txt"));
        assert_eq!(split_path("c.txt"), ("", "c.txt"));
        assert_eq!(split_path("a/b/"), ("a", "b"));
    }

    #[test]
    fn icons_explain_kind_and_direction() {
        use_test_identity();
        assert_eq!(kind_icon(ChangeKind::Created).1, "New file");
        assert_eq!(kind_icon(ChangeKind::Deleted).0, "user-trash-symbolic");
        assert_eq!(
            direction_tip(Direction::Upload, None),
            "Uploaded to the cloud"
        );
        assert_eq!(
            direction_tip(Direction::Download, Some(ChangeKind::Updated)),
            "Downloaded from the cloud"
        );
        assert_eq!(
            direction_tip(Direction::Upload, Some(ChangeKind::Deleted)),
            "Deleted here, so removed from the cloud"
        );
        assert_eq!(
            direction_tip(Direction::Download, Some(ChangeKind::Deleted)),
            "Deleted in the cloud, so removed here"
        );
    }

    #[test]
    fn recent_caption_joins_the_known_parts() {
        use_test_identity();
        let now = ts("2026-11-02T12:00:00Z");
        let mut t = done("Docs/a.bin", 2048, Some("2026-11-02T11:55:00Z"));
        assert_eq!(recent_caption(&t, now), "Docs · 2.0 KiB · 5 minutes ago");
        t.times = 3;
        assert_eq!(
            recent_caption(&t, now),
            "Docs · 2.0 KiB · 5 minutes ago · ×3"
        );
        let t = done("top.bin", -1, None);
        assert_eq!(recent_caption(&t, now), "", "nothing known, nothing shown");
    }

    #[test]
    fn empty_history_text_explains_itself() {
        let mut s = status();
        assert_eq!(
            empty_history_text(&s),
            "Nothing synced since rclone started"
        );
        s.rc_error = Some("down".into());
        assert_eq!(empty_history_text(&s), "History unavailable");
        s.rc_error = None;
        s.service_active = false;
        assert_eq!(empty_history_text(&s), "History unavailable");
    }

    #[test]
    fn offline_line_nudges_until_the_keeper_has_reported() {
        use_test_identity();
        let state = OfflineState::default();
        assert_eq!(
            offline_line(&[], &state),
            "Offline folders: finish setup in Settings"
        );
        let state = OfflineState {
            updated: 1,
            ..OfflineState::default()
        };
        assert_eq!(
            offline_line(&[], &state),
            "Offline: No folders kept offline"
        );
    }
}
