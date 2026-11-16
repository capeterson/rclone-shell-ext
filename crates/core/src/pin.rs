// SPDX-License-Identifier: GPL-3.0-only
//! Offline folders ("pins"): where their pieces live, what is allowed to be pinned, the state the
//! keeper shares with the UI, and the plan the keeper follows each cycle.
//!
//! A pinned folder `Work/Taxes` has:
//! - a local copy at `<data>/offline/<id>`, kept in two-way sync with the remote by `rclone
//!   bisync` (run inside the main mount's rclone process through its rc API);
//! - a mount of that copy on top of `<mount>/Work/Taxes`, so it shows up in place.
//!
//! `<id>` is a short hash of the path. bisync names its lock and listing files after both full
//! paths and (still, as of rclone 1.75) can't cope when that name is longer than a file name may
//! be, so the local side is kept short and the cloud side is addressed by its Drive id.

use crate::config::{expand, Config};
use crate::rc::{ChangeKind, Direction};
use crate::share;
use rclone_ui_i18n::fl;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// bisync's session name (both paths, `/` turned into `_`) must fit in a file name, along with
/// suffixes like `.path1.lst-new`.
const SESSION_NAME_MAX: usize = 230;
/// After a local change, wait this long for more before syncing.
pub const LOCAL_DEBOUNCE_SECS: i64 = 10;
/// First retry delay after a failed sync; doubles per failure, capped at the sync interval.
const RETRY_BASE_SECS: i64 = 30;

/// The folder name used for this build's data and state (the dev build keeps its own).
pub fn app_dir_name() -> &'static str {
    crate::identity::identity().slug
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(expand(fallback)), PathBuf::from)
        .join(app_dir_name())
}

/// `$XDG_DATA_HOME/<slug>`: the local copies live under here.
pub fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", "~/.local/share")
}

/// `$XDG_STATE_HOME/<slug>`: bisync's working files and `offline.json`.
pub fn state_dir() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", "~/.local/state")
}

pub fn state_file() -> PathBuf {
    state_dir().join("offline.json")
}

pub fn workdir() -> PathBuf {
    state_dir().join("bisync")
}

/// A stable 16-hex-digit name for a pinned path (FNV-1a, which unlike `DefaultHasher` never
/// changes between Rust releases).
pub fn pin_id(rel: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rel.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Where the local copy of `rel` lives.
pub fn local_dir(rel: &str) -> PathBuf {
    data_dir().join("offline").join(pin_id(rel))
}

/// Where `rel` shows up inside the mount.
pub fn mount_point(cfg: &Config, rel: &str) -> PathBuf {
    Path::new(&expand(&cfg.mount_path)).join(rel)
}

/// The remote side of `rel` for bisync, given the mounted remote `fs` (e.g. `gdrive:` or
/// `gdrive:sub`). Native Google Docs are skipped: rclone can only download them as exported
/// copies, which can't be synced back.
pub fn remote(fs: &str, rel: &str) -> String {
    let (name, root) = fs.split_once(':').unwrap_or((fs, ""));
    let path = [root.trim_matches('/'), rel]
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    format!("{name},skip_gdocs=true:{path}")
}

/// Like [`remote`], but addressing the folder by its Drive id (`root_folder_id`). bisync names its
/// working files after both paths; with the id the cloud side contributes only the remote's name,
/// so a deep cloud path no longer overflows a file name.
pub fn remote_by_id(fs: &str, id: &str) -> String {
    let name = fs.split_once(':').map_or(fs, |(n, _)| n);
    format!("{name},skip_gdocs=true,root_folder_id={id}:")
}

/// The pinned folder that `rel` is, or is inside of.
pub fn covering_pin<'a>(pins: &'a [String], rel: &str) -> Option<&'a str> {
    pins.iter()
        .map(String::as_str)
        .find(|p| rel == *p || rel.starts_with(&format!("{p}/")))
}

/// Check that `path` (an absolute path) can be pinned and return it relative to the mount.
pub fn normalize(
    mount: &str,
    path: &Path,
    is_dir: bool,
    pins: &[String],
    fs: &str,
) -> Result<String, String> {
    let rel =
        share::remote_path(&expand(mount), path).ok_or_else(|| fl!("offline-err-outside-drive"))?;
    if rel.is_empty() {
        return Err(fl!("offline-err-whole-drive"));
    }
    if !is_dir {
        return Err(fl!("offline-err-only-folders"));
    }
    if let Some(p) = covering_pin(pins, &rel) {
        return Err(if p == rel {
            fl!("offline-err-already")
        } else {
            fl!("offline-err-already-part", parent = p)
        });
    }
    if let Some(p) = pins.iter().find(|p| p.starts_with(&format!("{rel}/"))) {
        return Err(fl!("offline-err-contains", pin = p.as_str()));
    }
    // Folders are synced by Drive id, so only the remote's name and the local path count. (If the
    // id can't be looked up the keeper falls back to the path and checks again.)
    let session = session_name(&remote_by_id(fs, ""), &local_dir(&rel).to_string_lossy());
    if session.len() > SESSION_NAME_MAX {
        return Err(fl!("offline-err-path-too-long"));
    }
    Ok(rel)
}

