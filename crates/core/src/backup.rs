// SPDX-License-Identifier: GPL-3.0-only
//! Folder backups: any local folder, mirrored to `<remote>:Computers/<host>/<name>`.
//!
//! The Google Drive "Computers" tab can't be written through Google's public API (only Drive
//! for desktop can make a computer entry there), so backups live in a plain `Computers` folder
//! at the top of the remote, one subfolder per machine. It works with any rclone remote and
//! any backup can be restored (and optionally linked again) to any local folder.
//!
//! A backup is a mirror (`rclone sync`): the remote copy ends up exactly like the local folder,
//! so local deletions are deleted remotely too (Google Drive keeps them in its trash). A user
//! unit (`<slug>-backup.service`) runs [`run_keeper`], which mirrors every folder on start, when
//! the list changes, and every `backup_interval_secs`. It runs the `rclone` command line with
//! rclone's own config, independently of the mount, so backups also run while the mount is down.
//!
//! Persisted state and config carry codes (`source-missing`, ...); [`error_label`] translates
//! them at display time.

use crate::config::{self, BackupFolder, Config};
use rclone_ui_i18n::fl;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The top-level remote folder every machine's backups go under.
pub const COMPUTERS: &str = "Computers";

/// One folder's last outcome.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FolderStatus {
    /// Unix seconds of the last complete mirror.
    pub last_success: Option<u64>,
    /// Failure code of the last attempt (see [`error_label`]); `None` after a success.
    pub error: Option<String>,
}

/// What the keeper is doing and how each folder fared, keyed by the folder's `source`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BackupStatus {
    /// A pass is running right now.
    pub running: bool,
    /// The folder being mirrored right now.
    pub current: Option<String>,
    /// Per folder, keyed by `source`.
    pub folders: BTreeMap<String, FolderStatus>,
}

/// Translate a failure code for display in the current locale.
#[must_use]
pub fn error_label(code: &str) -> String {
    match code {
        "invalid-source" => fl!("backup-error-invalid-source"),
        "source-missing" => fl!("backup-error-source-missing"),
        "source-empty" => fl!("backup-error-source-empty"),
        "source-overlap" => fl!("backup-error-source-overlap"),
        "source-in-mount" => fl!("backup-error-source-in-mount"),
        "already-backed-up" => fl!("backup-error-already-backed-up"),
        "not-backed-up" => fl!("backup-error-not-backed-up"),
        "invalid-destination" => fl!("backup-error-invalid-destination"),
        "destination-overlap" => fl!("backup-error-destination-overlap"),
        "remote-unknown" => fl!("backup-error-remote-unknown"),
        "rclone-unavailable" => fl!("backup-error-rclone-unavailable"),
        "copy-failed" => fl!("backup-error-copy-failed"),
        "list-failed" => fl!("backup-error-list-failed"),
        "interrupted" => fl!("backup-error-interrupted"),
        "config-unreadable" => fl!("backup-error-config-unreadable"),
        "service-failed" => fl!("backup-error-service-failed"),
        _ => fl!("backup-error-unknown"),
    }
}

/// Runs `rclone` with the given arguments: stdout on success, stderr or a code on failure.
pub type Rclone<'a> = &'a dyn Fn(&[&str]) -> Result<String, RcloneError>;

/// Why an `rclone` run failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RcloneError {
    /// rclone couldn't be started.
    Unavailable,
    /// rclone exited unsuccessfully, with this stderr.
    Failed(String),
}

/// The real `rclone` command line.
///
/// # Errors
/// See [`RcloneError`].
pub fn rclone_cli(args: &[&str]) -> Result<String, RcloneError> {
    let out = Command::new("rclone")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| RcloneError::Unavailable)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(RcloneError::Failed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

fn copy_code(e: &RcloneError) -> String {
    match e {
        RcloneError::Unavailable => "rclone-unavailable",
        RcloneError::Failed(_) => "copy-failed",
    }
    .into()
}

fn state_path() -> PathBuf {
    crate::pin::state_dir().join("backup.json")
}

fn lock_path(name: &str) -> PathBuf {
    crate::pin::state_dir().join(name)
}

fn lock_file(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())
}

