// SPDX-License-Identifier: GPL-3.0-only
//! What the details window shows for one file or folder, and which buttons apply. A port of the
//! COSMIC window's view logic with the widgets taken out: [`DetailsState`] holds what has
//! arrived, [`DetailsState::view`] turns it into plain strings for the QML side.

use crate::Load;
use rclone_ui_core::config::{self, expand, Config};
use rclone_ui_core::drive::{self, DriveInfo, Kind, Permission};
use rclone_ui_core::mime_icon;
use rclone_ui_core::pin::{self, OfflineState, Phase};
use rclone_ui_core::share::{self, Target};
use rclone_ui_core::status::{self, human_bytes, Local, SyncState};
use rclone_ui_i18n::fl;
use serde_json::json;
use std::path::Path;

pub const PLACEHOLDER: &str = "…";
pub const NONE: &str = "—";

/// The offline-folder state as of one refresh. `cfg` is `None` if the refresh failed.
#[derive(Debug, Clone, Default)]
pub struct OfflineSnapshot {
    pub cfg: Option<Config>,
    pub state: OfflineState,
    /// Files and bytes in the local copy of the folder at `path` (or the one it's inside).
    pub usage: Option<(u64, u64)>,
}

/// Re-read the settings and the keeper's report, and measure the folder's local copy while it
/// is downloading or synced (it's cheap: a walk of local metadata).
pub fn offline_snapshot(path: &str) -> OfflineSnapshot {
    let cfg = config::store().load();
    let state = OfflineState::load();
    let usage = share::remote_path(&expand(&cfg.mount_path), Path::new(path))
        .and_then(|rel| pin::covering_pin(&cfg.pins, &rel).map(str::to_string))
        .filter(|p| {
            state
                .pin(p)
                .is_some_and(|s| matches!(s.phase, Phase::Downloading | Phase::Ready))
        })
        .map(|p| pin::local_usage(&p));
    OfflineSnapshot {
        cfg: Some(cfg),
        state,
        usage,
    }
}

/// How the "Open" button behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenButton {
    /// A folder opens directly.
    Direct,
    /// A file offers a menu with "Folder" and "File".
    Menu,
}

/// The offline button in the footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineButton {
    Hidden,
    /// "Keep offline"
    Keep,
    /// "Stop keeping offline"
    Stop,
}

/// The offline section: a title and caption, and the button that applies (if any).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineRow {
    pub title: String,
    pub caption: String,
    pub button: OfflineButton,
}

/// Which footer buttons show, and their state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buttons {
    pub open: OpenButton,
    pub manage_label: String,
    pub manage_enabled: bool,
    pub offline: OfflineButton,
}

/// Everything the QML side binds to, as plain values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // one per bound property
pub struct DetailsView {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub icon_name: String,
    pub error: String,
    pub sync_label: String,
    pub sync_icon: String,
    pub sync_detail: String,
    pub drive_modified: String,
    pub cache_state: String,
    pub size: String,
    pub version: String,
    pub uploading: String,
    pub downloading: String,
    pub cached: String,
    pub cached_label: String,
    pub shared: bool,
    pub sharing_summary: String,
    pub sharing_note: String,
    pub sharing_note_icon: String,
    pub permissions_json: String,
    pub manage_label: String,
    pub manage_enabled: bool,
    pub open_is_menu: bool,
    pub offline_enabled: bool,
    pub offline_title: String,
    pub offline_caption: String,
    /// "none", "keep" or "stop".
    pub offline_action: String,
    pub pinned_phase_label: String,
    pub offline_error: String,
}

#[derive(Debug, Clone)]
pub struct DetailsState {
    pub cfg: Config,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub target: Load<Target>,
    pub local: Load<Local>,
    pub drive: Load<DriveInfo>,
    pub offline: OfflineState,
    pub offline_error: Option<String>,
    pub offline_usage: Option<(u64, u64)>,
}

impl DetailsState {
    pub fn new(cfg: Config, paths: &[String]) -> Self {
        let first = paths.first().cloned().unwrap_or_default();
        let name = Path::new(&first)
            .file_name()
            .map_or_else(|| first.clone(), |n| n.to_string_lossy().into_owned());
        let is_dir = Path::new(&first).is_dir();
        let offline = if cfg.offline_enabled {
            OfflineState::load()
        } else {
            OfflineState::default()
        };
        DetailsState {
            cfg,
            name,
            path: first,
            is_dir,
            target: Load::Pending,
            local: Load::Pending,
            drive: Load::Pending,
            offline,
            offline_error: None,
            offline_usage: None,
        }
    }

    /// Every stage has answered (successfully or not).
    pub fn all_landed(&self) -> bool {
        !(self.target.waiting() || self.local.waiting() || self.drive.waiting())
    }

    /// The first stage's answer. A failure fills the later stages with the same reason so the
    /// loading screen can end.
    pub fn apply_target(&mut self, r: Result<Target, String>) {
        if let Err(e) = &r {
            self.local = Load::Done(Err(e.clone()));
            self.drive = Load::Done(Err(e.clone()));
        }
        self.target = Load::Done(r);
    }