/// Does bisync's working-file name for these two paths fit in a file name?
pub fn session_fits(path1: &str, path2: &str) -> bool {
    session_name(path1, path2).len() <= SESSION_NAME_MAX
}

/// Roughly how bisync names its working files: both paths with separators flattened.
fn session_name(path1: &str, path2: &str) -> String {
    let flat = |p: &str| p.trim_start_matches('/').replace(['/', ':'], "_");
    format!("{}..{}", flat(path1), flat(path2))
}

// ---- state shared between the keeper and the UI (`offline.json`) ----

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// First download in progress; the folder isn't mounted in place yet.
    #[default]
    Downloading,
    /// Synced at `last_sync`.
    Ready,
    /// Last sync failed for a reason that may fix itself (offline, mount down); retrying.
    Waiting,
    /// Stopped until someone looks: bisync refused for safety, or the folder is gone remotely.
    Attention,
    /// Unpinned; pushing last changes and cleaning up.
    Removing,
}

impl Phase {
    pub fn label(self) -> String {
        match self {
            Phase::Downloading => fl!("offline-phase-downloading"),
            Phase::Ready => fl!("offline-phase-ready"),
            Phase::Waiting => fl!("offline-phase-waiting"),
            Phase::Attention => fl!("offline-phase-attention"),
            Phase::Removing => fl!("offline-phase-removing"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Phase::Downloading => "folder-download-symbolic",
            Phase::Ready => "emblem-ok-symbolic",
            Phase::Waiting => "network-offline-symbolic",
            Phase::Attention => "dialog-warning-symbolic",
            Phase::Removing => "edit-delete-symbolic",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PinState {
    pub rel: String,
    pub phase: Phase,
    /// The first full sync has completed, so bisync has listings to compare against.
    pub initialized: bool,
    /// Unix seconds of the last successful sync.
    pub last_sync: Option<i64>,
    /// Unix seconds of the last attempt, successful or not.
    pub last_attempt: i64,
    /// Failed attempts in a row.
    pub failures: u32,
    #[serde(deserialize_with = "de_pin_error")]
    pub error: Option<PinError>,
    /// A sync is running right now.
    pub syncing: bool,
    /// The Google Drive id of the cloud folder this copy's bisync session is keyed on. Set after
    /// the first successful sync by id; a different id for the same path means the cloud folder
    /// was deleted and recreated, which must not be synced against the old baseline.
    pub folder_id: Option<String>,
}

/// How much of a folder's local copy is on disk: (files, bytes).
pub fn local_usage(rel: &str) -> (u64, u64) {
    fn walk(dir: &Path, acc: &mut (u64, u64)) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                walk(&e.path(), acc);
            } else {
                acc.0 += 1;
                acc.1 += md.len();
            }
        }
    }
    let mut acc = (0, 0);
    walk(&local_dir(rel), &mut acc);
    acc
}

/// One line describing a pinned folder's state for the UI, e.g. "Downloading for offline use ·
/// 12 files (3.0 MiB) so far" or "Syncing…". `usage` is from [`local_usage`], when measured.
pub fn describe(p: Option<&PinState>, usage: Option<(u64, u64)>) -> String {
    let Some(p) = p else {
        return fl!("offline-describe-waiting-to-start");
    };
    let files = |(n, b): (u64, u64)| {
        fl!(
            "offline-files-size",
            count = n,
            size = crate::status::human_bytes(b)
        )
    };
    let mut line = match (p.phase, p.syncing) {
        (Phase::Downloading, _) => match usage {
            Some(u) if u.0 > 0 => fl!(
                "offline-describe-so-far",
                phase = p.phase.label(),
                files = files(u)
            ),
            _ => p.phase.label(),
        },
        (Phase::Ready | Phase::Waiting, true) => fl!("offline-syncing"),
        (Phase::Ready, false) => match usage {
            Some(u) => fl!(
                "offline-describe-phase-files",
                phase = p.phase.label(),
                files = files(u)
            ),
            None => p.phase.label(),
        },
        (phase, _) => phase.label(),
    };
    // While a sync runs, the error is the last attempt's and may no longer apply.
    if let Some(e) = p.error.as_ref().filter(|_| !p.syncing) {
        line = fl!("offline-describe-error", line = line, error = e.label());
    }
    line
}

/// Something the user should hear about once, e.g. a resolved conflict.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notice {
    pub at: i64,
    pub rel: String,
    #[serde(deserialize_with = "de_notice_text")]
    pub text: NoticeText,
}