fn read_status(path: &Path) -> Result<BackupStatus, String> {
    match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BackupStatus::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn write_status(path: &Path, status: &BackupStatus) -> Result<(), String> {
    let text = serde_json::to_string(status).map_err(|e| e.to_string())?;
    config::write_atomic_mode(path, &text, Some(0o600)).map_err(|e| e.to_string())
}

/// The keeper's last recorded status. A pass that was cut off (the keeper was killed) shows as
/// `interrupted` on the folder it was working on.
///
/// # Errors
/// A message if the state file can't be read.
pub fn status() -> Result<BackupStatus, String> {
    status_in(&state_path(), &lock_path("backup-running.lock"))
}

fn status_in(path: &Path, lock: &Path) -> Result<BackupStatus, String> {
    let file = lock_file(lock)?;
    let locked = file.try_lock();
    let mut state = read_status(path)?;
    match locked {
        Ok(()) => {
            if state.running {
                state.running = false;
                if let Some(source) = state.current.take() {
                    state.folders.entry(source).or_default().error = Some("interrupted".into());
                }
            }
        }
        Err(std::fs::TryLockError::WouldBlock) => state.running = true,
        Err(std::fs::TryLockError::Error(e)) => return Err(e.to_string()),
    }
    Ok(state)
}

/// `remote:path` split into the remote name and its non-empty path segments.
fn destination_parts(destination: &str) -> Result<(&str, Vec<&str>), &'static str> {
    let (remote, path) = destination.split_once(':').ok_or("invalid-destination")?;
    if remote.is_empty()
        || !remote
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b' '))
        || path.is_empty()
        || path.starts_with('/')
        || path.chars().any(char::is_control)
    {
        return Err("invalid-destination");
    }
    let parts: Vec<_> = path
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if parts.is_empty() || parts.contains(&"..") {
        return Err("invalid-destination");
    }
    Ok((remote, parts))
}

/// An absolute path with `.`/`..` resolved and symlinks resolved as far as the path exists.
fn normalized_absolute(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(config::expand(path));
    if !path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => out.push("/"),
            Component::Normal(p) => out.push(p),
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir | Component::Prefix(_) => {}
        }
    }
    let mut ancestor = out.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent()?;
    }
    let canonical = fs::canonicalize(ancestor).ok()?;
    let rest = out.strip_prefix(ancestor).ok()?;
    // `join("")` would add a trailing slash.
    Some(if rest.as_os_str().is_empty() {
        canonical
    } else {
        canonical.join(rest)
    })
}

fn source_path(source: &str) -> Result<PathBuf, &'static str> {
    if !Path::new(&config::expand(source)).is_absolute() {
        return Err("invalid-source");
    }
    let path = normalized_absolute(source).ok_or("invalid-source")?;
    if !path.is_dir() {
        return Err("source-missing");
    }
    Ok(path)
}

/// Folders the app itself keeps (the mount, rclone's cache, offline copies and state). A backup
/// may contain them, and they are left out of it, but can't be inside one.
fn own_dirs(cfg: &Config) -> (Option<PathBuf>, Vec<PathBuf>) {
    let mount = normalized_absolute(&cfg.mount_path);
    let others = [
        cfg.cache_dir.clone(),
        crate::pin::data_dir().to_string_lossy().into_owned(),
        crate::pin::state_dir().to_string_lossy().into_owned(),
    ]
    .iter()
    .filter_map(|p| normalized_absolute(p))
    .collect();
    (mount, others)
}

/// Escape rclone filter-glob metacharacters so a path matches literally.
fn glob_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `--exclude` rules for the app's own folders inside `source`.
fn excludes(cfg: &Config, source: &Path) -> Vec<String> {
    let (mount, others) = own_dirs(cfg);
    mount
        .into_iter()
        .chain(others)
        .filter_map(|d| {
            let rel = d.strip_prefix(source).ok()?.to_str()?.to_string();
            (!rel.is_empty()).then(|| format!("/{}/**", glob_escape(&rel)))
        })
        .collect()
}

/// Check one folder against the rules every backup follows, and against `others` (the rest of
/// the list).
fn check_folder(
    cfg: &Config,
    folder: &BackupFolder,
    others: &[BackupFolder],
) -> Result<PathBuf, &'static str> {
    let source = source_path(&folder.source)?;
    let (mount, own) = own_dirs(cfg);
    if mount.as_ref().is_some_and(|m| source.starts_with(m)) {
        return Err("source-in-mount");
    }
    if own.iter().any(|d| source.starts_with(d)) {
        return Err("source-overlap");
    }
    let (remote, path) = destination_parts(&folder.destination)?;
    for other in others {
        if normalized_absolute(&other.source)
            .is_some_and(|o| source.starts_with(&o) || o.starts_with(&source))
        {
            return Err("source-overlap");
        }
        if let Ok((r, p)) = destination_parts(&other.destination) {
            if r == remote && (p.starts_with(&path) || path.starts_with(&p)) {
                return Err("destination-overlap");
            }
        }
    }
    Ok(source)
}

/// The machine name backups are filed under (`Computers/<host>`).
#[must_use]
pub fn hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.contains('/'))
        .unwrap_or_else(|| "linux".into())
}

/// The remote's name (`gdrive`) from the mount's fs (`gdrive:` or `gdrive:sub/path`).
fn remote_name(fs: &str) -> Option<&str> {
    let (name, _) = fs.split_once(':')?;
    (!name.is_empty() && !name.starts_with(':')).then_some(name)
}

/// Which remote backups go to: the mounted one, else the one existing backups use.
fn backup_remote(cfg: &Config, mount_fs: Option<&str>) -> Option<String> {
    mount_fs
        .and_then(remote_name)
        .or_else(|| {
            cfg.backup_folders
                .first()
                .and_then(|f| remote_name(&f.destination))
        })
        .map(str::to_string)
}