    pub fn apply_drive(&mut self, r: Result<DriveInfo, String>) {
        self.drive = Load::Done(r);
    }

    /// A refresh that fails keeps showing the last good answer.
    pub fn apply_local(&mut self, r: Result<Local, String>) {
        if r.is_ok() || self.local.ok().is_none() {
            self.local = Load::Done(r);
        }
    }

    pub fn apply_snapshot(&mut self, snap: OfflineSnapshot) {
        if let Some(cfg) = snap.cfg {
            self.cfg = cfg;
            self.offline = snap.state;
            self.offline_usage = snap.usage;
        }
    }

    fn remote_rel(&self) -> Option<String> {
        share::remote_path(&expand(&self.cfg.mount_path), Path::new(&self.path))
    }

    // --- sync status ---

    /// Icon, label and right-hand caption for the status line.
    pub fn status_line(&self) -> (&'static str, String, String) {
        const LOADING: &str = "content-loading-symbolic";
        if let Some(e) = self.target.err() {
            return ("dialog-warning-symbolic", e.to_string(), String::new());
        }
        if let Some(e) = self.local.err() {
            return (
                "dialog-error-symbolic",
                fl!("details-status-error"),
                e.to_string(),
            );
        }
        // An offline folder is downloaded and synced by the keeper (a bisync job that writes
        // straight into the local copy), which the mount's own transfer counts never see.
        if let Some(status) = self.offline_status() {
            return status;
        }
        let checking = (LOADING, fl!("details-status-checking"), String::new());
        let Some(local) = self.local.ok() else {
            return checking;
        };
        match local {
            Local::File {
                transferring,
                cached,
            } => match status::file_state(*transferring, cached.as_ref(), self.drive.ok()) {
                Some(s) => (s.icon(), s.label(), String::new()),
                None if self.drive.err().is_some() => (
                    "dialog-warning-symbolic",
                    fl!("details-status-cached-no-compare"),
                    String::new(),
                ),
                None => checking,
            },
            Local::Folder(f) => {
                if f.uploading + f.downloading > 0 {
                    let count = f.uploading + f.downloading;
                    let detail = if f.truncated {
                        fl!("details-files-pending-more", count = count)
                    } else {
                        fl!("details-files-pending", count = count)
                    };
                    (
                        SyncState::LocalNewer.icon(),
                        fl!("details-status-transferring"),
                        detail,
                    )
                } else {
                    (
                        SyncState::InSync.icon(),
                        fl!("details-status-nothing-pending"),
                        String::new(),
                    )
                }
            }
        }
    }

    /// The status line for an item inside an offline folder whose keeper is busy (or stuck).
    fn offline_status(&self) -> Option<(&'static str, String, String)> {
        if !self.cfg.offline_enabled {
            return None;
        }
        let rel = self.remote_rel()?;
        let p = self.offline.pin(pin::covering_pin(&self.cfg.pins, &rel)?)?;
        let so_far = || match self.offline_usage {
            Some((n, b)) if n > 0 => {
                fl!("details-offline-so-far", count = n, size = human_bytes(b))
            }
            _ => String::new(),
        };
        let loading = "content-loading-symbolic";
        match (p.phase, p.syncing) {
            (Phase::Downloading, _) => Some((
                SyncState::Downloading.icon(),
                Phase::Downloading.label(),
                so_far(),
            )),
            (Phase::Removing, _) => Some((loading, Phase::Removing.label(), String::new())),
            (Phase::Attention, _) => Some((
                "dialog-warning-symbolic",
                fl!("details-offline-needs-attention"),
                String::new(),
            )),
            (Phase::Ready | Phase::Waiting, true) => Some((
                SyncState::LocalNewer.icon(),
                fl!("details-offline-syncing"),
                String::new(),
            )),
            (Phase::Waiting, false) => {
                Some((loading, fl!("details-offline-waiting"), String::new()))
            }
            // A file inside an idle offline folder: not in the mount's cache, but here.
            (Phase::Ready, false) if !self.is_dir => Some((
                SyncState::InSync.icon(),
                Phase::Ready.label(),
                String::new(),
            )),
            (Phase::Ready, false) => None,
        }
    }

    /// "362 file(s), 563.1 MiB" for a folder that is itself kept offline.
    fn offline_copy(&self) -> Option<String> {
        if !self.cfg.offline_enabled {
            return None;
        }
        let rel = self.remote_rel()?;
        (pin::covering_pin(&self.cfg.pins, &rel)? == rel).then_some(())?;
        let (n, b) = self.offline_usage?;
        Some(fl!("details-files-size", count = n, size = human_bytes(b)))
    }

