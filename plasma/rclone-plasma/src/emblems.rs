// SPDX-License-Identifier: GPL-3.0-only
//! Overlay emblems for `FileStates`: the `SyncState` mapping, pin membership, and the bounded
//! path cache the daemon answers from.

use rclone_ui_core::config::{expand, Config};
use rclone_ui_core::pin::{self, OfflineState, Phase};
use rclone_ui_core::rc;
use rclone_ui_core::share::{self, Provider, Target};
use rclone_ui_core::status::{self, Local, SyncState};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

pub const CACHE_CAP: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMark {
    None,
    Offline,
    Attention,
}

pub fn sync_emblem(s: SyncState) -> &'static str {
    match s {
        SyncState::InSync => "rclone-plasma-emblem-synced",
        SyncState::OnlineOnly => "rclone-plasma-emblem-online",
        SyncState::PartlyCached => "rclone-plasma-emblem-partial",
        SyncState::Uploading | SyncState::QueuedUpload | SyncState::LocalNewer => {
            "rclone-plasma-emblem-uploading"
        }
        SyncState::Downloading | SyncState::DriveNewer => "rclone-plasma-emblem-downloading",
        SyncState::Differs => "emblem-warning",
    }
}

pub fn emblem_list(sync: Option<SyncState>, pin: PinMark) -> Vec<String> {
    let mut v: Vec<String> = sync
        .map(|s| sync_emblem(s).to_string())
        .into_iter()
        .collect();
    match pin {
        PinMark::None => {}
        PinMark::Offline => v.push("rclone-plasma-emblem-offline".into()),
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
}

/// Work out the emblems for one absolute path. Blocking: reads the cache metadata and asks
/// rclone's local rc API about running transfers.
pub fn compute(cfg: &Config, offline: &OfflineState, path: &str) -> Outcome {
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
    let Some(fs) = rc::mount_fs(&cfg.mount_path) else {
        return Outcome::Retry;
    };
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
        } => status::file_state(transferring, cached.as_ref(), None).or(Some(SyncState::InSync)),
        Local::Folder(sum) => {
            if sum.uploading > 0 {
                Some(SyncState::Uploading)
            } else if sum.downloading > 0 {
                Some(SyncState::Downloading)
            } else {
                None
            }
        }
    };
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
            (InSync, "rclone-plasma-emblem-synced"),
            (OnlineOnly, "rclone-plasma-emblem-online"),
            (PartlyCached, "rclone-plasma-emblem-partial"),
            (Uploading, "rclone-plasma-emblem-uploading"),
            (QueuedUpload, "rclone-plasma-emblem-uploading"),
            (LocalNewer, "rclone-plasma-emblem-uploading"),
            (Downloading, "rclone-plasma-emblem-downloading"),
            (DriveNewer, "rclone-plasma-emblem-downloading"),
            (Differs, "emblem-warning"),
        ];
        for (s, e) in want {
            assert_eq!(emblem_list(Some(s), PinMark::None), vec![e.to_string()]);
        }
        assert_eq!(
            emblem_list(Some(InSync), PinMark::Offline),
            [
                "rclone-plasma-emblem-synced",
                "rclone-plasma-emblem-offline"
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
