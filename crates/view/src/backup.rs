// SPDX-License-Identifier: GPL-3.0-only
//! The settings window's Backups section: the folders this machine backs up, and the backups in
//! Drive that can be restored. Both front ends show exactly these rows and labels.

use crate::ago;
use rclone_ui_core::backup::{self, BackupStatus, RemoteBackup};
use rclone_ui_core::config::Config;
use rclone_ui_i18n::fl;
use serde_json::json;

/// One backed-up folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRow {
    /// The local folder (what "Stop backing up" passes back to `backup::remove`).
    pub source: String,
    /// Where it goes, without the remote name: `Computers/laptop/Documents`.
    pub destination: String,
    /// "Backed up 5 minutes ago", "Backing up…", "Not backed up yet", or the error.
    pub status: String,
    /// `status` is an error.
    pub error: bool,
}

/// `destination` without the `remote:` prefix.
fn display_destination(destination: &str) -> String {
    destination
        .split_once(':')
        .map_or(destination, |(_, p)| p)
        .to_string()
}

/// The rows for every backed-up folder, in config order.
#[must_use]
pub fn rows(cfg: &Config, status: &BackupStatus, now: i64) -> Vec<BackupRow> {
    cfg.backup_folders
        .iter()
        .map(|f| {
            let st = status.folders.get(&f.source);
            let running = status.running && status.current.as_deref() == Some(f.source.as_str());
            let (text, error) = match st {
                _ if running => (fl!("backup-status-running"), false),
                Some(s) if s.error.is_some() => (
                    backup::error_label(s.error.as_deref().unwrap_or_default()),
                    true,
                ),
                Some(s) => s.last_success.map_or_else(
                    || (fl!("backup-status-pending"), false),
                    |t| {
                        let t = i64::try_from(t).unwrap_or(i64::MAX);
                        (fl!("backup-status-done", when = ago(now, t)), false)
                    },
                ),
                None => (fl!("backup-status-pending"), false),
            };
            BackupRow {
                source: f.source.clone(),
                destination: display_destination(&f.destination),
                status: text,
                error,
            }
        })
        .collect()
}

/// [`rows`] as a JSON array for QML: `[{source, destination, status, error}]`.
#[must_use]
pub fn rows_json(rows: &[BackupRow]) -> String {
    serde_json::Value::Array(
        rows.iter()
            .map(|r| {
                json!({
                    "source": r.source, "destination": r.destination,
                    "status": r.status, "error": r.error,
                })
            })
            .collect(),
    )
    .to_string()
}

/// One backup in Drive that can be restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRow {
    /// "laptop / Documents".
    pub label: String,
    /// What to pass to `backup::spawn_restore`.
    pub destination: String,
}

/// The restorable backups, sorted by machine then folder.
#[must_use]
pub fn restore_rows(list: &[RemoteBackup]) -> Vec<RestoreRow> {
    let mut rows: Vec<_> = list
        .iter()
        .map(|b| RestoreRow {
            label: fl!(
                "settings-restore-row",
                host = b.host.as_str(),
                folder = b.name.as_str()
            ),
            destination: b.destination.clone(),
        })
        .collect();
    rows.sort_by(|a, b| a.label.cmp(&b.label));
    rows
}

/// [`restore_rows`] as a JSON array for QML: `[{label, destination}]`.
#[must_use]
pub fn restore_rows_json(rows: &[RestoreRow]) -> String {
    serde_json::Value::Array(
        rows.iter()
            .map(|r| json!({ "label": r.label, "destination": r.destination }))
            .collect(),
    )
    .to_string()
}

/// The section's description, naming the folder backups go to.
#[must_use]
pub fn description() -> String {
    fl!(
        "settings-backup-description",
        folder = format!("{}/{}", backup::COMPUTERS, backup::hostname())
    )
}

/// The note shown once a restore has been started in the background.
pub fn restore_started(target: &str) -> String {
    fl!("settings-restore-started", folder = target)
}

/// A failure code from `backup::add` / `remove` / `list_remote`, for the window's error line.
#[must_use]
pub fn error(code: &str) -> String {
    backup::error_label(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rclone_ui_core::backup::FolderStatus;
    use rclone_ui_core::config::BackupFolder;

    fn cfg() -> Config {
        Config {
            backup_folders: vec![
                BackupFolder {
                    source: "/home/u/Documents".into(),
                    destination: "gdrive:Computers/nixie/Documents".into(),
                },
                BackupFolder {
                    source: "/home/u/Photos".into(),
                    destination: "gdrive:Computers/nixie/Photos".into(),
                },
            ],
            ..Config::default()
        }
    }

    #[test]
    fn rows_show_running_done_error_and_pending() {
        rclone_ui_i18n::init();
        let mut st = BackupStatus {
            running: true,
            current: Some("/home/u/Documents".into()),
            ..BackupStatus::default()
        };
        st.folders.insert(
            "/home/u/Documents".into(),
            FolderStatus {
                last_success: Some(100),
                error: None,
            },
        );
        let r = rows(&cfg(), &st, 400);
        assert_eq!(r[0].destination, "Computers/nixie/Documents");
        assert_eq!(r[0].status, fl!("backup-status-running"));
        assert_eq!(r[1].status, fl!("backup-status-pending"));

        st.running = false;
        st.folders.insert(
            "/home/u/Photos".into(),
            FolderStatus {
                last_success: Some(100),
                error: Some("source-empty".into()),
            },
        );
        let r = rows(&cfg(), &st, 400);
        assert_eq!(r[0].status, fl!("backup-status-done", when = ago(400, 100)));
        assert!(!r[0].error);
        assert_eq!(r[1].status, backup::error_label("source-empty"));
        assert!(r[1].error);
    }

    #[test]
    fn restore_rows_are_sorted_and_keep_the_full_destination() {
        rclone_ui_i18n::init();
        let list = [
            RemoteBackup {
                host: "work".into(),
                name: "src".into(),
                destination: "gdrive:Computers/work/src".into(),
            },
            RemoteBackup {
                host: "nixie".into(),
                name: "Documents".into(),
                destination: "gdrive:Computers/nixie/Documents".into(),
            },
        ];
        let r = restore_rows(&list);
        assert_eq!(r[0].destination, "gdrive:Computers/nixie/Documents");
        assert!(r[0].label.contains("nixie") && r[0].label.contains("Documents"));
    }
}
