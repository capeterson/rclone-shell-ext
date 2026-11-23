// SPDX-License-Identifier: GPL-3.0-only
//! The D-Bus service (`rclone-shell-ext-plasma --daemon`). See `docs/dbus-interface.md`.
//!
//! Two worker threads feed shared state: the poller (rclone status every 2 s, offline state,
//! history debounce, property updates, cache invalidation) and the emblem worker (answers
//! `FileStates` cache misses in the background). The bus is served by zbus's own thread.

use crate::emblems::{self, EmblemCache, Outcome};
use crate::recheck::{self, FolderThrottle, SettleQueue};
use crate::{setconfig, snapshot};
use rclone_ui_core::config::{expand, store, Config};
use rclone_ui_core::pin::OfflineState;
use rclone_ui_core::rc::{self, CompletedTransfer};
use rclone_ui_view::history::History;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};
use zbus::blocking::{connection, Connection};
use zbus::fdo::RequestNameFlags;
use zbus::zvariant::Value;

use rclone_ui_core::identity::{identity, DBUS_INTERFACE as IFACE, DBUS_PATH as PATH};
const POLL: Duration = Duration::from_secs(2);
const WORKER_BATCH: usize = 64;

struct Inner {
    health: String,
    status_json: String,
    mount_path: String,
    cfg: Config,
    offline: OfflineState,
    cache: EmblemCache,
}

struct Shared {
    inner: Mutex<Inner>,
    conn: OnceLock<Connection>,
    tx: Mutex<Sender<String>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit_items(&self, paths: &[String]) {
        if paths.is_empty() {
            return;
        }
        if let Some(c) = self.conn.get() {
            let _ = c.emit_signal(None::<&str>, PATH, IFACE, "ItemsChanged", &(paths,));
        }
    }

    fn emit_properties(&self, changed: HashMap<&str, Value<'_>>) {
        if changed.is_empty() {
            return;
        }
        if let Some(c) = self.conn.get() {
            let _ = c.emit_signal(
                None::<&str>,
                PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(IFACE, changed, Vec::<&str>::new()),
            );
        }
    }

    /// Hand paths to the emblem worker (each at most once while it is waiting).
    fn enqueue(&self, paths: Vec<String>) {
        let fresh: Vec<String> = {
            let mut inner = self.lock();
            paths.into_iter().filter(|p| inner.cache.queue(p)).collect()
        };
        let tx = self.tx.lock().unwrap_or_else(PoisonError::into_inner);
        for p in fresh {
            if tx.send(p.clone()).is_err() {
                self.lock().cache.unqueue(&p);
            }
        }
    }
}

/// Absolute, `~`-expanded mount path without a trailing slash.
fn mount_dir(cfg: &Config) -> String {
    expand(&cfg.mount_path).trim_end_matches('/').to_string()
}

struct Service(Arc<Shared>);

#[allow(clippy::unused_self)]
#[zbus::interface(name = "com.ca_peterson.RcloneShellExt1")]
impl Service {
    #[zbus(property)]
    fn health(&self) -> String {
        self.0.lock().health.clone()
    }

    #[zbus(property)]
    fn status_json(&self) -> String {
        self.0.lock().status_json.clone()
    }

    #[zbus(property)]
    fn mount_path(&self) -> String {
        self.0.lock().mount_path.clone()
    }

    fn service_action(&self, verb: &str) -> bool {
        let cfg = self.0.lock().cfg.clone();
        let owned = verb.to_string();
        start_service_action(verb, move || {
            rc::service_action(&cfg, &owned);
        })
    }

    fn open_folder(&self) {
        let cfg = self.0.lock().cfg.clone();
        rc::open_mount(&cfg);
    }

    fn open_details(&self, path: &str) {
        if path.starts_with('/') {
            spawn_self(&["--menu", path]);
        } else {
            let cfg = self.0.lock().cfg.clone();
            rc::open_details(&cfg, path);
        }
    }

    fn open_settings(&self) {
        spawn_self(&["--settings"]);
    }

    /// Every argument-free UI message in the daemon's language, for the plasmoid and the Dolphin
    /// plugins, which cannot load the Fluent files themselves.
    fn strings(&self) -> BTreeMap<String, String> {
        rclone_ui_i18n::static_messages()
    }

    /// Answers from the cache only; misses return `[]` and are computed in the background.
    fn file_states(&self, paths: Vec<String>) -> HashMap<String, Vec<String>> {
        let mut out = HashMap::with_capacity(paths.len());
        let mut misses = Vec::new();
        {
            let inner = self.0.lock();
            for p in paths {
                if let Some(e) = inner.cache.get(&p) {
                    out.insert(p, e.clone());
                } else {
                    if emblems::rel_in_mount(&inner.cfg, &p).is_some() {
                        misses.push(p.clone());
                    }
                    out.insert(p, Vec::new());
                }
            }
        }
        if !misses.is_empty() {
            self.0.enqueue(misses);
        }
        out
    }