    /// The label/value pairs under the status line, with placeholders while loading: four for a
    /// file, three for a folder.
    pub fn detail_rows(&self) -> Vec<(String, String)> {
        let from_drive = |f: &dyn Fn(&DriveInfo) -> Option<String>| match &self.drive {
            Load::Pending => PLACEHOLDER.to_string(),
            Load::Done(Ok(d)) => f(d).unwrap_or_else(|| NONE.into()),
            Load::Done(Err(_)) => NONE.to_string(),
        };
        if self.is_dir {
            let from_local = |f: &dyn Fn(&status::FolderSummary) -> String| match self.local.ok() {
                Some(Local::Folder(s)) => f(s),
                _ if self.local.waiting() => PLACEHOLDER.to_string(),
                _ => NONE.to_string(),
            };
            return vec![
                (
                    fl!("details-row-uploading"),
                    from_local(&|s| s.uploading.to_string()),
                ),
                (
                    fl!("details-row-downloading"),
                    from_local(&|s| s.downloading.to_string()),
                ),
                // An offline folder's files live in its local copy, not in the mount's cache.
                match self.offline_copy() {
                    Some(copy) => (fl!("details-row-offline-copy"), copy),
                    None => (
                        fl!("details-row-cached"),
                        from_local(&|s| {
                            if s.truncated {
                                fl!(
                                    "details-files-size-more",
                                    count = s.cached_files,
                                    size = human_bytes(s.cached_bytes)
                                )
                            } else {
                                fl!(
                                    "details-files-size",
                                    count = s.cached_files,
                                    size = human_bytes(s.cached_bytes)
                                )
                            }
                        }),
                    ),
                },
            ];
        }
        let cached = match self.local.ok() {
            Some(Local::File { cached, .. }) => match cached {
                None => fl!("common-no"),
                Some(c) if c.have >= c.size => fl!("common-yes"),
                Some(c) => fl!(
                    "details-cached-partly",
                    have = human_bytes(c.have),
                    size = human_bytes(c.size)
                ),
            },
            _ if self.local.waiting() => PLACEHOLDER.to_string(),
            _ => NONE.to_string(),
        };
        let size = match (&self.drive, self.local.ok()) {
            (Load::Done(Ok(DriveInfo { size: Some(s), .. })), _) => human_bytes(*s),
            (
                _,
                Some(Local::File {
                    cached: Some(c), ..
                }),
            ) => human_bytes(c.size),
            (Load::Pending, _) => PLACEHOLDER.to_string(),
            _ => NONE.to_string(),
        };
        vec![
            (
                fl!("details-row-drive-modified"),
                from_drive(&|d| d.modified.map(status::fmt_time)),
            ),
            (fl!("details-row-cached-locally"), cached),
            (fl!("details-row-size"), size),
            (
                fl!("details-row-version"),
                from_drive(&|d| d.version.clone()),
            ),
        ]
    }

    // --- sharing ---

    /// The caption on the right of the "Sharing" heading.
    pub fn sharing_summary(&self) -> String {
        match &self.drive {
            Load::Pending => PLACEHOLDER.to_string(),
            Load::Done(Err(_)) => fl!("details-sharing-unavailable"),
            Load::Done(Ok(d)) if !d.shared => fl!("details-sharing-not-shared"),
            Load::Done(Ok(d)) => {
                let others = d
                    .permissions
                    .iter()
                    .filter(|p| p.role != drive::Role::Owner)
                    .count();
                fl!("details-sharing-shared-with", count = others)
            }
        }
    }

    /// An icon and message to show instead of the table (empty states and errors).
    pub fn sharing_note(&self) -> Option<(&'static str, String)> {
        match &self.drive {
            Load::Pending => Some(("content-loading-symbolic", PLACEHOLDER.to_string())),
            Load::Done(Err(e)) => Some((
                "dialog-warning-symbolic",
                fl!("details-sharing-error", error = e.as_str()),
            )),
            Load::Done(Ok(d)) if !d.shared => {
                Some(("changes-prevent-symbolic", fl!("details-sharing-only-you")))
            }
            Load::Done(Ok(d)) if d.permissions.is_empty() => Some((
                "dialog-information-symbolic",
                fl!("details-sharing-unlisted"),
            )),
            Load::Done(Ok(_)) => None,
        }
    }

    pub fn permissions(&self) -> &[Permission] {
        match &self.drive {
            Load::Done(Ok(d)) if d.shared => &d.permissions,
            _ => &[],
        }
    }

    // --- offline ---

    /// The "Keep offline" section for a folder, or a note for things inside an offline folder.
    /// `None` when offline folders are off, or the item is a file outside any offline folder.
    pub fn offline_row(&self) -> Option<OfflineRow> {
        if !self.cfg.offline_enabled {
            return None;
        }
        let rel = self.remote_rel()?;
        let covering = pin::covering_pin(&self.cfg.pins, &rel);
        let (title, caption, button) = match covering {
            Some(p) if p != rel => (
                Phase::Ready.label(),
                fl!(
                    "details-offline-part-of",
                    folder = p,
                    state = pin::describe(self.offline.pin(p), None)
                ),
                OfflineButton::Hidden,
            ),
            _ if !self.is_dir => return None,
            Some(p) => (
                fl!("details-keep-offline"),
                pin::describe(self.offline.pin(p), self.offline_usage),
                OfflineButton::Stop,
            ),
            None => (
                fl!("details-keep-offline"),
                fl!("details-keep-offline-hint"),
                OfflineButton::Keep,
            ),
        };
        let caption = self.offline_error.clone().unwrap_or(caption);
        Some(OfflineRow {
            title,
            caption,
            button,
        })
    }

