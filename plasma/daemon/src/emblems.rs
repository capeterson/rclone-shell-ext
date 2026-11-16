// SPDX-License-Identifier: GPL-3.0-only
//! Overlay emblems for `FileStates`: the `SyncState` mapping, pin membership, and the bounded
//! path cache the daemon answers from.

use crate::recheck::FolderThrottle;
use rclone_ui_core::config::{expand, Config};
use rclone_ui_core::pin::{self, OfflineState, Phase};
use rclone_ui_core::rc;
use rclone_ui_core::share::{self, Provider, Target};
use rclone_ui_core::status::{self, Cached, Local, SyncState};
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const CACHE_CAP: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMark {
    None,
    Offline,
    Attention,
}

pub fn sync_emblem(s: SyncState) -> String {
    use rclone_ui_core::identity::emblem_icon as emblem;
    match s {
        SyncState::InSync => emblem("synced"),
        SyncState::OnlineOnly => emblem("online"),
        SyncState::PartlyCached => emblem("partial"),
        SyncState::Uploading | SyncState::QueuedUpload | SyncState::LocalNewer => {
            emblem("uploading")
        }
        SyncState::Downloading | SyncState::DriveNewer => emblem("downloading"),
        SyncState::Differs => "emblem-warning".into(),
    }
}

pub fn emblem_list(sync: Option<SyncState>, pin: PinMark) -> Vec<String> {
    let mut v: Vec<String> = sync.map(sync_emblem).into_iter().collect();
    match pin {
        PinMark::None => {}
        PinMark::Offline => v.push(rclone_ui_core::identity::emblem_icon("offline")),
        PinMark::Attention => v.push("emblem-error".into()),
    }
    v
}

/// Whether `rel` is in (or is) a pinned folder, and whether that pin needs attention.
pub fn pin_mark(cfg: &Config, offline: &OfflineState, rel: &str) -> PinMark {
    if !cfg.offline_enabled {
        return PinMark::None;
    }
    match pin::covering_pin(&cfg.pins, rel) {
        None => PinMark::None,
        Some(p) if offline.pin(p).is_some_and(|s| s.phase == Phase::Attention) => {
            PinMark::Attention
        }
        Some(_) => PinMark::Offline,
    }
}

/// The path relative to the mount, if `path` is inside it (not the mount itself).
pub fn rel_in_mount(cfg: &Config, path: &str) -> Option<String> {
    let mount = expand(&cfg.mount_path);
    share::remote_path(&mount, Path::new(path)).filter(|r| !r.is_empty())
}

pub enum Outcome {
    Ready(Vec<String>),
    /// Couldn't tell right now (not mounted, rc unreachable); try again later.
    Retry,
    /// A folder computed too recently; try again at this time.
    Defer(Instant),
}

/// The VFS metadata file for `rel`: `<cache>/vfsMeta/<remote name>/<rel>`.
fn meta_path(cfg: &Config, fs: &str, rel: &str) -> PathBuf {
    // `gdrive:` -> `gdrive`; `gdrive:sub/dir` -> `gdrive/sub/dir`.
    let name = fs.replacen(':', "/", 1);
    Path::new(&expand(&cfg.cache_dir))
        .join("vfsMeta")
        .join(name.trim_end_matches('/'))
        .join(rel)
}