/// `base`, or `base (2)`, `base (3)`, ... whichever isn't in `taken`.
fn unique_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    // At most `taken.len() + 1` candidates can be needed.
    (2..=taken.len() + 2)
        .map(|n| format!("{base} ({n})"))
        .find(|n| !taken.iter().any(|t| t == n))
        .unwrap_or_default()
}

/// Folder names directly under `dir` on the remote (empty if `dir` doesn't exist yet).
fn remote_dirs(rclone: Rclone, dir: &str) -> Result<Vec<String>, &'static str> {
    match rclone(&["lsf", "--dirs-only", "--", dir]) {
        Ok(out) => Ok(out
            .lines()
            .map(|l| l.trim_end_matches('/').to_string())
            .filter(|l| !l.is_empty())
            .collect()),
        Err(RcloneError::Failed(e)) if e.contains("directory not found") => Ok(Vec::new()),
        Err(RcloneError::Unavailable) => Err("rclone-unavailable"),
        Err(RcloneError::Failed(_)) => Err("list-failed"),
    }
}

/// Work out where a new backup of `source` goes: `<remote>:Computers/<host>/<folder name>`,
/// with ` (2)`... added when that name is already used by another backup or already exists on
/// the remote (a mirror would otherwise replace what's there).
fn plan_add(
    cfg: &Config,
    source: &str,
    mount_fs: Option<&str>,
    host: &str,
    rclone: Rclone,
) -> Result<BackupFolder, &'static str> {
    let path = source_path(source)?;
    let source = path.to_string_lossy().into_owned();
    if cfg
        .backup_folders
        .iter()
        .any(|f| normalized_absolute(&f.source).is_some_and(|p| p == path))
    {
        return Err("already-backed-up");
    }
    let remote = backup_remote(cfg, mount_fs).ok_or("remote-unknown")?;
    let base = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(host);
    let dir = format!("{remote}:{COMPUTERS}/{host}");
    let mut taken = remote_dirs(rclone, &dir)?;
    taken.extend(cfg.backup_folders.iter().filter_map(|f| {
        f.destination
            .strip_prefix(&format!("{dir}/"))
            .map(str::to_string)
    }));
    let folder = BackupFolder {
        source,
        destination: format!("{dir}/{}", unique_name(base, &taken)),
    };
    check_folder(cfg, &folder, &cfg.backup_folders)?;
    Ok(folder)
}

/// Start backing up `source` (any local folder outside the mount). Returns the new entry.
///
/// # Errors
/// A failure code (see [`error_label`]).
pub fn add(source: &str) -> Result<BackupFolder, String> {
    let cfg = config::store()
        .load_checked()
        .map_err(|_| "config-unreadable")?;
    let mount_fs = crate::rc::mount_fs(&cfg.mount_path);
    let folder = plan_add(&cfg, source, mount_fs.as_deref(), &hostname(), &rclone_cli)?;
    save_folders(|list| list.push(folder.clone()))?;
    Ok(folder)
}

/// Stop backing up `source`. The remote copy is left as it is.
///
/// # Errors
/// A failure code (see [`error_label`]).
pub fn remove(source: &str) -> Result<BackupFolder, String> {
    let cfg = config::store()
        .load_checked()
        .map_err(|_| "config-unreadable")?;
    let want = normalized_absolute(source);
    let folder = cfg
        .backup_folders
        .iter()
        .find(|f| f.source == source || normalized_absolute(&f.source) == want)
        .cloned()
        .ok_or("not-backed-up")?;
    save_folders(|list| list.retain(|f| f != &folder))?;
    Ok(folder)
}

/// Is `path` (or a folder containing it) backed up? Returns that entry.
#[must_use]
pub fn backed_up(cfg: &Config, path: &str) -> Option<BackupFolder> {
    let path = normalized_absolute(path)?;
    cfg.backup_folders
        .iter()
        .find(|f| normalized_absolute(&f.source).is_some_and(|s| path.starts_with(s)))
        .cloned()
}

/// Edit the backup list in the shared config, then install or remove the keeper's unit to match.
fn save_folders(edit: impl Fn(&mut Vec<BackupFolder>)) -> Result<(), String> {
    let cfg = config::store()
        .save(&mut |c| edit(&mut c.backup_folders))
        .map_err(|_| "config-unreadable")?;
    let unit = if cfg.backup_folders.is_empty() {
        remove_service()
    } else {
        crate::units::shared_exe().and_then(|exe| {
            install_in(
                &crate::units::unit_dir(),
                &exe,
                &unit_name(),
                &crate::units::systemctl,
            )
        })
    };
    unit.map_err(|_| "service-failed".to_string())
}

/// A backup found on the remote, under `Computers/<host>/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBackup {
    /// The machine it came from.
    pub host: String,
    /// The folder's name.
    pub name: String,
    /// Its rclone path, for [`restore`].
    pub destination: String,
}