    /// The keeper's phase for the pin covering this item, e.g. "Downloading for offline use".
    fn pinned_phase_label(&self) -> String {
        if !self.cfg.offline_enabled {
            return String::new();
        }
        self.remote_rel()
            .and_then(|rel| pin::covering_pin(&self.cfg.pins, &rel).map(str::to_string))
            .map_or_else(String::new, |p| {
                self.offline
                    .pin(&p)
                    .map_or(Phase::Downloading, |s| s.phase)
                    .label()
            })
    }

    // --- buttons ---

    pub fn buttons(&self) -> Buttons {
        let manage_label = self.target.ok().map_or_else(
            || fl!("details-manage-sharing"),
            |t| {
                fl!(
                    "details-manage-sharing-provider",
                    provider = t.provider.name()
                )
            },
        );
        Buttons {
            open: if self.is_dir {
                OpenButton::Direct
            } else {
                OpenButton::Menu
            },
            manage_label,
            manage_enabled: self.target.ok().is_some(),
            offline: self
                .offline_row()
                .map_or(OfflineButton::Hidden, |r| r.button),
        }
    }

    /// The icon for the item itself: a folder, or the first theme name for its file type.
    fn icon_name(&self) -> String {
        if self.is_dir {
            return "folder".to_string();
        }
        mime_icon::icon_names(&self.name)
            .into_iter()
            .next()
            .unwrap_or_else(|| mime_icon::FALLBACK_MIME_ICON.to_string())
    }

    /// Everything the QML side shows, as plain values.
    pub fn view(&self) -> DetailsView {
        let (sync_icon, sync_label, sync_detail) = self.status_line();
        let rows = self.detail_rows();
        let row = |label: &str| {
            rows.iter()
                .find(|(l, _)| l == label)
                .map_or_else(String::new, |(_, v)| v.clone())
        };
        let buttons = self.buttons();
        let offline = self.offline_row();
        let (note_icon, note) = self
            .sharing_note()
            .map_or((String::new(), String::new()), |(i, t)| (i.to_string(), t));
        let (cached_label, cached) = if self.is_dir {
            let label = rows
                .get(2)
                .map_or_else(|| fl!("details-row-cached"), |r| r.0.clone());
            let value = row(&label);
            (label, value)
        } else {
            (fl!("details-row-cached"), String::new())
        };
        DetailsView {
            name: self.name.clone(),
            path: self.path.clone(),
            is_dir: self.is_dir,
            icon_name: self.icon_name(),
            error: self.target.err().unwrap_or_default().to_string(),
            sync_label,
            sync_icon: sync_icon.to_string(),
            sync_detail,
            drive_modified: row(&fl!("details-row-drive-modified")),
            cache_state: row(&fl!("details-row-cached-locally")),
            size: row(&fl!("details-row-size")),
            version: row(&fl!("details-row-version")),
            uploading: row(&fl!("details-row-uploading")),
            downloading: row(&fl!("details-row-downloading")),
            cached,
            cached_label,
            shared: matches!(&self.drive, Load::Done(Ok(d)) if d.shared),
            sharing_summary: self.sharing_summary(),
            sharing_note: note,
            sharing_note_icon: note_icon,
            permissions_json: permissions_json(self.permissions()),
            manage_label: buttons.manage_label,
            manage_enabled: buttons.manage_enabled,
            open_is_menu: buttons.open == OpenButton::Menu,
            offline_enabled: offline.is_some(),
            offline_title: offline
                .as_ref()
                .map_or_else(String::new, |o| o.title.clone()),
            offline_caption: offline
                .as_ref()
                .map_or_else(String::new, |o| o.caption.clone()),
            offline_action: match buttons.offline {
                OfflineButton::Hidden => "none",
                OfflineButton::Keep => "keep",
                OfflineButton::Stop => "stop",
            }
            .to_string(),
            pinned_phase_label: self.pinned_phase_label(),
            offline_error: self.offline_error.clone().unwrap_or_default(),
        }
    }
}

/// The theme icon for a kind of grantee.
pub fn kind_icon(kind: Kind) -> &'static str {
    match kind {
        Kind::User => "user-identity",
        Kind::Group => "system-users",
        Kind::Domain => "network-workgroup",
        Kind::Anyone => "insert-link",
    }
}

/// One row of the sharing table, as finished strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRow {
    pub name: String,
    /// Empty when it would only repeat the name.
    pub address: String,
    /// "Name <address>", or just the name; for tooltips.
    pub full: String,
    pub role: String,
    pub kind_label: String,
    pub kind: Kind,
}