/// What a [`Notice`] says, stored as a code plus its facts so the wording is chosen in the
/// language of whoever reads it, not whoever wrote it (the keeper may run under another locale).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeText {
    /// A sync conflict on `path` (relative to the mount root) was resolved.
    Conflict {
        path: String,
        winner: crate::conflict::Winner,
    },
    /// Text that is not ours to translate, or written before notices were coded.
    Other(String),
}

impl Default for NoticeText {
    fn default() -> Self {
        NoticeText::Other(String::new())
    }
}

impl NoticeText {
    /// The wording, in the current language.
    pub fn label(&self) -> String {
        match self {
            NoticeText::Conflict { path, winner } => crate::conflict::notice_label(path, *winner),
            NoticeText::Other(s) => s.clone(),
        }
    }
}

/// Builds a [`PinError`] that carries raw text.
type MakeError = fn(String) -> PinError;

/// Why a folder shows an error. Stored as a code (plus the raw rclone/OS text where there is
/// one), never as localized wording; [`PinError::label`] words it when it is displayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinError {
    /// The folder is gone from the Drive mount.
    FolderMissing,
    /// The cloud folder was deleted and recreated.
    FolderReplaced,
    /// One side is empty and syncing would empty the other.
    OneSideEmpty,
    /// The folder is gone from the cloud.
    CloudFolderMissing,
    /// bisync stopped for safety.
    SyncStopped,
    /// Couldn't mount the offline copy over its place.
    MountInPlace { error: String },
    /// Removing: no connection to upload the last changes.
    WaitingToUpload,
    /// Removing: uploading the last changes failed.
    UploadLast { error: String },
    /// Removing: files in the folder are open.
    FilesOpen,
    /// Removing: deleting the local copy failed.
    DeleteLocal { error: String },
    /// No network.
    NoNetwork,
    /// rclone accepted `method` but didn't return a job id (`method` is rclone's own name).
    NoJobId { method: String },
    /// Lost contact with rclone while it ran `method`; `error` is the transport's own text.
    LostContact { method: String, error: String },
    /// A sync started earlier is still running in rclone.
    EarlierSync,
    /// Couldn't look up the cloud folder's Drive id; `error` is rclone's own text.
    CloudLookup { error: String },
    /// The folder's path is too long for bisync's working-file names.
    PathTooLong,
    /// The Drive mount isn't running.
    MountDown,
    /// Removing: uploading the last changes failed for a reason that is itself coded.
    UploadLastFailed { cause: Box<PinError> },
    /// rclone's own text, or text written before errors were coded.
    Other(String),
}

impl From<String> for PinError {
    fn from(s: String) -> Self {
        PinError::Other(s)
    }
}

impl PinError {
    /// Whether this is rclone's/the OS's own text and it contains `needle`. Errors the keeper
    /// coded itself never match: there is no text of ours to search.
    pub fn raw_contains(&self, needle: &str) -> bool {
        matches!(self, PinError::Other(s) if s.contains(needle))
    }

    /// The wording, in the current language.
    pub fn label(&self) -> String {
        match self {
            PinError::FolderMissing => fl!("offline-err-mount-folder-missing"),
            PinError::FolderReplaced => fl!("offline-err-folder-replaced"),
            PinError::OneSideEmpty => fl!("offline-err-one-side-empty"),
            PinError::CloudFolderMissing => fl!("offline-err-cloud-folder-missing"),
            PinError::SyncStopped => fl!("offline-err-sync-stopped"),
            PinError::MountInPlace { error } => {
                fl!("offline-err-mount-in-place", error = error.as_str())
            }
            PinError::WaitingToUpload => fl!("offline-err-waiting-to-upload"),
            PinError::UploadLast { error } => {
                fl!("offline-err-upload-last", error = error.as_str())
            }
            PinError::FilesOpen => fl!("offline-err-files-open"),
            PinError::DeleteLocal { error } => {
                fl!("offline-err-delete-local", error = error.as_str())
            }
            PinError::NoNetwork => fl!("offline-err-no-network"),
            PinError::NoJobId { method } => {
                fl!("offline-err-no-job-id", method = method.as_str())
            }
            PinError::LostContact { method, error } => fl!(
                "offline-err-lost-contact",
                method = method.as_str(),
                error = error.as_str()
            ),
            PinError::EarlierSync => fl!("offline-err-earlier-sync"),
            PinError::CloudLookup { error } => {
                fl!("offline-err-cloud-lookup", error = error.as_str())
            }
            PinError::PathTooLong => fl!("offline-err-path-too-long"),
            PinError::MountDown => fl!("offline-err-mount-down"),
            PinError::UploadLastFailed { cause } => {
                fl!("offline-err-upload-last", error = cause.label())
            }
            PinError::Other(s) => s.clone(),
        }
    }