/// Parse one `vfsMeta` file the way core does (its parser is private).
fn parse_meta(text: &str) -> Option<Cached> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let size = v
        .get("Size")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let size = u64::try_from(size).unwrap_or(0);
    let have: u64 = v
        .get("Rs")
        .and_then(serde_json::Value::as_array)
        .map_or(0, |rs| {
            rs.iter()
                .filter_map(|r| r.get("Size").and_then(serde_json::Value::as_i64))
                .map(|n| u64::try_from(n).unwrap_or(0))
                .sum()
        })
        .min(size);
    Some(Cached {
        size,
        have,
        mtime: v
            .get("ModTime")
            .and_then(serde_json::Value::as_str)
            .and_then(|t| t.parse().ok()),
        dirty: v
            .get("Dirty")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

/// Names of the files rclone is transferring right now (one `core/stats` call).
fn running_names(cfg: &Config) -> Option<HashSet<String>> {
    let stats = rc::rc_request(cfg, "core/stats", &serde_json::json!({})).ok()?;
    Some(
        stats
            .get("transferring")
            .and_then(serde_json::Value::as_array)
            .map_or_else(HashSet::new, |a| {
                a.iter()
                    .filter_map(|t| t.get("name").and_then(serde_json::Value::as_str))
                    .map(str::to_string)
                    .collect()
            }),
    )
}

fn folder_sync(sum: &status::FolderSummary) -> Option<SyncState> {
    if sum.uploading > 0 {
        Some(SyncState::Uploading)
    } else if sum.downloading > 0 {
        Some(SyncState::Downloading)
    } else {
        None
    }
}

/// Work out the emblems for a batch of absolute paths. Blocking: reads cache metadata and
/// asks rclone's local rc API about running transfers once for the whole batch (and only if
/// some path needs it). Files read their VFS metadata directly. Folders walk the cache tree,
/// so with nothing transferring they get no sync emblem without a walk, and otherwise are
/// recomputed at most once per `FOLDER_MIN_INTERVAL` (`Outcome::Defer`).
pub fn compute_batch(
    cfg: &Config,
    offline: &OfflineState,
    paths: &[String],
    throttle: &mut FolderThrottle,
    now: Instant,
) -> Vec<(String, Outcome)> {
    // `None`: not fetched yet; `Some(None)`: the fetch failed.
    let running = OnceCell::new();
    let fs = OnceCell::new();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let o = compute_one(cfg, offline, path, &running, &fs, throttle, now);
        out.push((path.clone(), o));
    }
    out
}

fn compute_one(
    cfg: &Config,
    offline: &OfflineState,
    path: &str,
    running: &OnceCell<Option<HashSet<String>>>,
    fs: &OnceCell<Option<String>>,
    throttle: &mut FolderThrottle,
    now: Instant,
) -> Outcome {
    let Some(rel) = rel_in_mount(cfg, path) else {
        return Outcome::Ready(Vec::new());
    };
    let mark = pin_mark(cfg, offline, &rel);
    let Ok(meta) = std::fs::metadata(path) else {
        // Gone, or the mount isn't answering.
        return if rc::is_mounted(&cfg.mount_path) {
            Outcome::Ready(Vec::new())
        } else {
            Outcome::Retry
        };
    };
    let Some(fs) = fs.get_or_init(|| rc::mount_fs(&cfg.mount_path)).clone() else {
        return Outcome::Retry;
    };
    let Some(names) = running.get_or_init(|| running_names(cfg)).as_ref() else {
        return Outcome::Retry;
    };
    // Offline-folder activity is worked out inside core; defer to it for those paths.
    let in_pin = cfg.offline_enabled && pin::covering_pin(&cfg.pins, &rel).is_some();
    if meta.is_dir() && names.is_empty() && !in_pin {
        return Outcome::Ready(emblem_list(None, mark));
    }
    if meta.is_dir() || in_pin {
        if meta.is_dir() {
            if let Err(at) = throttle.admit(path, now) {
                return Outcome::Defer(at);
            }
        }
        let target = Target {
            provider: Provider::GoogleDrive,
            id: String::new(),
            is_dir: meta.is_dir(),
            fs,
            rel,
        };
        let Ok(local) = status::local(cfg, &target) else {
            return Outcome::Retry;
        };
        let sync = match local {
            Local::File {
                transferring,
                cached,
            } => {
                status::file_state(transferring, cached.as_ref(), None).or(Some(SyncState::InSync))
            }
            Local::Folder(sum) => folder_sync(&sum),
        };
        return Outcome::Ready(emblem_list(sync, mark));
    }
    let cached = std::fs::read_to_string(meta_path(cfg, &fs, &rel))
        .ok()
        .and_then(|t| parse_meta(&t));
    let sync =
        status::file_state(names.contains(&rel), cached.as_ref(), None).or(Some(SyncState::InSync));
    Outcome::Ready(emblem_list(sync, mark))
}

/// Whether two mount-relative paths are the same, or one contains the other.
pub fn related(a: &str, b: &str) -> bool {
    a == b
        || a.strip_prefix(b).is_some_and(|r| r.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|r| r.starts_with('/'))
}

/// Cached absolute paths whose mount-relative name is related to one of `names`.
pub fn affected(cfg: &Config, cached: &[String], names: &HashSet<String>) -> Vec<String> {
    cached
        .iter()
        .filter(|p| {
            rel_in_mount(cfg, p)
                .is_some_and(|r| names.iter().any(|n| related(&r, n.trim_matches('/'))))
        })
        .cloned()
        .collect()
}

/// path -> emblems, bounded, plus the set of paths waiting for the worker.
#[derive(Default)]
pub struct EmblemCache {
    map: HashMap<String, Vec<String>>,
    order: VecDeque<String>,
    queued: HashSet<String>,
    missed: HashSet<String>,
}

impl EmblemCache {
    pub fn get(&self, path: &str) -> Option<&Vec<String>> {
        self.map.get(path)
    }

    /// Store `emblems`, returning whether the entry changed (a new entry counts as changed
    /// unless it is empty, since an empty answer is what a miss returned).
    pub fn insert(&mut self, path: &str, emblems: Vec<String>) -> bool {
        self.queued.remove(path);
        self.missed.remove(path);
        if let Some(old) = self.map.get_mut(path) {
            let changed = *old != emblems;
            *old = emblems;
            return changed;
        }
        let changed = !emblems.is_empty();
        if self.map.len() >= CACHE_CAP {
            while let Some(k) = self.order.pop_front() {
                if self.map.remove(&k).is_some() {
                    break;
                }
            }
        }
        self.map.insert(path.to_string(), emblems);
        self.order.push_back(path.to_string());
        changed
    }

    /// Mark `path` as waiting; false if it already was.
    pub fn queue(&mut self, path: &str) -> bool {
        self.queued.insert(path.to_string())
    }

    pub fn unqueue(&mut self, path: &str) {
        self.queued.remove(path);
    }

    /// Remember a path the worker couldn't answer yet.
    pub fn miss(&mut self, path: &str) {
        self.queued.remove(path);
        if self.missed.len() < 1000 {
            self.missed.insert(path.to_string());
        }
    }

    pub fn take_missed(&mut self) -> Vec<String> {
        self.missed.drain().collect()
    }

    pub fn paths(&self) -> Vec<String> {
        self.map.keys().cloned().collect()
    }

    pub fn missed_paths(&self) -> Vec<String> {
        self.missed.iter().cloned().collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rclone_ui_core::pin::PinState;

    #[test]
    fn maps_every_sync_state() {
        use SyncState::*;
        let want = [
            (InSync, "rclone-shell-ext-emblem-synced"),
            (OnlineOnly, "rclone-shell-ext-emblem-online"),
            (PartlyCached, "rclone-shell-ext-emblem-partial"),
            (Uploading, "rclone-shell-ext-emblem-uploading"),
            (QueuedUpload, "rclone-shell-ext-emblem-uploading"),
            (LocalNewer, "rclone-shell-ext-emblem-uploading"),
            (Downloading, "rclone-shell-ext-emblem-downloading"),
            (DriveNewer, "rclone-shell-ext-emblem-downloading"),
            (Differs, "emblem-warning"),
        ];
        for (s, e) in want {
            assert_eq!(emblem_list(Some(s), PinMark::None), vec![e.to_string()]);
        }
        assert_eq!(
            emblem_list(Some(InSync), PinMark::Offline),
            [
                "rclone-shell-ext-emblem-synced",
                "rclone-shell-ext-emblem-offline"
            ]
        );
        assert_eq!(emblem_list(None, PinMark::Attention), ["emblem-error"]);
        assert_eq!(emblem_list(None, PinMark::None).len(), 0);
    }

    #[test]
    fn pin_membership() {
        let cfg = Config {
            offline_enabled: true,
            pins: vec!["Work".into(), "Bad".into()],
            ..Config::default()
        };
        let mut off = OfflineState::default();
        off.pins.push(PinState {
            rel: "Bad".into(),
            phase: Phase::Attention,
            ..PinState::default()
        });
        assert_eq!(pin_mark(&cfg, &off, "Work"), PinMark::Offline);
        assert_eq!(pin_mark(&cfg, &off, "Work/a/b"), PinMark::Offline);
        assert_eq!(pin_mark(&cfg, &off, "Workshop"), PinMark::None);
        assert_eq!(pin_mark(&cfg, &off, "Bad/x"), PinMark::Attention);
        let off_cfg = Config {
            offline_enabled: false,
            ..cfg
        };
        assert_eq!(pin_mark(&off_cfg, &off, "Work"), PinMark::None);
    }

    #[test]
    fn parses_vfs_meta() {
        let c = parse_meta(
            r#"{"Size":100,"ModTime":"2026-01-02T03:04:05Z","Dirty":true,"Rs":[{"Pos":0,"Size":40},{"Pos":50,"Size":30}]}"#,
        )
        .unwrap();
        assert_eq!((c.size, c.have, c.dirty), (100, 70, true));
        assert!(c.mtime.is_some());
        let d = parse_meta(r#"{"Size":10,"Rs":[{"Size":999}]}"#).unwrap();
        assert_eq!((d.have, d.dirty, d.mtime), (10, false, None));
        assert!(parse_meta("garbage").is_none());
    }

    #[test]
    fn mount_paths_and_relations() {
        let cfg = Config {
            mount_path: "/mnt/d".into(),
            ..Config::default()
        };
        assert_eq!(rel_in_mount(&cfg, "/mnt/d/a/b").as_deref(), Some("a/b"));
        assert_eq!(rel_in_mount(&cfg, "/mnt/d"), None);
        assert_eq!(rel_in_mount(&cfg, "/other/a"), None);
        assert_eq!(rel_in_mount(&cfg, "/mnt/d/../x"), None);
        assert!(related("a/b", "a") && related("a", "a/b") && related("a", "a"));
        assert!(!related("ab", "a"));
        let names: HashSet<String> = ["a/b.txt".to_string()].into();
        let cached = vec![
            "/mnt/d/a".to_string(),
            "/mnt/d/a/b.txt".to_string(),
            "/mnt/d/c".to_string(),
        ];
        assert_eq!(affected(&cfg, &cached, &names), cached[..2]);
    }

    #[test]
    fn cache_changes_and_bound() {
        let mut c = EmblemCache::default();
        assert!(c.queue("/p"));
        assert!(!c.queue("/p"));
        assert!(!c.insert("/p", vec![]), "empty new entry is not a change");
        assert!(c.queue("/p"), "insert clears the queued mark");
        assert!(c.insert("/p", vec!["x".into()]));
        assert!(!c.insert("/p", vec!["x".into()]));
        assert!(c.insert("/p", vec![]));
        for i in 0..CACHE_CAP + 50 {
            c.insert(&format!("/q{i}"), vec!["e".into()]);
        }
        assert_eq!(c.len(), CACHE_CAP);
        assert!(c.get("/p").is_none() && c.get(&format!("/q{}", CACHE_CAP + 49)).is_some());
        c.miss("/m");
        assert_eq!(c.take_missed(), ["/m"]);
    }
}