fn list_remote_in(remote: &str, rclone: Rclone) -> Result<Vec<RemoteBackup>, &'static str> {
    let root = format!("{remote}:{COMPUTERS}");
    let mut out = Vec::new();
    for host in remote_dirs(rclone, &root)? {
        for name in remote_dirs(rclone, &format!("{root}/{host}"))? {
            out.push(RemoteBackup {
                destination: format!("{root}/{host}/{name}"),
                host: host.clone(),
                name,
            });
        }
    }
    Ok(out)
}

/// Every backup on the remote, from any machine.
///
/// # Errors
/// A failure code (see [`error_label`]).
pub fn list_remote() -> Result<Vec<RemoteBackup>, String> {
    let cfg = config::store().load();
    let remote = backup_remote(&cfg, crate::rc::mount_fs(&cfg.mount_path).as_deref())
        .ok_or("remote-unknown")?;
    Ok(list_remote_in(&remote, &rclone_cli)?)
}

/// Check a restore before anything is downloaded; returns the entry a link would add.
fn plan_restore(
    cfg: &Config,
    destination: &str,
    target: &str,
    link: bool,
) -> Result<BackupFolder, &'static str> {
    destination_parts(destination)?;
    let path = normalized_absolute(target).ok_or("invalid-source")?;
    if path.exists() && !path.is_dir() {
        return Err("invalid-source");
    }
    let (mount, own) = own_dirs(cfg);
    if mount.as_ref().is_some_and(|m| path.starts_with(m)) {
        return Err("source-in-mount");
    }
    if own.iter().any(|d| path.starts_with(d)) {
        return Err("source-overlap");
    }
    let folder = BackupFolder {
        source: path.to_string_lossy().into_owned(),
        destination: destination.to_string(),
    };
    if link {
        // Another folder already mirroring here would undo the restore on its next pass.
        if cfg.backup_folders.iter().any(|f| {
            normalized_absolute(&f.source)
                .is_some_and(|s| path.starts_with(&s) || s.starts_with(&path))
        }) {
            return Err("already-backed-up");
        }
        if cfg
            .backup_folders
            .iter()
            .any(|f| destination_parts(&f.destination).ok() == destination_parts(destination).ok())
        {
            return Err("destination-overlap");
        }
    }
    Ok(folder)
}

/// Download the backup at `destination` into `target` (created if missing; files already there
/// are kept unless the backup has a newer copy). With `link`, `target` is then backed up to
/// `destination` from now on.
///
/// # Errors
/// A failure code (see [`error_label`]).
pub fn restore(destination: &str, target: &str, link: bool) -> Result<BackupFolder, String> {
    let cfg = config::store()
        .load_checked()
        .map_err(|_| "config-unreadable")?;
    let folder = plan_restore(&cfg, destination, target, link)?;
    fs::create_dir_all(&folder.source).map_err(|_| "invalid-source")?;
    rclone_cli(&[
        "copy",
        "--create-empty-src-dirs",
        "--",
        destination,
        &folder.source,
    ])
    .map_err(|e| copy_code(&e))?;
    if link {
        // Re-check against the list as it is now: it may have changed during the download.
        let cfg = config::store()
            .load_checked()
            .map_err(|_| "config-unreadable")?;
        plan_restore(&cfg, destination, &folder.source, true)?;
        save_folders(|list| list.push(folder.clone()))?;
    }
    Ok(folder)
}