    /// Read an `offline.json` value from before errors were coded: the English sentence the old
    /// keeper wrote. Known sentences get their variant back; anything else stays raw text.
    fn from_legacy(s: String) -> Self {
        const EXACT: [(&str, PinError); 10] = [
            (
                "The folder wasn't found in the Drive mount (moved or deleted?)",
                PinError::FolderMissing,
            ),
            (
                "The folder in the cloud was deleted and recreated, so this offline copy no longer matches it. Repair merges both sides without deleting anything.",
                PinError::FolderReplaced,
            ),
            (
                "One side of the folder is empty, and syncing would empty the other side too. Repair brings the files back.",
                PinError::OneSideEmpty,
            ),
            (
                "The folder wasn't found in the cloud (moved or deleted?)",
                PinError::CloudFolderMissing,
            ),
            (
                "Sync stopped for safety (too many deletions, or an interrupted run). Repair merges both sides without deleting anything.",
                PinError::SyncStopped,
            ),
            (
                "Waiting for a connection to upload the last changes",
                PinError::WaitingToUpload,
            ),
            ("Files in the folder are still open", PinError::FilesOpen),
            (
                "An earlier sync is still finishing",
                PinError::EarlierSync,
            ),
            (
                "The folder's path is too long for rclone bisync",
                PinError::PathTooLong,
            ),
            ("The Drive mount is down", PinError::MountDown),
        ];
        if let Some((_, v)) = EXACT.iter().find(|(text, _)| *text == s) {
            return v.clone();
        }
        if s == "no network" {
            return PinError::NoNetwork;
        }
        if let Some(method) = s.strip_suffix(": rclone didn't return a job id") {
            return PinError::NoJobId {
                method: method.to_string(),
            };
        }
        if let Some((method, error)) = s
            .strip_prefix("lost contact with rclone during ")
            .and_then(|rest| rest.split_once(": "))
        {
            return PinError::LostContact {
                method: method.to_string(),
                error: error.to_string(),
            };
        }
        if let Some(rest) = s.strip_prefix("Couldn't upload the last changes: ") {
            // The reason may itself be one of our sentences.
            return match Self::from_legacy(rest.to_string()) {
                PinError::Other(error) => PinError::UploadLast { error },
                cause => PinError::UploadLastFailed {
                    cause: Box::new(cause),
                },
            };
        }
        let prefixed: [(&str, MakeError); 3] = [
            ("Couldn't look up the cloud folder: ", |error| {
                PinError::CloudLookup { error }
            }),
            ("Couldn't mount the offline copy in place: ", |error| {
                PinError::MountInPlace { error }
            }),
            ("Couldn't delete the local copy: ", |error| {
                PinError::DeleteLocal { error }
            }),
        ];
        for (prefix, make) in prefixed {
            if let Some(rest) = s.strip_prefix(prefix) {
                return make(rest.to_string());
            }
        }
        PinError::Other(s)
    }
}

/// Either the coded form or a bare string from an older `offline.json`.
#[derive(Deserialize)]
#[serde(untagged)]
enum PinErrorWire {
    Coded(PinError),
    Legacy(String),
}