    /// Declared for introspection; emitted by `Shared::emit_items`.
    #[zbus(signal)]
    async fn items_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        paths: Vec<String>,
    ) -> zbus::Result<()>;

    fn get_config(&self) -> String {
        setconfig::get_json(store().load_checked())
    }

    fn set_config(&self, json: &str) -> String {
        setconfig::set(json)
    }
}

/// Run this executable with `args`, reaping the child in the background.
/// Accept `verb` and run `run` on its own thread: `systemctl` can take seconds, and a D-Bus
/// method handler must never block on it. Returns whether the verb was accepted, not whether
/// systemctl succeeded (the next status poll shows the outcome).
fn start_service_action(verb: &str, run: impl FnOnce() + Send + 'static) -> bool {
    if !matches!(verb, "restart" | "stop" | "start") {
        return false;
    }
    std::thread::spawn(run);
    true
}

fn spawn_self(args: &[&str]) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Ok(mut child) = Command::new(exe).args(args).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[derive(Default)]
struct Memory {
    mounted: bool,
    transfer_names: HashSet<String>,
    recent_keys: HashSet<(String, Option<String>)>,
    sig: Option<String>,
    settle: SettleQueue,
    sweep_cursor: usize,
    last_sweep: Option<Instant>,
}

fn recent_keys(recent: &[CompletedTransfer]) -> HashSet<(String, Option<String>)> {
    recent
        .iter()
        .map(|r| (r.name.clone(), r.at.map(|t| t.to_string())))
        .collect()
}

/// Everything that changes which emblems a path gets, other than transfers.
fn state_signature(cfg: &Config, offline: &OfflineState) -> String {
    let pins: Vec<String> = offline
        .pins
        .iter()
        .map(|p| format!("{}:{:?}:{}", p.rel, p.phase, p.syncing))
        .collect();
    format!(
        "{}|{}|{:?}|{}",
        cfg.mount_path,
        cfg.offline_enabled,
        cfg.pins,
        pins.join(",")
    )
}

fn poll_once(shared: &Shared, mem: &mut Memory, history: &mut History) {
    let cfg = store().load();
    let (st, offline) = snapshot::poll(&cfg);
    history.update(&st.recent, Instant::now());
    let doc = snapshot::build(&st, history.shown(), &offline, &cfg);
    let status_json = serde_json::to_string(&doc).unwrap_or_default();
    let mount_path = mount_dir(&cfg);
    let health = doc.health.clone();

    let sig = state_signature(&cfg, &offline);
    let names: HashSet<String> = st.transfers.iter().map(|t| t.name.clone()).collect();
    let keys = recent_keys(&st.recent);
    let mut changed_names: HashSet<String> = names
        .symmetric_difference(&mem.transfer_names)
        .cloned()
        .collect();
    changed_names.extend(keys.difference(&mem.recent_keys).map(|(n, _)| n.clone()));

    let (cached, missed, changed_props) = {
        let mut inner = shared.lock();
        let mut props: Vec<&'static str> = Vec::new();
        if inner.health != health {
            inner.health = health;
            props.push("Health");
        }
        if inner.status_json != status_json {
            inner.status_json = status_json;
            props.push("StatusJson");
        }
        if inner.mount_path != mount_path {
            inner.mount_path = mount_path;
            props.push("MountPath");
        }
        inner.cfg = cfg.clone();
        inner.offline = offline;
        (inner.cache.paths(), inner.cache.missed_paths(), props)
    };
    if !changed_props.is_empty() {
        let (h, s, m) = {
            let i = shared.lock();
            (
                i.health.clone(),
                i.status_json.clone(),
                i.mount_path.clone(),
            )
        };
        let mut map: HashMap<&str, Value<'_>> = HashMap::new();
        for p in &changed_props {
            let v = match *p {
                "Health" => &h,
                "StatusJson" => &s,
                _ => &m,
            };
            map.insert(p, Value::from(v.as_str()));
        }
        shared.emit_properties(map);
    }

    let now = Instant::now();
    let remounted = st.mounted && !mem.mounted;
    if remounted {
        let mut all = cached.clone();
        all.extend(missed);
        shared.emit_items(&all);
        shared.enqueue(cached);
    } else if mem.sig.as_ref().is_some_and(|s| *s != sig) {
        shared.enqueue(cached);
    } else {
        let mut recheck: Vec<String> = Vec::new();
        if !changed_names.is_empty() {
            // Transfer activity: recompute now, and again once rclone has settled its
            // metadata (it is written around, or after, the end of a transfer).
            let hit = emblems::affected(&cfg, &cached, &changed_names);
            mem.settle.schedule(&hit, now);
            recheck.extend(hit);
        }
        let due = mem.settle.drain_due(now);
        if !due.is_empty() {
            let known: HashSet<&String> = cached.iter().collect();
            recheck.extend(due.into_iter().filter(|p| known.contains(p)));
        }
        if st.mounted
            && mem
                .last_sweep
                .is_none_or(|t| now.duration_since(t) >= recheck::SWEEP_EVERY)
        {
            mem.last_sweep = Some(now);
            let mut all = cached;
            recheck.extend(recheck::round_robin(
                &mut all,
                &mut mem.sweep_cursor,
                recheck::SWEEP_BATCH,
            ));
        }
        shared.enqueue(recheck);
    }
    if st.mounted && st.rc_error.is_none() {
        let retry = shared.lock().cache.take_missed();
        shared.enqueue(retry);
    }

    mem.mounted = st.mounted;
    mem.transfer_names = names;
    mem.recent_keys = keys;
    mem.sig = Some(sig);
}

