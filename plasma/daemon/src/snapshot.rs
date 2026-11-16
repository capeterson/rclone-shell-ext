// SPDX-License-Identifier: GPL-3.0-only
//! The `StatusJson` document of the D-Bus contract, shared by `--status --json` and the daemon.

use rclone_ui_core::config::Config;
use rclone_ui_core::history::with_changes;
use rclone_ui_core::mime_icon;
use rclone_ui_core::pin::{OfflineState, Phase};
use rclone_ui_core::rc::{self, ChangeKind, CompletedTransfer, Direction, Health, Status};
use rclone_ui_core::status::fmt_time;
use serde::Serialize;

const FALLBACK_ICON: &str = "text-x-generic";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusJson {
    pub health: String,
    pub health_label: String,
    pub service_active: bool,
    pub mounted: bool,
    pub rc_error: Option<String>,
    pub uploads_in_progress: u64,
    pub uploads_queued: u64,
    pub cache_files: u64,
    pub cache_bytes: u64,
    pub errored_files: u64,
    pub out_of_space: bool,
    pub speed: f64,
    pub speed_text: String,
    pub last_error: Option<String>,
    pub rclone_version: Option<String>,
    pub outdated_rclone: Option<String>,
    pub transfers: Vec<TransferJson>,
    pub recent: Vec<RecentJson>,
    pub offline: OfflineJson,
    // --- migration (rclone_ui_core::migrate::pending) ---
    /// `none`, `pending`, or a failure code (see the D-Bus contract).
    pub migration: String,
    /// Finished text for `migration`; empty when it is `none`.
    pub migration_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferJson {
    pub name: String,
    pub size: i64,
    pub bytes: u64,
    pub percentage: u64,
    pub speed: f64,
    pub direction: Option<Direction>,
    pub mime_icon: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentJson {
    pub name: String,
    pub size: i64,
    pub at: Option<String>,
    pub at_text: Option<String>,
    pub error: Option<String>,
    pub direction: Option<Direction>,
    pub times: u32,
    pub kind: Option<ChangeKind>,
    pub mime_icon: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OfflineJson {
    pub enabled: bool,
    pub pins: Vec<PinJson>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinJson {
    pub rel: String,
    pub phase: String,
    pub phase_label: String,
    pub phase_icon: String,
    pub error: Option<String>,
}

/// The `Health` property value for `h`.
pub fn health_name(h: Health) -> &'static str {
    match h {
        Health::ServiceStopped => "ServiceStopped",
        Health::NotMounted => "NotMounted",
        Health::Unreachable => "Unreachable",
        Health::CacheFull => "CacheFull",
        Health::FileErrors => "FileErrors",
        Health::Uploading => "Uploading",
        Health::Downloading => "Downloading",
        Health::Transferring => "Transferring",
        Health::Healthy => "Healthy",
    }
}

fn headline(h: Health) -> &'static str {
    match h {
        Health::Healthy => "Healthy",
        Health::Uploading => "Uploading",
        Health::Downloading => "Downloading",
        Health::Transferring => "Uploading and downloading",
        Health::FileErrors => "Some files failed",
        Health::CacheFull => "Cache is full",
        Health::Unreachable => "Can't reach rclone",
        Health::NotMounted => "Not mounted",
        Health::ServiceStopped => "Service stopped",
    }
}

fn health_label(st: &Status) -> String {
    let h = st.health();
    let base = headline(h);
    let count = match h {
        Health::Uploading | Health::Downloading | Health::Transferring => {
            let n = if st.transfers.is_empty() {
                st.uploads_in_progress + st.uploads_queued
            } else {
                st.transfers.len() as u64
            };
            Some(n)
        }
        Health::FileErrors => Some(st.errored_files),
        _ => None,
    };
    match count {
        Some(n) if n > 0 => format!("{base}: {n} file{}", if n == 1 { "" } else { "s" }),
        _ => base.to_string(),
    }
}

fn mime_icon_for(name: &str) -> String {
    mime_icon::icon_names(name)
        .into_iter()
        .next()
        .unwrap_or_else(|| FALLBACK_ICON.to_string())
}

fn offline_json(cfg: &Config, state: &OfflineState) -> OfflineJson {
    let pins = cfg
        .pins
        .iter()
        .map(|rel| match state.pin(rel) {
            Some(p) => PinJson {
                rel: rel.clone(),
                phase: format!("{:?}", p.phase),
                phase_label: p.phase.label(),
                phase_icon: p.phase.icon().to_string(),
                error: p.error.as_ref().map(rclone_ui_core::pin::PinError::label),
            },
            None => PinJson {
                rel: rel.clone(),
                phase: format!("{:?}", Phase::Waiting),
                phase_label: "Waiting for the keeper".to_string(),
                phase_icon: Phase::Waiting.icon().to_string(),
                error: None,
            },
        })
        .collect();
    OfflineJson {
        enabled: cfg.offline_enabled,
        pins,
    }
}

/// Build the document. `recent` is the (already debounced) history to show.
pub fn build(
    st: &Status,
    recent: &[CompletedTransfer],
    offline: &OfflineState,
    cfg: &Config,
) -> StatusJson {
    let speed = st.display_speed();
    StatusJson {
        health: health_name(st.health()).to_string(),
        health_label: health_label(st),
        service_active: st.service_active,
        mounted: st.mounted,
        rc_error: st.rc_error.clone(),
        uploads_in_progress: st.uploads_in_progress,
        uploads_queued: st.uploads_queued,
        cache_files: st.cache_files,
        cache_bytes: st.cache_bytes,
        errored_files: st.errored_files,
        out_of_space: st.out_of_space,
        speed,
        speed_text: format!("{}/s", rc::human(speed)),
        last_error: st.last_error.clone(),
        rclone_version: st.rclone_version.clone(),
        outdated_rclone: st.outdated_rclone().map(str::to_string),
        transfers: st
            .transfers
            .iter()
            .map(|t| TransferJson {
                name: t.name.clone(),
                size: t.size,
                bytes: t.bytes,
                percentage: t.percentage,
                speed: t.speed,
                direction: t.direction(),
                mime_icon: mime_icon_for(&t.name),
            })
            .collect(),
        recent: recent
            .iter()
            .map(|r| RecentJson {
                name: r.name.clone(),
                size: r.size,
                at: r.at.map(|t| t.to_string()),
                at_text: r.at.map(fmt_time),
                error: r.error.clone(),
                direction: r.direction,
                times: r.times,
                kind: r.kind,
                mime_icon: mime_icon_for(&r.name),
            })
            .collect(),
        offline: offline_json(cfg, offline),
        migration: rclone_ui_core::migrate::pending().code().into(),
        migration_label: rclone_ui_core::migrate::pending().label(),
    }
}

/// The document before the first poll has finished.
pub fn loading() -> StatusJson {
    StatusJson {
        health: "Loading".into(),
        health_label: "Checking…".into(),
        service_active: false,
        mounted: false,
        rc_error: None,
        uploads_in_progress: 0,
        uploads_queued: 0,
        cache_files: 0,
        cache_bytes: 0,
        errored_files: 0,
        out_of_space: false,
        speed: 0.0,
        speed_text: format!("{}/s", rc::human(0.0)),
        last_error: None,
        rclone_version: None,
        outdated_rclone: None,
        transfers: Vec::new(),
        recent: Vec::new(),
        offline: OfflineJson {
            enabled: false,
            pins: Vec::new(),
        },
        migration: "none".into(),
        migration_label: String::new(),
    }
}

/// One full poll: rclone's status (with keeper changes mixed into `recent`) and the offline state.
pub fn poll(cfg: &Config) -> (Status, OfflineState) {
    let mut st = rc::fetch(cfg);
    let offline = if cfg.offline_enabled {
        OfflineState::load()
    } else {
        OfflineState::default()
    };
    st.recent = with_changes(std::mem::take(&mut st.recent), &offline.changes);
    (st, offline)
}

/// `--status --json`: a one-shot document with the history shown as-is.
pub fn one_shot(cfg: &Config) -> StatusJson {
    let (st, offline) = poll(cfg);
    build(&st, &st.recent, &offline, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rclone_ui_core::pin::PinState;
    use rclone_ui_core::rc::Transfer;
    use serde_json::{json, Value};

    fn status() -> Status {
        Status {
            service_active: true,
            mounted: true,
            rc_error: None,
            uploads_in_progress: 1,
            uploads_queued: 2,
            cache_files: 120,
            cache_bytes: 1000,
            errored_files: 0,
            out_of_space: false,
            speed: 1_048_576.0,
            last_error: None,
            transfers: vec![Transfer {
                name: "Docs/a.pdf".into(),
                size: 1000,
                bytes: 500,
                percentage: 50,
                speed: 1024.0,
                src_fs: Some("/cache".into()),
                dst_fs: Some("gdrive:".into()),
            }],
            recent: vec![],
            rclone_version: Some("v1.75.2".into()),
        }
    }

    #[test]
    fn json_shape() {
        let recent = vec![CompletedTransfer {
            name: "Docs/b.txt".into(),
            size: 10,
            bytes: 10,
            at: Some("2026-11-14T12:00:00Z".parse().unwrap()),
            error: None,
            direction: Some(Direction::Download),
            times: 2,
            kind: Some(ChangeKind::Updated),
        }];
        let mut cfg = Config {
            offline_enabled: true,
            pins: vec!["Projects".into(), "New".into()],
            ..Config::default()
        };
        cfg.pins.truncate(2);
        let mut off = OfflineState::default();
        off.pins.push(PinState {
            rel: "Projects".into(),
            phase: Phase::Ready,
            ..PinState::default()
        });
        let v: Value = serde_json::to_value(build(&status(), &recent, &off, &cfg)).unwrap();
        assert_eq!(v["health"], "Uploading");
        assert_eq!(v["healthLabel"], "Uploading: 1 file");
        assert_eq!(v["speedText"], "1.0 MiB/s");
        assert_eq!(v["uploadsQueued"], 2);
        assert_eq!(v["rcError"], Value::Null);
        assert_eq!(v["outdatedRclone"], Value::Null);
        assert_eq!(v["transfers"][0]["direction"], "Upload");
        assert_eq!(v["transfers"][0]["mimeIcon"], "application-pdf");
        assert_eq!(v["recent"][0]["direction"], "Download");
        assert_eq!(v["recent"][0]["kind"], "Updated");
        assert_eq!(v["recent"][0]["at"], "2026-11-14T12:00:00Z");
        assert!(v["recent"][0]["atText"].is_string());
        assert_eq!(v["offline"]["enabled"], true);
        assert_eq!(v["offline"]["pins"][0]["phase"], "Ready");
        assert_eq!(v["offline"]["pins"][0]["phaseLabel"], "Available offline");
        assert_eq!(v["offline"]["pins"][1]["phase"], "Waiting");
        for k in [
            "serviceActive",
            "cacheFiles",
            "cacheBytes",
            "erroredFiles",
            "outOfSpace",
            "lastError",
            "rcloneVersion",
        ] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        assert_eq!(v["rcloneVersion"], json!("v1.75.2"));
    }

    #[test]
    fn labels_and_loading() {
        let mut st = status();
        st.service_active = false;
        assert_eq!(
            build(&st, &[], &OfflineState::default(), &Config::default()).health,
            "ServiceStopped"
        );
        assert_eq!(health_label(&st), "Service stopped");
        let l = serde_json::to_value(loading()).unwrap();
        assert_eq!(l["health"], "Loading");
        assert_eq!(l["offline"]["pins"], json!([]));
    }
}