/// The sharing table rows, in the order Drive listed them.
pub fn permission_rows(perms: &[Permission]) -> Vec<PermissionRow> {
    perms
        .iter()
        .map(|p| {
            let address = if p.address == p.name { "" } else { &p.address };
            let full = if address.is_empty() {
                p.name.clone()
            } else {
                format!("{} <{address}>", p.name)
            };
            PermissionRow {
                name: p.name.clone(),
                address: address.to_string(),
                full,
                role: p.role.label(),
                kind_label: p.kind.label(),
                kind: p.kind,
            }
        })
        .collect()
}

/// The sharing table as JSON for the QML list: name, address, role, type and an icon.
pub fn permissions_json(perms: &[Permission]) -> String {
    let rows: Vec<_> = permission_rows(perms)
        .into_iter()
        .map(|r| {
            json!({
                "name": r.name,
                "address": r.address,
                "full": r.full,
                "role": r.role,
                "kind": r.kind_label,
                "icon": kind_icon(r.kind),
            })
        })
        .collect();
    serde_json::Value::Array(rows).to_string()
}

/// The folder the file lives in, for the "Open → Folder" choice.
pub fn parent_dir(path: &str) -> String {
    Path::new(path)
        .parent()
        .map_or_else(String::new, |p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MIN_LOADING;
    use rclone_ui_core::drive::Role;
    use rclone_ui_core::share::Provider;
    use rclone_ui_core::status::{Cached, FolderSummary};

    fn state(is_dir: bool) -> DetailsState {
        let mut s = DetailsState::new(Config::default(), &["/m/d/a.txt".to_string()]);
        s.is_dir = is_dir;
        s
    }

    fn target(is_dir: bool) -> Target {
        Target {
            provider: Provider::GoogleDrive,
            id: "id".into(),
            is_dir,
            fs: "gdrive:".into(),
            rel: "d/a.txt".into(),
        }
    }

    fn cached(size: u64, have: u64, dirty: bool) -> Cached {
        Cached {
            size,
            have,
            mtime: Some("2026-05-10T02:33:55Z".parse().unwrap()),
            dirty,
        }
    }

    fn file_local(transferring: bool, cached: Option<Cached>) -> Local {
        Local::File {
            transferring,
            cached,
        }
    }

    fn drive_info() -> DriveInfo {
        DriveInfo {
            modified: Some("2026-05-10T02:33:55Z".parse().unwrap()),
            size: Some(10),
            version: Some("7".into()),
            ..DriveInfo::default()
        }
    }

    fn loaded(is_dir: bool, local: Local, drive: Result<DriveInfo, String>) -> DetailsState {
        let mut s = state(is_dir);
        s.target = Load::Done(Ok(target(is_dir)));
        s.local = Load::Done(Ok(local));
        s.drive = Load::Done(drive);
        s
    }

    fn perm(name: &str, role: Role) -> Permission {
        Permission {
            name: name.into(),
            address: format!("{name}@example.com"),
            role,
            kind: Kind::User,
        }
    }

    // ---- stages ----

    #[test]
    fn loading_until_every_stage_has_answered() {
        let mut s = state(false);
        assert!(!s.all_landed());
        s.target = Load::Done(Ok(target(false)));
        assert!(!s.all_landed(), "local and drive still pending");
        s.apply_local(Ok(file_local(false, None)));
        assert!(!s.all_landed(), "drive still pending");
        s.apply_drive(Ok(drive_info()));
        assert!(s.all_landed());
        assert_eq!(MIN_LOADING.as_millis(), 450);
    }

    #[test]
    fn a_failed_lookup_fills_every_stage_so_the_spinner_can_end() {
        let mut s = state(false);
        s.apply_target(Err("Not inside the mount".into()));
        assert_eq!(s.target.err(), Some("Not inside the mount"));
        assert_eq!(s.local.err(), Some("Not inside the mount"));
        assert_eq!(s.drive.err(), Some("Not inside the mount"));
        assert!(s.all_landed());
        let (icon, label, _) = s.status_line();
        assert_eq!(
            (icon, label.as_str()),
            ("dialog-warning-symbolic", "Not inside the mount")
        );
        assert_eq!(s.view().error, "Not inside the mount");
    }

    #[test]
    fn resolving_does_not_finish_the_other_stages() {
        let mut s = state(false);
        s.apply_target(Ok(target(false)));
        assert!(s.target.ok().is_some());
        assert!(s.local.waiting() && s.drive.waiting());
        assert!(!s.all_landed());
    }

    #[test]
    fn a_failed_refresh_keeps_the_last_good_counts() {
        let mut s = state(true);
        let good = Local::Folder(FolderSummary {
            uploading: 2,
            ..FolderSummary::default()
        });
        s.apply_local(Ok(good.clone()));
        s.apply_local(Err("rc down".into()));
        assert_eq!(s.local.ok(), Some(&good));
        // But a first failure is shown.
        s.local = Load::Pending;
        s.apply_local(Err("rc down".into()));
        assert_eq!(s.local.err(), Some("rc down"));
    }

    // ---- status line ----

    #[test]
    fn status_says_checking_until_it_can_decide() {
        let (icon, label, _) = state(false).status_line();
        assert_eq!(
            (icon, label.as_str()),
            ("content-loading-symbolic", "Checking…")
        );
        // Cached and complete: needs Drive before it can say "in sync".
        let mut s = state(false);
        s.target = Load::Done(Ok(target(false)));
        s.local = Load::Done(Ok(file_local(false, Some(cached(10, 10, false)))));
        assert_eq!(s.status_line().1, "Checking…");
    }

    #[test]
    fn uncached_files_do_not_wait_for_drive() {
        let mut s = state(false);
        s.target = Load::Done(Ok(target(false)));
        s.local = Load::Done(Ok(file_local(false, None)));
        assert_eq!(s.status_line().1, "Online only (not cached)");
    }

    #[test]
    fn cached_files_compare_against_drive() {
        let s = loaded(
            false,
            file_local(false, Some(cached(10, 10, false))),
            Ok(drive_info()),
        );
        let (icon, label, _) = s.status_line();
        assert_eq!(
            (icon, label.as_str()),
            ("emblem-ok-symbolic", "In sync (cached)")
        );
    }

    #[test]
    fn upload_states_come_from_the_local_view() {
        let queued = loaded(
            false,
            file_local(false, Some(cached(10, 10, true))),
            Err("x".into()),
        );
        assert_eq!(queued.status_line().1, "Queued for upload");
        let active = loaded(
            false,
            file_local(true, Some(cached(10, 10, true))),
            Err("x".into()),
        );
        assert_eq!(active.status_line().1, "Uploading");
    }

    #[test]
    fn a_drive_failure_does_not_hide_what_is_known_locally() {
        let s = loaded(
            false,
            file_local(false, Some(cached(10, 10, false))),
            Err("denied".into()),
        );
        let (icon, label, _) = s.status_line();
        assert_eq!(icon, "dialog-warning-symbolic");
        assert!(label.contains("can't compare"), "{label}");
        let s = loaded(false, file_local(false, None), Err("denied".into()));
        assert_eq!(s.status_line().1, "Online only (not cached)");
    }

    #[test]
    fn a_local_failure_is_an_error_with_the_reason() {
        let mut s = loaded(false, file_local(false, None), Ok(drive_info()));
        s.local = Load::Done(Err("rclone not running".into()));
        let (icon, label, detail) = s.status_line();
        assert_eq!(
            (icon, label.as_str(), detail.as_str()),
            ("dialog-error-symbolic", "Error", "rclone not running")
        );
    }

    #[test]
    fn folders_summarise_transfers() {
        let folder =
            |f: FolderSummary| loaded(true, Local::Folder(f), Ok(drive_info())).status_line();
        let (_, label, detail) = folder(FolderSummary {
            uploading: 2,
            downloading: 1,
            ..FolderSummary::default()
        });
        assert_eq!(
            (label.as_str(), detail.as_str()),
            ("Transferring", "3 file(s)")
        );
        let (icon, label, _) = folder(FolderSummary::default());
        assert_eq!(
            (icon, label.as_str()),
            ("emblem-ok-symbolic", "Nothing pending")
        );
        let (_, _, detail) = folder(FolderSummary {
            uploading: 5,
            truncated: true,
            ..FolderSummary::default()
        });
        assert_eq!(detail, "5 or more file(s)");
    }

    /// A state for the offline folder `d` (mounted at `/m`) whose keeper is in `phase`.
    fn in_offline_folder(phase: Phase, syncing: bool) -> DetailsState {
        let mut s = state(true);
        s.cfg.mount_path = "/m".into();
        s.cfg.offline_enabled = true;
        s.cfg.pins = vec!["d".into()];
        s.path = "/m/d".into();
        s.target = Load::Done(Ok(target(true)));
        s.local = Load::Done(Ok(Local::Folder(FolderSummary::default())));
        s.offline.pins.push(pin::PinState {
            rel: "d".into(),
            phase,
            syncing,
            ..pin::PinState::default()
        });
        s
    }

    #[test]
    fn the_initial_offline_download_is_not_nothing_pending() {
        let mut s = in_offline_folder(Phase::Downloading, true);
        s.offline_usage = Some((12, 3 * 1024 * 1024));
        let (icon, label, detail) = s.status_line();
        assert_eq!(icon, SyncState::Downloading.icon());
        assert_eq!(label, "Downloading for offline use");
        assert_eq!(detail, "12 file(s), 3.0 MiB so far");
        s.offline_usage = None;
        assert_eq!(s.status_line().2, "");
    }

    #[test]
    fn an_offline_folder_shows_its_copy_instead_of_an_empty_cache() {
        let mut s = in_offline_folder(Phase::Ready, false);
        s.offline_usage = Some((362, 563 * 1024 * 1024));
        let rows = s.detail_rows();
        assert_eq!(rows[2].0, "Offline copy");
        assert!(rows[2].1.starts_with("362 file(s), 563"), "{}", rows[2].1);
        let view = s.view();
        assert_eq!(view.cached_label, "Offline copy");
        assert!(view.cached.starts_with("362 file(s)"));
        s.path = "/m/d/sub".into();
        assert_eq!(s.detail_rows()[2].0, "Cached");
    }

    #[test]
    fn a_file_in_an_idle_offline_folder_is_available_offline() {
        let mut s = in_offline_folder(Phase::Ready, false);
        s.is_dir = false;
        s.path = "/m/d/sub/a.txt".into();
        assert_eq!(s.status_line().1, "Available offline");
    }

    #[test]
    fn offline_syncs_and_problems_show_in_the_status_line() {
        let label = |phase, syncing| in_offline_folder(phase, syncing).status_line().1;
        assert_eq!(label(Phase::Ready, true), "Syncing offline copy");
        assert_eq!(label(Phase::Waiting, true), "Syncing offline copy");
        assert_eq!(label(Phase::Waiting, false), "Waiting to sync offline copy");
        assert_eq!(
            label(Phase::Attention, false),
            "Offline sync needs attention"
        );
        assert_eq!(label(Phase::Removing, false), "Removing offline copy");
        assert_eq!(label(Phase::Ready, false), "Nothing pending");
    }

    #[test]
    fn items_inside_the_offline_folder_follow_its_state_and_others_do_not() {
        let mut s = in_offline_folder(Phase::Downloading, true);
        s.path = "/m/d/sub/a.txt".into();
        assert_eq!(s.status_line().1, "Downloading for offline use");
        s.path = "/m/other".into();
        assert_eq!(s.status_line().1, "Nothing pending");
        s.path = "/m/d".into();
        s.cfg.offline_enabled = false;
        assert_eq!(s.status_line().1, "Nothing pending");
    }

    // ---- detail rows ----

    fn labels(rows: &[(String, String)]) -> Vec<&str> {
        rows.iter().map(|(l, _)| l.as_str()).collect()
    }

    fn value<'a>(rows: &'a [(String, String)], label: &str) -> &'a str {
        rows.iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
            .unwrap()
    }

    #[test]
    fn a_file_has_four_rows_and_a_folder_three_with_stable_labels() {
        let file = ["Drive modified", "Cached locally", "Size", "Version"];
        let folder = ["Uploading", "Downloading", "Cached"];
        assert_eq!(labels(&state(false).detail_rows()), file);
        assert_eq!(labels(&state(true).detail_rows()), folder);
        let done = loaded(false, file_local(false, None), Ok(drive_info()));
        assert_eq!(labels(&done.detail_rows()), file);
    }

    #[test]
    fn rows_show_placeholders_while_pending_and_dashes_on_failure() {
        let rows = state(false).detail_rows();
        assert!(rows.iter().all(|(_, v)| v == PLACEHOLDER));
        let mut s = state(false);
        s.local = Load::Done(Err("x".into()));
        s.drive = Load::Done(Err("x".into()));
        let rows = s.detail_rows();
        assert!(rows.iter().all(|(_, v)| v == NONE), "{rows:?}");
    }

    #[test]
    fn file_rows_combine_drive_and_cache() {
        let rows = loaded(
            false,
            file_local(false, Some(cached(10, 4, false))),
            Ok(drive_info()),
        )
        .detail_rows();
        assert_eq!(value(&rows, "Cached locally"), "Partly (4.0 B of 10.0 B)");
        assert_eq!(value(&rows, "Size"), "10.0 B");
        assert_eq!(value(&rows, "Version"), "7");
        assert!(value(&rows, "Drive modified").contains("2026"));
        let rows = loaded(
            false,
            file_local(false, Some(cached(10, 10, false))),
            Ok(drive_info()),
        )
        .detail_rows();
        assert_eq!(value(&rows, "Cached locally"), "Yes");
        let rows = loaded(false, file_local(false, None), Ok(drive_info())).detail_rows();
        assert_eq!(value(&rows, "Cached locally"), "No");
    }

    #[test]
    fn size_falls_back_to_the_cache_when_drive_has_none() {
        let rows = loaded(
            false,
            file_local(false, Some(cached(2048, 2048, false))),
            Err("x".into()),
        )
        .detail_rows();
        assert_eq!(value(&rows, "Size"), "2.0 KiB");
        assert_eq!(value(&rows, "Version"), NONE);
    }

    #[test]
    fn folder_rows_show_the_counts() {
        let rows = loaded(
            true,
            Local::Folder(FolderSummary {
                uploading: 1,
                downloading: 2,
                cached_files: 4,
                cached_bytes: 2048,
                truncated: true,
            }),
            Ok(drive_info()),
        )
        .detail_rows();
        assert_eq!(value(&rows, "Uploading"), "1");
        assert_eq!(value(&rows, "Downloading"), "2");
        assert_eq!(value(&rows, "Cached"), "4+ file(s), 2.0 KiB");
    }

    // ---- sharing ----

    #[test]
    fn sharing_summary_covers_every_state() {
        let mut s = state(false);
        assert_eq!(s.sharing_summary(), PLACEHOLDER);
        s.drive = Load::Done(Err("x".into()));
        assert_eq!(s.sharing_summary(), "Unavailable");
        s.drive = Load::Done(Ok(DriveInfo {
            shared: false,
            ..DriveInfo::default()
        }));
        assert_eq!(s.sharing_summary(), "Not shared");
        s.drive = Load::Done(Ok(DriveInfo {
            shared: true,
            permissions: vec![
                perm("me", Role::Owner),
                perm("amy", Role::Editor),
                perm("bob", Role::Viewer),
            ],
            ..DriveInfo::default()
        }));
        assert_eq!(s.sharing_summary(), "Shared with 2");
        assert!(s.sharing_note().is_none());
        assert_eq!(s.permissions().len(), 3);
    }

    #[test]
    fn sharing_notes_explain_empty_and_failed_states() {
        let mut s = state(false);
        s.drive = Load::Done(Err("denied".into()));
        assert!(s.sharing_note().unwrap().1.contains("denied"));
        s.drive = Load::Done(Ok(DriveInfo::default()));
        assert_eq!(s.sharing_note().unwrap().1, "Only you have access");
        assert_eq!(s.permissions().len(), 0);
        s.drive = Load::Done(Ok(DriveInfo {
            shared: true,
            ..DriveInfo::default()
        }));
        assert!(s.sharing_note().unwrap().1.contains("didn't list"));
    }

    #[test]
    fn permissions_serialise_for_the_table() {
        let json = permissions_json(&[
            perm("amy", Role::Editor),
            Permission {
                name: "same".into(),
                address: "same".into(),
                role: Role::Viewer,
                kind: Kind::Anyone,
            },
        ]);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v[0]["name"], "amy");
        assert_eq!(v[0]["role"], "Editor");
        assert_eq!(v[0]["full"], "amy <amy@example.com>");
        assert_eq!(v[1]["address"], "");
        assert_eq!(v[1]["full"], "same");
        assert_eq!(v[1]["kind"], "Anyone with link");
        assert_eq!(permissions_json(&[]), "[]");
        let rows = permission_rows(&[perm("amy", Role::Editor)]);
        assert_eq!(rows[0].full, "amy <amy@example.com>");
        assert_eq!(rows[0].kind, Kind::User);
    }

    // ---- buttons ----

    #[test]
    fn open_is_a_menu_only_for_files() {
        assert_eq!(state(false).buttons().open, OpenButton::Menu);
        assert_eq!(state(true).buttons().open, OpenButton::Direct);
        assert!(state(false).view().open_is_menu);
        assert_eq!(parent_dir("/m/d/a.txt"), "/m/d");
    }

    #[test]
    fn manage_sharing_waits_for_the_target() {
        let s = state(false);
        let b = s.buttons();
        assert_eq!(
            (b.manage_label.as_str(), b.manage_enabled),
            ("Manage Sharing", false)
        );
        let b = loaded(false, file_local(false, None), Ok(drive_info())).buttons();
        assert_eq!(b.manage_label, "Manage Sharing (Google Drive)");
        assert!(b.manage_enabled);
    }

    #[test]
    fn the_offline_button_follows_the_folder() {
        // Offline folders off: nothing.
        let mut s = in_offline_folder(Phase::Ready, false);
        s.cfg.offline_enabled = false;
        assert_eq!(s.buttons().offline, OfflineButton::Hidden);
        assert!(s.offline_row().is_none());
        // The offline folder itself can be stopped.
        let mut s = in_offline_folder(Phase::Ready, false);
        assert_eq!(s.buttons().offline, OfflineButton::Stop);
        assert_eq!(s.view().offline_action, "stop");
        assert_eq!(s.view().pinned_phase_label, Phase::Ready.label());
        // A folder inside it only gets a note.
        s.path = "/m/d/sub".into();
        let row = s.offline_row().unwrap();
        assert_eq!(row.button, OfflineButton::Hidden);
        assert!(row.caption.starts_with("Part of the offline folder d"));
        // A folder outside can be kept.
        s.path = "/m/other".into();
        assert_eq!(s.buttons().offline, OfflineButton::Keep);
        assert_eq!(s.view().offline_action, "keep");
        assert_eq!(s.view().pinned_phase_label, "");
        // A file outside has no offline row at all.
        s.is_dir = false;
        assert!(s.offline_row().is_none());
        assert!(!s.view().offline_enabled);
    }

    #[test]
    fn an_offline_error_replaces_the_caption() {
        let mut s = in_offline_folder(Phase::Ready, false);
        s.offline_error = Some("nope".into());
        assert_eq!(s.offline_row().unwrap().caption, "nope");
        assert_eq!(s.view().offline_error, "nope");
    }

    #[test]
    fn views_build_for_every_state() {
        let states = [
            state(false),
            state(true),
            loaded(
                false,
                file_local(false, Some(cached(10, 10, false))),
                Ok(drive_info()),
            ),
            loaded(
                true,
                Local::Folder(FolderSummary::default()),
                Ok(drive_info()),
            ),
            loaded(false, file_local(false, None), Err("denied".into())),
        ];
        for s in states {
            let v = s.view();
            assert_ne!(v.sync_label, "");
            assert_ne!(v.icon_name, "");
        }
        assert_eq!(state(true).view().icon_name, "folder");
    }
}