fn poller(shared: &Arc<Shared>) {
    let (mut mem, mut history) = (Memory::default(), History::default());
    // One poll at a time: the next starts `POLL` after the previous finished.
    loop {
        poll_once(shared, &mut mem, &mut history);
        std::thread::sleep(POLL);
    }
}

fn worker(shared: &Arc<Shared>, rx: &Receiver<String>) {
    let mut throttle = FolderThrottle::default();
    // Folders computed too recently, with the time they may run again.
    let mut deferred: Vec<(Instant, String)> = Vec::new();
    loop {
        let mut batch: Vec<String> = Vec::new();
        if deferred.is_empty() {
            match rx.recv() {
                Ok(p) => batch.push(p),
                Err(_) => return,
            }
        } else {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(p) => batch.push(p),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        while batch.len() < WORKER_BATCH {
            match rx.try_recv() {
                Ok(p) => batch.push(p),
                Err(_) => break,
            }
        }
        let now = Instant::now();
        deferred.retain(|(at, p)| {
            if *at <= now && !batch.contains(p) && batch.len() < WORKER_BATCH {
                batch.push(p.clone());
                false
            } else {
                *at > now && !batch.contains(p)
            }
        });
        if batch.is_empty() {
            continue;
        }
        let (cfg, offline) = {
            let i = shared.lock();
            (i.cfg.clone(), i.offline.clone())
        };
        let results = emblems::compute_batch(&cfg, &offline, &batch, &mut throttle, now);
        let mut changed = Vec::new();
        let mut inner = shared.lock();
        for (p, outcome) in results {
            match outcome {
                Outcome::Ready(e) => {
                    if inner.cache.insert(&p, e) {
                        changed.push(p);
                    }
                }
                Outcome::Retry => inner.cache.miss(&p),
                Outcome::Defer(at) => deferred.push((at, p)),
            }
        }
        drop(inner);
        shared.emit_items(&changed);
    }
}

pub fn run() -> i32 {
    rclone_ui_i18n::init();
    // The dev build owns a `-dev` name, so a dev and a release daemon don't clash.
    let name = identity().dbus_name();
    let (tx, rx) = mpsc::channel();
    let cfg = store().load();
    let shared = Arc::new(Shared {
        inner: Mutex::new(Inner {
            health: "Loading".into(),
            status_json: serde_json::to_string(&snapshot::loading()).unwrap_or_default(),
            mount_path: mount_dir(&cfg),
            cfg,
            offline: OfflineState::default(),
            cache: EmblemCache::default(),
        }),
        conn: OnceLock::new(),
        tx: Mutex::new(tx),
    });
    // The builder's own `name()` queues behind an existing owner; ask not to be queued.
    let built = connection::Builder::session()
        .and_then(|b| b.serve_at(PATH, Service(shared.clone())))
        .and_then(connection::Builder::build)
        .and_then(|c| {
            c.request_name_with_flags(name, RequestNameFlags::DoNotQueue.into())
                .map(|_| c)
        });
    let conn = match built {
        Ok(c) => c,
        Err(zbus::Error::NameTaken) => {
            eprintln!("{name} is already running");
            return 0;
        }
        Err(e) => {
            eprintln!("cannot serve {name} on the session bus: {e}");
            return 1;
        }
    };
    let _ = shared.conn.set(conn);
    // Move an old install over only now that the name is ours, and off this thread: D-Bus
    // activation must never wait for it. `StatusJson.migration` reports how it is going.
    let _ = rclone_ui_core::migrate::spawn_background();
    // Remind once per boot if the mount's unit isn't enabled to start at login.
    rclone_ui_core::autostart::spawn_check();
    let w = shared.clone();
    std::thread::spawn(move || worker(&w, &rx));
    let p = shared.clone();
    std::thread::spawn(move || poller(&p));
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_action_returns_before_the_command_finishes() {
        let (release, wait) = mpsc::channel::<()>();
        let (done_tx, done) = mpsc::channel::<()>();
        // The "command" blocks until the test lets it go; the handler must not.
        assert!(start_service_action("restart", move || {
            let _ = wait.recv();
            let _ = done_tx.send(());
        }));
        assert!(done.try_recv().is_err(), "still running in the background");
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn unknown_verbs_are_refused_without_running_anything() {
        let (tx, rx) = mpsc::channel::<()>();
        assert!(!start_service_action("disable", move || {
            let _ = tx.send(());
        }));
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }
}