fn de_pin_error<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<PinError>, D::Error> {
    Ok(Option::<PinErrorWire>::deserialize(d)?.map(|w| match w {
        PinErrorWire::Coded(e) => e,
        PinErrorWire::Legacy(s) => PinError::from_legacy(s),
    }))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NoticeTextWire {
    Coded(NoticeText),
    Legacy(String),
}

fn de_notice_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<NoticeText, D::Error> {
    Ok(match NoticeTextWire::deserialize(d)? {
        NoticeTextWire::Coded(t) => t,
        NoticeTextWire::Legacy(s) => NoticeText::Other(s),
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OfflineState {
    /// Unix seconds when the keeper last wrote this.
    pub updated: i64,
    pub pins: Vec<PinState>,
    /// Newest last; trimmed to [`NOTICES_KEPT`].
    pub notices: Vec<Notice>,
    /// What offline-folder syncs did, newest last; trimmed to [`ACTIVITY_KEPT`]. rclone's own
    /// history can't say whether a file was new, changed or deleted, or which way it went.
    pub changes: Vec<Change>,
}

pub const NOTICES_KEPT: usize = 20;
pub const ACTIVITY_KEPT: usize = 100;

/// One file a sync of an offline folder created, updated or deleted on the other side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Unix seconds when the sync applied it.
    pub at: i64,
    /// Path relative to the mount root, e.g. `Work/Taxes/2024.pdf`.
    pub path: String,
    /// Bytes, or `-1` for a deletion.
    pub size: i64,
    pub kind: ChangeKind,
    /// `Upload`: changed on this computer and sent to the cloud; `Download`: the reverse.
    pub direction: Direction,
}

impl OfflineState {
    pub fn record_change(&mut self, change: Change) {
        self.changes.push(change);
        let extra = self.changes.len().saturating_sub(ACTIVITY_KEPT);
        self.changes.drain(..extra);
    }

    pub fn pin(&self, rel: &str) -> Option<&PinState> {
        self.pins.iter().find(|p| p.rel == rel)
    }

    pub fn pin_mut(&mut self, rel: &str) -> Option<&mut PinState> {
        self.pins.iter_mut().find(|p| p.rel == rel)
    }

    pub fn notice(&mut self, at: i64, rel: &str, text: NoticeText) {
        self.notices.push(Notice {
            at,
            rel: rel.to_string(),
            text,
        });
        let extra = self.notices.len().saturating_sub(NOTICES_KEPT);
        self.notices.drain(..extra);
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn load() -> Self {
        Self::load_from(&state_file())
    }

    /// Write atomically (temp file + rename) so readers never see half a file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

/// One-line summary for the panel popup, e.g. "3 folders · all available offline".
pub fn summary(pins: &[String], state: &OfflineState) -> String {
    let folders = fl!("offline-folders-count", count = pins.len());
    let worst = pins
        .iter()
        .map(|r| state.pin(r).map_or(Phase::Downloading, |p| p.phase))
        .max_by_key(|p| match p {
            Phase::Ready => 0,
            Phase::Removing => 1,
            Phase::Downloading => 2,
            Phase::Waiting => 3,
            Phase::Attention => 4,
        });
    match worst {
        None => fl!("offline-summary-none"),
        Some(Phase::Ready) => fl!("offline-summary-all-ready", folders = folders),
        Some(p) => fl!(
            "offline-summary-phase",
            folders = folders,
            phase = match p {
                Phase::Downloading => "downloading",
                Phase::Waiting => "waiting",
                Phase::Attention => "attention",
                Phase::Removing => "removing",
                Phase::Ready => "ready",
            }
        ),
    }
}

// ---- the keeper's plan ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Create the local copy and do the first full download (bisync `--resync`).
    InitialSync(String),
    /// Mount the local copy in place.
    StartMount(String),
    /// A normal two-way sync.
    Sync(String),
    /// Re-establish bisync's baseline after it refused to continue (the user asked for it).
    Repair(String),
    /// Unpinned: push last changes, unmount, delete the local copy.
    Remove(String),
}

/// What the keeper knows at the start of a cycle besides config and state.
pub struct Facts<'a> {
    pub now: i64,
    pub interval: i64,
    /// The main mount and its rc API are up; nothing can be synced otherwise.
    pub online: bool,
    /// Pins whose in-place mount is active.
    pub mounted: &'a HashSet<String>,
    /// Pins whose local copy changed since their last attempt.
    pub changed: &'a HashSet<String>,
    /// Pins the user asked to repair.
    pub repair: &'a HashSet<String>,
}

fn retry_delay(failures: u32, interval: i64) -> i64 {
    if failures == 0 {
        interval
    } else {
        (RETRY_BASE_SECS << failures.min(6).saturating_sub(1)).min(interval)
    }
}

/// Decide what to do this cycle. Idempotent: running it again before acting yields the same
/// steps, so a crash between cycles loses nothing.
pub fn plan(wanted: &[String], state: &OfflineState, f: &Facts) -> Vec<Step> {
    let mut steps = vec![];
    for rel in wanted {
        let Some(p) = state.pin(rel) else {
            if f.online {
                steps.push(Step::InitialSync(rel.clone()));
            }
            continue;
        };
        // A local copy that has been fully downloaded is usable even with no network.
        if p.initialized && !f.mounted.contains(rel) {
            steps.push(Step::StartMount(rel.clone()));
        }
        if !f.online {
            continue;
        }
        if p.phase == Phase::Attention {
            if f.repair.contains(rel) {
                steps.push(Step::Repair(rel.clone()));
            }
            continue;
        }
        if !p.initialized {
            if f.now - p.last_attempt >= retry_delay(p.failures, f.interval) || p.failures == 0 {
                steps.push(Step::InitialSync(rel.clone()));
            }
            continue;
        }
        let since = f.now - p.last_attempt;
        let due = since >= retry_delay(p.failures, f.interval);
        let changed = f.changed.contains(rel) && since >= LOCAL_DEBOUNCE_SECS;
        if due || changed {
            steps.push(Step::Sync(rel.clone()));
        }
    }
    for p in &state.pins {
        if !wanted.contains(&p.rel) {
            steps.push(Step::Remove(p.rel.clone()));
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = "/home/u/Drive";

    fn norm(path: &str, is_dir: bool, pins: &[&str]) -> Result<String, String> {
        let pins: Vec<String> = pins.iter().map(|s| (*s).to_string()).collect();
        normalize(M, Path::new(path), is_dir, &pins, "gdrive:")
    }

    #[test]
    fn normalizes_folders_inside_the_mount() {
        assert_eq!(
            norm("/home/u/Drive/Work/Taxes", true, &[]).unwrap(),
            "Work/Taxes"
        );
        assert_eq!(
            norm("/home/u/Drive/Work/Taxes/", true, &[]).unwrap(),
            "Work/Taxes"
        );
        assert_eq!(
            norm("/home/u/Drive/My Stuff", true, &[]).unwrap(),
            "My Stuff"
        );
    }

    #[test]
    fn rejects_root_outside_files_and_climbing() {
        assert!(norm("/home/u/Drive", true, &[])
            .unwrap_err()
            .contains("whole Drive"));
        assert!(norm("/home/u/Other", true, &[])
            .unwrap_err()
            .contains("inside the Drive"));
        assert!(norm("/home/u/Drive/../x", true, &[]).is_err());
        assert!(norm("/home/u/Drive/a.txt", false, &[])
            .unwrap_err()
            .contains("folders"));
    }

    #[test]
    fn rejects_overlapping_pins() {
        assert_eq!(
            norm("/home/u/Drive/Work", true, &["Work"]).unwrap_err(),
            "Already kept offline"
        );
        assert!(norm("/home/u/Drive/Work/Taxes", true, &["Work"])
            .unwrap_err()
            .contains("part of Work"));
        assert!(norm("/home/u/Drive/Work", true, &["Work/Taxes"])
            .unwrap_err()
            .contains("Contains Work/Taxes"));
        // A shared prefix that isn't a parent is fine.
        assert_eq!(
            norm("/home/u/Drive/Workshop", true, &["Work"]).unwrap(),
            "Workshop"
        );
    }

    #[test]
    fn long_cloud_paths_are_fine_because_folders_sync_by_id() {
        let long = format!("/home/u/Drive/{}", "x".repeat(200));
        assert!(norm(&long, true, &[]).is_ok());
        // ...but addressed by path, the same folder would overflow bisync's file names.
        assert!(!session_fits(
            &remote("gdrive:", &"x".repeat(200)),
            &local_dir("x").to_string_lossy()
        ));
        assert!(session_fits(
            &remote_by_id("gdrive:", "1AbC"),
            &local_dir("x").to_string_lossy()
        ));
    }

    #[test]
    fn id_remote_keeps_the_options_and_drops_the_path() {
        assert_eq!(
            remote_by_id("gdrive:sub/", "F1"),
            "gdrive,skip_gdocs=true,root_folder_id=F1:"
        );
    }

    #[test]
    fn covering_pin_matches_self_and_children_only() {
        let pins = vec!["Work".to_string()];
        assert_eq!(covering_pin(&pins, "Work"), Some("Work"));
        assert_eq!(covering_pin(&pins, "Work/a/b"), Some("Work"));
        assert_eq!(covering_pin(&pins, "Workshop"), None);
    }

    #[test]
    fn pin_ids_are_stable_and_short() {
        assert_eq!(pin_id(""), "cbf29ce484222325");
        assert_eq!(pin_id("Work/Taxes").len(), 16);
        assert_ne!(pin_id("a"), pin_id("b"));
    }

    #[test]
    fn remote_skips_google_docs_and_keeps_the_mount_root() {
        assert_eq!(remote("gdrive:", "Work"), "gdrive,skip_gdocs=true:Work");
        assert_eq!(
            remote("gdrive:sub/", "Work"),
            "gdrive,skip_gdocs=true:sub/Work"
        );
    }

    #[test]
    fn state_round_trips_and_tolerates_garbage() {
        let dir = std::env::temp_dir().join(format!("rcs-pinstate-{}", std::process::id()));
        let f = dir.join("offline.json");
        let mut s = OfflineState::default();
        s.pins.push(PinState {
            rel: "Work".into(),
            phase: Phase::Ready,
            initialized: true,
            last_sync: Some(5),
            ..PinState::default()
        });
        s.notice(1, "Work", NoticeText::Other("hi".into()));
        s.save_to(&f).unwrap();
        assert_eq!(OfflineState::load_from(&f), s);
        std::fs::write(&f, "not json").unwrap();
        assert_eq!(OfflineState::load_from(&f), OfflineState::default());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn notices_are_capped() {
        let mut s = OfflineState::default();
        for i in 0..(NOTICES_KEPT + 5) {
            s.notice(
                i64::try_from(i).unwrap(),
                "x",
                NoticeText::Other(format!("{i}")),
            );
        }
        assert_eq!(s.notices.len(), NOTICES_KEPT);
        assert_eq!(s.notices[0].text, NoticeText::Other("5".into()));
    }

    #[test]
    fn describe_follows_the_keeper() {
        assert_eq!(describe(None, None), "Waiting to start…");
        let mut p = PinState {
            rel: "a".into(),
            ..PinState::default()
        };
        assert_eq!(describe(Some(&p), None), "Downloading for offline use");
        assert_eq!(
            describe(Some(&p), Some((12, 3 * 1024 * 1024))),
            "Downloading for offline use · 12 files (3.0 MiB) so far"
        );
        p.phase = Phase::Ready;
        p.syncing = true;
        assert_eq!(describe(Some(&p), Some((1, 5))), "Syncing…");
        p.syncing = false;
        assert_eq!(
            describe(Some(&p), Some((1, 5))),
            "Available offline · 1 file (5.0 B)"
        );
        p.phase = Phase::Waiting;
        p.error = Some(PinError::NoNetwork);
        assert_eq!(describe(Some(&p), None), "Waiting to sync: no network");
    }

    #[test]
    fn summary_reports_the_worst_state() {
        let pins = vec!["a".to_string(), "b".to_string()];
        let mut s = OfflineState::default();
        assert_eq!(summary(&[], &s), "No folders kept offline");
        for r in &pins {
            s.pins.push(PinState {
                rel: r.clone(),
                phase: Phase::Ready,
                ..PinState::default()
            });
        }
        assert_eq!(summary(&pins, &s), "2 folders · all available offline");
        s.pin_mut("b").unwrap().phase = Phase::Attention;
        assert_eq!(summary(&pins, &s), "2 folders · needs attention");
    }

    // ---- plan ----

    struct World {
        mounted: HashSet<String>,
        changed: HashSet<String>,
        repair: HashSet<String>,
    }

    impl World {
        fn new() -> Self {
            Self {
                mounted: HashSet::new(),
                changed: HashSet::new(),
                repair: HashSet::new(),
            }
        }
        fn facts(&self, now: i64, online: bool) -> Facts<'_> {
            Facts {
                now,
                interval: 300,
                online,
                mounted: &self.mounted,
                changed: &self.changed,
                repair: &self.repair,
            }
        }
    }

    fn ready(rel: &str, last: i64) -> PinState {
        PinState {
            rel: rel.into(),
            phase: Phase::Ready,
            initialized: true,
            last_sync: Some(last),
            last_attempt: last,
            ..PinState::default()
        }
    }

    fn state(pins: Vec<PinState>) -> OfflineState {
        OfflineState {
            pins,
            ..OfflineState::default()
        }
    }

    fn wanted(r: &[&str]) -> Vec<String> {
        r.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn new_pin_downloads_first_and_only_online() {
        let w = World::new();
        let s = OfflineState::default();
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(0, true)),
            [Step::InitialSync("a".into())]
        );
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(0, false)),
            Vec::<Step>::new()
        );
    }

    #[test]
    fn half_downloaded_pin_is_not_mounted() {
        let w = World::new();
        let s = state(vec![PinState {
            rel: "a".into(),
            last_attempt: 0,
            ..PinState::default()
        }]);
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(1, true)),
            [Step::InitialSync("a".into())]
        );
    }

    #[test]
    fn downloaded_pin_is_mounted_even_offline() {
        let w = World::new();
        let s = state(vec![ready("a", 0)]);
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(10, false)),
            [Step::StartMount("a".into())]
        );
    }

    #[test]
    fn steady_state_syncs_on_interval_or_local_change() {
        let mut w = World::new();
        w.mounted.insert("a".into());
        let s = state(vec![ready("a", 1000)]);
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(1100, true)),
            Vec::<Step>::new()
        );
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(1300, true)),
            [Step::Sync("a".into())]
        );
        w.changed.insert("a".into());
        assert!(
            plan(&wanted(&["a"]), &s, &w.facts(1005, true)).is_empty(),
            "debounced"
        );
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(1010, true)),
            [Step::Sync("a".into())]
        );
    }

    #[test]
    fn failures_retry_sooner_than_the_interval_with_backoff() {
        let mut w = World::new();
        w.mounted.insert("a".into());
        let mut p = ready("a", 1000);
        p.phase = Phase::Waiting;
        p.failures = 1;
        let s = state(vec![p.clone()]);
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(1029, true)),
            Vec::<Step>::new()
        );
        assert_eq!(plan(&wanted(&["a"]), &s, &w.facts(1030, true)).len(), 1);
        p.failures = 3; // 120s
        let s = state(vec![p]);
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(1100, true)),
            Vec::<Step>::new()
        );
        assert_eq!(plan(&wanted(&["a"]), &s, &w.facts(1120, true)).len(), 1);
    }

    #[test]
    fn attention_waits_for_a_repair_request() {
        let mut w = World::new();
        w.mounted.insert("a".into());
        let mut p = ready("a", 0);
        p.phase = Phase::Attention;
        let s = state(vec![p]);
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(10_000, true)),
            Vec::<Step>::new()
        );
        w.repair.insert("a".into());
        assert_eq!(
            plan(&wanted(&["a"]), &s, &w.facts(10_000, true)),
            [Step::Repair("a".into())]
        );
    }

    #[test]
    fn unpinned_folders_are_removed_even_offline() {
        // Removal itself checks it can push changes first; planning it is always right.
        let w = World::new();
        let s = state(vec![ready("gone", 0)]);
        assert_eq!(
            plan(&[], &s, &w.facts(1, false)),
            [Step::Remove("gone".into())]
        );
    }

    #[test]
    fn an_old_offline_json_with_english_errors_still_loads() {
        let old = r#"{
            "updated": 5,
            "pins": [
                {"rel": "A", "phase": "Attention",
                 "error": "The folder wasn't found in the Drive mount (moved or deleted?)"},
                {"rel": "B", "phase": "Removing", "error": "Couldn't upload the last changes: boom"},
                {"rel": "C", "phase": "Waiting", "error": "no network"},
                {"rel": "D", "phase": "Waiting", "error": "couldn't resolve host"},
                {"rel": "E", "phase": "Ready"}
            ],
            "notices": [{"at": 1, "rel": "A", "text": "A/x was changed in two places"}]
        }"#;
        let s: OfflineState = serde_json::from_str(old).unwrap();
        let err = |rel: &str| s.pin(rel).unwrap().error.clone();
        assert_eq!(err("A"), Some(PinError::FolderMissing));
        assert_eq!(
            err("B"),
            Some(PinError::UploadLast {
                error: "boom".into()
            })
        );
        assert_eq!(err("C"), Some(PinError::NoNetwork));
        assert_eq!(
            err("D"),
            Some(PinError::Other("couldn't resolve host".into()))
        );
        assert_eq!(err("E"), None);
        assert_eq!(
            s.notices[0].text,
            NoticeText::Other("A/x was changed in two places".into())
        );
        // Displayed wording is unchanged.
        assert_eq!(
            PinError::FolderMissing.label(),
            "The folder wasn't found in the Drive mount (moved or deleted?)"
        );
    }

    #[test]
    fn errors_and_notices_are_stored_as_codes() {
        let mut s = OfflineState::default();
        s.pins.push(PinState {
            rel: "A".into(),
            error: Some(PinError::MountInPlace { error: "x".into() }),
            ..PinState::default()
        });
        s.notice(
            1,
            "A",
            NoticeText::Conflict {
                path: "A/f".into(),
                winner: crate::conflict::Winner::Local,
            },
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("mount_in_place"), "{json}");
        assert!(!json.contains("Couldn't"), "{json}");
        assert!(!json.contains("two places"), "{json}");
        let back: OfflineState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn sync_errors_round_trip_as_codes_and_word_as_before() {
        let cases = [
            (
                PinError::NoJobId {
                    method: "sync/bisync".into(),
                },
                "sync/bisync: rclone didn't return a job id",
            ),
            (
                PinError::LostContact {
                    method: "sync/bisync".into(),
                    error: "timed out".into(),
                },
                "lost contact with rclone during sync/bisync: timed out",
            ),
            (PinError::EarlierSync, "An earlier sync is still finishing"),
            (
                PinError::CloudLookup {
                    error: "boom".into(),
                },
                "Couldn't look up the cloud folder: boom",
            ),
            (
                PinError::PathTooLong,
                "The folder's path is too long for rclone bisync",
            ),
            (PinError::MountDown, "The Drive mount is down"),
            (
                PinError::UploadLastFailed {
                    cause: Box::new(PinError::MountDown),
                },
                "Couldn't upload the last changes: The Drive mount is down",
            ),
        ];
        for (e, english) in cases {
            let json = serde_json::to_string(&e).unwrap();
            for word in [
                "Couldn't",
                "lost contact",
                "earlier sync",
                "too long",
                "is down",
            ] {
                assert!(!json.contains(word), "{json}");
            }
            assert_eq!(serde_json::from_str::<PinError>(&json).unwrap(), e);
            assert_eq!(e.label(), english);
            // The English an older keeper stored maps back to the same code.
            assert_eq!(PinError::from_legacy(english.to_string()), e);
        }
    }

    #[test]
    fn summary_names_the_worst_phase() {
        let pins = vec!["A".to_string()];
        let mut st = OfflineState::default();
        st.pins.push(PinState {
            rel: "A".into(),
            phase: Phase::Attention,
            ..PinState::default()
        });
        assert_eq!(summary(&pins, &st), "1 folder · needs attention");
    }
}