/// Run `--backup-restore` in a separate process of the shared binary, which notifies when it
/// is done (a window can close meanwhile).
///
/// # Errors
/// A message if the process couldn't be started.
pub fn spawn_restore(destination: &str, target: &str, link: bool) -> Result<(), String> {
    let exe = crate::units::shared_exe()?;
    let mut cmd = Command::new(exe);
    cmd.args(["--backup-restore", destination, target]);
    if link {
        cmd.arg("--link");
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Tell the user how a command started from a file manager or window went (no terminal there).
pub fn notify(text: &str) {
    let _ = Command::new("notify-send")
        .args([
            &format!("--app-name={}", fl!("common-app-name")),
            "--icon=folder-sync-symbolic",
            &fl!("backup-heading"),
            text,
        ])
        .status();
}

const UNIT_MARKER: &str = "# Written by rclone-shell-ext backup; do not edit.\n";

fn unit_name() -> String {
    format!("{}-backup.service", crate::identity::identity().slug)
}

fn unit_contents(exe: &Path) -> String {
    let exe = exe
        .to_string_lossy()
        .replace('%', "%%")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!(
        "{UNIT_MARKER}[Unit]\nDescription=Back up local folders with rclone (rclone-shell-ext)\n\
         After=network-online.target\n\n[Service]\nExecStart=\"{exe}\" --backup-keeper\n\
         Restart=on-failure\nRestartSec=30\n\n[Install]\nWantedBy=default.target\n"
    )
}

fn owned_unit(path: &Path) -> Result<bool, String> {
    match fs::read_to_string(path) {
        Ok(text) if text.starts_with(UNIT_MARKER) => Ok(true),
        Ok(_) => Err(format!(
            "refusing to replace or remove a unit this app didn't write: {}",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

fn install_in(
    dir: &Path,
    exe: &Path,
    name: &str,
    systemctl: &dyn Fn(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    let path = dir.join(name);
    owned_unit(&path)?;
    if crate::units::write_if_changed(&path, &unit_contents(exe)).map_err(|e| e.to_string())? {
        systemctl(&["daemon-reload"])?;
    }
    systemctl(&["enable", "--now", name])
}

fn remove_in(
    dir: &Path,
    name: &str,
    systemctl: &dyn Fn(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    let path = dir.join(name);
    if !owned_unit(&path)? {
        return Ok(());
    }
    systemctl(&["disable", "--now", name])?;
    fs::remove_file(path).map_err(|e| e.to_string())?;
    systemctl(&["daemon-reload"])
}

/// Stop, disable and remove the backup unit; settings and remote copies are left alone.
///
/// # Errors
/// systemctl's message.
pub fn remove_service() -> Result<(), String> {
    remove_in(
        &crate::units::unit_dir(),
        &unit_name(),
        &crate::units::systemctl,
    )
}

/// Mirror one folder (the caller holds the run lock).
fn mirror(cfg: &Config, folder: &BackupFolder, rclone: Rclone) -> Result<(), String> {
    let others: Vec<_> = cfg
        .backup_folders
        .iter()
        .filter(|f| *f != folder)
        .cloned()
        .collect();
    let source = check_folder(cfg, folder, &others)?;
    // An empty folder is most likely a disk that isn't mounted right now; mirroring it would
    // empty the backup.
    if fs::read_dir(&source)
        .map_err(|_| "source-missing")?
        .next()
        .is_none()
    {
        return Err("source-empty".into());
    }
    let src = source.to_string_lossy();
    let rules = excludes(cfg, &source);
    let mut args = vec!["sync", "--create-empty-src-dirs", "--skip-links"];
    for r in &rules {
        args.extend(["--exclude", r.as_str()]);
    }
    args.extend(["--", &src, &folder.destination]);
    rclone(&args).map(|_| ()).map_err(|e| copy_code(&e))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Mirror every folder once, recording each outcome.
fn pass(cfg: &Config, state_path: &Path, lock_path: &Path, rclone: Rclone) -> Result<(), String> {
    let lock = lock_file(lock_path)?;
    lock.lock().map_err(|e| e.to_string())?;
    let mut state = read_status(state_path)?;
    // Forget folders that are no longer backed up.
    state
        .folders
        .retain(|k, _| cfg.backup_folders.iter().any(|f| &f.source == k));
    state.running = true;
    for folder in &cfg.backup_folders {
        state.current = Some(folder.source.clone());
        write_status(state_path, &state)?;
        let result = mirror(cfg, folder, rclone);
        let entry = state.folders.entry(folder.source.clone()).or_default();
        match result {
            Ok(()) => {
                entry.last_success = Some(now());
                entry.error = None;
            }
            Err(code) => entry.error = Some(code),
        }
    }
    state.running = false;
    state.current = None;
    write_status(state_path, &state)
}

/// Run by `<slug>-backup.service`: mirror every folder now, when the list changes, and every
/// `backup_interval_secs`. One keeper per identity.
///
/// # Errors
/// State or lock I/O failures (systemd restarts the unit).
pub fn run_keeper() -> Result<(), String> {
    let keeper_lock = lock_file(&lock_path("backup-keeper.lock"))?;
    keeper_lock.try_lock().map_err(|e| e.to_string())?;
    loop {
        let cfg = config::store().load_checked()?;
        if !cfg.backup_folders.is_empty() {
            pass(
                &cfg,
                &state_path(),
                &lock_path("backup-running.lock"),
                &rclone_cli,
            )?;
        }
        let mut waited = 0;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            waited += 2;
            let current = config::store().load();
            if current.backup_folders != cfg.backup_folders
                || waited >= current.backup_interval_secs.max(60)
            {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "backup-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(fs::canonicalize(&path).unwrap())
        }
        fn dir(&self, rel: &str) -> String {
            let p = self.0.join(rel);
            fs::create_dir_all(&p).unwrap();
            p.to_string_lossy().into_owned()
        }
        fn s(&self, rel: &str) -> String {
            self.0.join(rel).to_string_lossy().into_owned()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn cfg(t: &Temp) -> Config {
        Config {
            mount_path: t.s("mount"),
            cache_dir: t.s("cache"),
            ..Config::default()
        }
    }

    /// An rclone that answers `lsf` from `dirs` and records every call.
    struct FakeRclone {
        dirs: BTreeMap<String, Vec<&'static str>>,
        calls: RefCell<Vec<Vec<String>>>,
        fail: Option<RcloneError>,
    }
    impl FakeRclone {
        fn new() -> Self {
            Self {
                dirs: BTreeMap::new(),
                calls: RefCell::new(Vec::new()),
                fail: None,
            }
        }
        fn run(&self, args: &[&str]) -> Result<String, RcloneError> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|s| (*s).to_string()).collect());
            if args[0] == "lsf" {
                let dir = args.last().unwrap();
                return self.dirs.get(*dir).map_or_else(
                    || {
                        Err(RcloneError::Failed(
                            "error listing: directory not found".into(),
                        ))
                    },
                    |d| {
                        Ok(d.iter()
                            .map(|n| format!("{n}/"))
                            .collect::<Vec<_>>()
                            .join("\n"))
                    },
                );
            }
            self.fail.clone().map_or(Ok(String::new()), Err)
        }
    }

    #[test]
    fn a_new_backup_goes_under_computers_and_this_host() {
        let t = Temp::new();
        let docs = t.dir("home/Documents");
        let rc = FakeRclone::new();
        let f = plan_add(&cfg(&t), &docs, Some("gdrive:"), "nixie", &|a| rc.run(a)).unwrap();
        assert_eq!(f.destination, "gdrive:Computers/nixie/Documents");
        assert_eq!(f.source, docs);
    }

    #[test]
    fn a_name_already_on_the_remote_or_in_use_gets_a_number() {
        let t = Temp::new();
        let mut rc = FakeRclone::new();
        rc.dirs
            .insert("gdrive:Computers/nixie".into(), vec!["Documents"]);
        let mut c = cfg(&t);
        c.backup_folders.push(BackupFolder {
            source: t.dir("a/Documents"),
            destination: "gdrive:Computers/nixie/Documents (2)".into(),
        });
        let f = plan_add(&c, &t.dir("b/Documents"), Some("gdrive:"), "nixie", &|a| {
            rc.run(a)
        })
        .unwrap();
        assert_eq!(f.destination, "gdrive:Computers/nixie/Documents (3)");
    }

    #[test]
    fn folders_inside_the_mount_or_already_backed_up_are_refused() {
        let t = Temp::new();
        let rc = FakeRclone::new();
        let mut c = cfg(&t);
        let run = |a: &[&str]| rc.run(a);
        assert_eq!(
            plan_add(&c, &t.dir("mount/x"), Some("gdrive:"), "h", &run),
            Err("source-in-mount")
        );
        assert_eq!(
            plan_add(&c, &t.s("nope"), Some("gdrive:"), "h", &run),
            Err("source-missing")
        );
        assert_eq!(
            plan_add(&c, &t.dir("x"), None, "h", &run),
            Err("remote-unknown")
        );
        c.backup_folders.push(BackupFolder {
            source: t.dir("home"),
            destination: "gdrive:Computers/h/home".into(),
        });
        assert_eq!(
            plan_add(&c, &t.dir("home"), Some("gdrive:"), "h", &run),
            Err("already-backed-up")
        );
        assert_eq!(
            plan_add(&c, &t.dir("home/sub"), Some("gdrive:"), "h", &run),
            Err("source-overlap")
        );
        // With the mount down, the remote existing backups use is taken.
        let f = plan_add(&c, &t.dir("other"), None, "h", &run).unwrap();
        assert_eq!(f.destination, "gdrive:Computers/h/other");
    }

    #[test]
    fn a_symlink_into_the_mount_is_refused() {
        let t = Temp::new();
        t.dir("mount/folder");
        std::os::unix::fs::symlink(t.0.join("mount/folder"), t.0.join("link")).unwrap();
        let rc = FakeRclone::new();
        assert_eq!(
            plan_add(&cfg(&t), &t.s("link"), Some("gdrive:"), "h", &|a| rc.run(a)),
            Err("source-in-mount")
        );
    }

    #[test]
    fn mirroring_a_folder_that_holds_the_mount_leaves_the_mount_out() {
        let t = Temp::new();
        let home = t.dir("home");
        fs::write(t.0.join("home/notes.txt"), "x").unwrap();
        let mut c = cfg(&t);
        c.mount_path = t.dir("home/Google [Drive]");
        c.cache_dir = t.dir("home/.cache/rclone");
        let folder = BackupFolder {
            source: home.clone(),
            destination: "gdrive:Computers/h/home".into(),
        };
        c.backup_folders.push(folder.clone());
        let rc = FakeRclone::new();
        mirror(&c, &folder, &|a| rc.run(a)).unwrap();
        let call = rc.calls.borrow()[0].clone();
        assert_eq!(call[0], "sync");
        let excl: Vec<_> = call
            .windows(2)
            .filter(|w| w[0] == "--exclude")
            .map(|w| w[1].clone())
            .collect();
        assert!(
            excl.contains(&"/Google \\[Drive\\]/**".to_string()),
            "{excl:?}"
        );
        assert!(excl.contains(&"/.cache/rclone/**".to_string()), "{excl:?}");
        assert_eq!(
            &call[call.len() - 3..],
            ["--", home.as_str(), "gdrive:Computers/h/home"]
        );
    }

    #[test]
    fn an_empty_or_missing_folder_never_empties_its_backup() {
        let t = Temp::new();
        let folder = BackupFolder {
            source: t.dir("usb"),
            destination: "gdrive:Computers/h/usb".into(),
        };
        let rc = FakeRclone::new();
        assert_eq!(
            mirror(&cfg(&t), &folder, &|a| rc.run(a)),
            Err("source-empty".into())
        );
        fs::remove_dir(t.0.join("usb")).unwrap();
        assert_eq!(
            mirror(&cfg(&t), &folder, &|a| rc.run(a)),
            Err("source-missing".into())
        );
        assert!(rc.calls.borrow().is_empty());
    }

    #[test]
    fn each_folder_records_its_own_outcome_and_interruptions_show() {
        let t = Temp::new();
        let mut c = cfg(&t);
        for name in ["a", "b"] {
            t.dir(name);
            fs::write(t.0.join(name).join("f"), "x").unwrap();
            c.backup_folders.push(BackupFolder {
                source: t.s(name),
                destination: format!("gdrive:Computers/h/{name}"),
            });
        }
        let state = t.0.join("state.json");
        let lock = t.0.join("run.lock");
        let rc = FakeRclone {
            fail: Some(RcloneError::Failed("boom".into())),
            ..FakeRclone::new()
        };
        pass(&c, &state, &lock, &|a| {
            if a.last() == Some(&"gdrive:Computers/h/b") {
                rc.run(a)
            } else {
                Ok(String::new())
            }
        })
        .unwrap();
        let s = status_in(&state, &lock).unwrap();
        assert!(s.folders[&t.s("a")].last_success.is_some());
        assert_eq!(s.folders[&t.s("b")].error.as_deref(), Some("copy-failed"));

        let mut cut = s.clone();
        cut.running = true;
        cut.current = Some(t.s("a"));
        write_status(&state, &cut).unwrap();
        let s = status_in(&state, &lock).unwrap();
        assert!(!s.running);
        assert_eq!(s.folders[&t.s("a")].error.as_deref(), Some("interrupted"));
    }

    #[test]
    fn a_running_pass_shows_as_running() {
        use std::sync::{Arc, Barrier};
        let t = Temp::new();
        let mut c = cfg(&t);
        t.dir("src");
        fs::write(t.0.join("src/f"), "x").unwrap();
        c.backup_folders.push(BackupFolder {
            source: t.s("src"),
            destination: "gdrive:Computers/h/src".into(),
        });
        let state = t.0.join("state.json");
        let lock = t.0.join("run.lock");
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let worker = {
            let (entered, release) = (entered.clone(), release.clone());
            let (state, lock) = (state.clone(), lock.clone());
            std::thread::spawn(move || {
                pass(&c, &state, &lock, &|_| {
                    entered.wait();
                    release.wait();
                    Ok(String::new())
                })
                .unwrap();
            })
        };
        entered.wait();
        assert!(status_in(&state, &lock).unwrap().running);
        release.wait();
        worker.join().unwrap();
        assert!(!status_in(&state, &lock).unwrap().running);
    }

    #[test]
    fn remote_backups_are_listed_per_machine() {
        let mut rc = FakeRclone::new();
        rc.dirs
            .insert("gdrive:Computers".into(), vec!["nixie", "work"]);
        rc.dirs
            .insert("gdrive:Computers/nixie".into(), vec!["Documents", "Photos"]);
        rc.dirs.insert("gdrive:Computers/work".into(), vec!["src"]);
        let list = list_remote_in("gdrive", &|a| rc.run(a)).unwrap();
        let names: Vec<_> = list
            .iter()
            .map(|b| format!("{}/{}", b.host, b.name))
            .collect();
        assert_eq!(names, ["nixie/Documents", "nixie/Photos", "work/src"]);
        assert_eq!(list[2].destination, "gdrive:Computers/work/src");
        // No Computers folder yet: nothing to restore, not an error.
        let empty = FakeRclone::new();
        assert_eq!(list_remote_in("gdrive", &|a| empty.run(a)), Ok(vec![]));
    }

    #[test]
    fn restore_targets_follow_the_backup_rules_when_linked() {
        let t = Temp::new();
        let mut c = cfg(&t);
        let dest = "gdrive:Computers/old/Documents";
        assert_eq!(
            plan_restore(&c, dest, &t.s("mount/x"), false),
            Err("source-in-mount")
        );
        let f = plan_restore(&c, dest, &t.s("new/Documents"), true).unwrap();
        assert_eq!(f.source, t.s("new/Documents"));
        c.backup_folders.push(BackupFolder {
            source: t.dir("elsewhere"),
            destination: dest.into(),
        });
        assert_eq!(
            plan_restore(&c, dest, &t.s("new/Documents"), true),
            Err("destination-overlap")
        );
        // A plain restore of a folder that is still backed up elsewhere is fine.
        assert!(plan_restore(&c, dest, &t.s("new/Documents"), false).is_ok());
        assert_eq!(
            plan_restore(&c, "gdrive:Computers/x/y", &t.s("elsewhere/in"), true),
            Err("already-backed-up")
        );
    }

    #[test]
    fn config_round_trips_and_old_files_have_no_backups() {
        let t = Temp::new();
        let mut c = cfg(&t);
        c.backup_folders.push(BackupFolder {
            source: "/home/u/Documents".into(),
            destination: "gdrive:Computers/h/Documents".into(),
        });
        let text = toml::to_string(&c).unwrap();
        assert_eq!(config::parse_toml(&text), c);
        let old = config::parse_toml("service = 'existing.service'");
        assert_eq!(old.backup_folders, Vec::<BackupFolder>::new());
        assert_eq!(old.backup_interval_secs, 3600);
    }

    /// The real `rclone` binary against a throwaway local remote (`testr:`, defined through the
    /// environment of each call only): deletions are mirrored, the mount inside the folder is
    /// left out, and an emptied folder never empties its backup.
    #[test]
    #[ignore = "runs the installed rclone; `just test-live`"]
    fn real_rclone_backup_mirrors_and_leaves_the_mount_out() {
        let t = Temp::new();
        let remote_root = t.dir("remote");
        let rclone = |args: &[&str]| -> Result<String, RcloneError> {
            // The local remote's relative paths resolve against the working directory.
            let out = Command::new("rclone")
                .current_dir(&remote_root)
                .env("RCLONE_CONFIG", "/dev/null")
                .env("RCLONE_CONFIG_TESTR_TYPE", "local")
                .args(args)
                .output()
                .map_err(|_| RcloneError::Unavailable)?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            } else {
                Err(RcloneError::Failed(
                    String::from_utf8_lossy(&out.stderr).into_owned(),
                ))
            }
        };
        let home = t.dir("home");
        fs::write(t.0.join("home/keep.txt"), "keep").unwrap();
        fs::write(t.0.join("home/gone.txt"), "gone").unwrap();
        t.dir("home/Drive");
        fs::write(t.0.join("home/Drive/cloud.txt"), "cloud").unwrap();
        let mut c = cfg(&t);
        c.mount_path = t.s("home/Drive");
        let dest = "testr:Computers/h/home".to_string();
        let folder = BackupFolder {
            source: home.clone(),
            destination: dest.clone(),
        };
        c.backup_folders.push(folder.clone());
        let backup = t.0.join("remote/Computers/h/home");

        mirror(&c, &folder, &rclone).unwrap();
        assert!(backup.join("gone.txt").exists());
        assert!(!backup.join("Drive").exists(), "the mount was backed up");

        fs::remove_file(t.0.join("home/gone.txt")).unwrap();
        mirror(&c, &folder, &rclone).unwrap();
        assert!(!backup.join("gone.txt").exists(), "deletion not mirrored");
        assert_eq!(fs::read_to_string(backup.join("keep.txt")).unwrap(), "keep");

        // Emptied (the mount gone too): refused, and the backup keeps its last copy.
        fs::remove_file(t.0.join("home/keep.txt")).unwrap();
        fs::remove_dir_all(t.0.join("home/Drive")).unwrap();
        assert_eq!(mirror(&c, &folder, &rclone), Err("source-empty".into()));
        assert!(backup.join("keep.txt").exists());

        // A restore is a copy back to any folder.
        let target = t.s("restored");
        rclone(&["copy", "--create-empty-src-dirs", "--", &dest, &target]).unwrap();
        assert_eq!(
            fs::read_to_string(t.0.join("restored/keep.txt")).unwrap(),
            "keep"
        );
        let listed = list_remote_in("testr", &rclone).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].host, "h");
        assert_eq!(listed[0].name, "home");
    }

    #[test]
    fn units_are_owned_and_removal_preserves_unrelated_files() {
        let t = Temp::new();
        let name = "backup.service";
        let calls = RefCell::new(Vec::new());
        let runner = |args: &[&str]| {
            calls.borrow_mut().push(args.join(" "));
            Ok(())
        };
        fs::write(t.0.join(name), "foreign").unwrap();
        assert!(install_in(&t.0, Path::new("/bin/test"), name, &runner).is_err());
        assert!(remove_in(&t.0, name, &runner).is_err());
        fs::remove_file(t.0.join(name)).unwrap();
        install_in(&t.0, Path::new("/bin/test"), name, &runner).unwrap();
        remove_in(&t.0, name, &runner).unwrap();
        assert_eq!(
            calls.borrow().as_slice(),
            [
                "daemon-reload",
                "enable --now backup.service",
                "disable --now backup.service",
                "daemon-reload"
            ]
        );
    }
}
