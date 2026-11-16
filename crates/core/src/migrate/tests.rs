// SPDX-License-Identifier: GPL-3.0-only
//! Migration tests: every one runs against a temp directory with a fake `systemctl`; nothing
//! here can reach the real home directory or the real systemd.

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);
const ID: &str = "05d3a45b469151e9";

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A fake `systemd --user`: which units are enabled, active and loaded (a unit file is only
/// known to systemd after `daemon-reload`), so an `enable` of a unit nobody reloaded fails the
/// way the real thing does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct SystemdState {
    enabled: BTreeSet<String>,
    active: BTreeSet<String>,
    loaded: BTreeSet<String>,
}

struct Sandbox {
    root: PathBuf,
    calls: RefCell<Vec<String>>,
    sd: RefCell<SystemdState>,
    /// Unfinished-job counts to report, one per query; the last one repeats.
    jobs: RefCell<Vec<usize>>,
    /// `None`: the rc server can't be reached.
    jobs_reachable: Cell<bool>,
    mountinfo: RefCell<String>,
    sleeps: Cell<u32>,
    /// Make every systemctl that changes something fail.
    break_systemctl: Cell<bool>,
}

fn glob_match(pattern: &str, unit: &str) -> bool {
    match pattern.split_once('*') {
        Some((pre, post)) => unit.starts_with(pre) && unit.ends_with(post),
        None => pattern == unit,
    }
}

impl Sandbox {
    fn new() -> Self {
        crate::identity::use_test_identity();
        let root = std::env::temp_dir().join(format!(
            "rcs-migrate-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["config", "data", "state"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        Self {
            root,
            calls: RefCell::default(),
            sd: RefCell::default(),
            jobs: RefCell::new(vec![0]),
            jobs_reachable: Cell::new(true),
            mountinfo: RefCell::default(),
            sleeps: Cell::new(0),
            break_systemctl: Cell::new(false),
        }
    }

    fn config(&self) -> PathBuf {
        self.root.join("config")
    }
    fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    fn state(&self) -> PathBuf {
        self.root.join("state")
    }
    fn units(&self) -> PathBuf {
        self.config().join("systemd/user")
    }
    fn enable_old(&self, unit: &str) {
        self.sd.borrow_mut().enabled.insert(unit.into());
    }
    fn activate(&self, unit: &str) {
        self.sd.borrow_mut().active.insert(unit.into());
    }
    fn systemd(&self) -> SystemdState {
        self.sd.borrow().clone()
    }

    /// `daemon-reload`: systemd re-reads the unit files.
    fn reload(&self) {
        let loaded = std::fs::read_dir(self.units())
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().is_file())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        self.sd.borrow_mut().loaded = loaded;
    }

    fn fake_systemctl(&self, args: &[&str]) -> Result<String, String> {
        let verb = args.first().copied().unwrap_or_default();
        let unit = args.last().copied().unwrap_or_default().to_string();
        let is_loaded = |sd: &SystemdState, u: &str| {
            sd.loaded.contains(u)
                || u.split_once('@').is_some_and(|(pre, post)| {
                    let ext = post.rsplit('.').next().unwrap_or_default();
                    sd.loaded.contains(&format!("{pre}@.{ext}"))
                })
        };
        let mut sd = self.sd.borrow_mut();
        match verb {
            "is-enabled" => {
                return if sd.enabled.contains(&unit) {
                    Ok("enabled\n".into())
                } else {
                    Err("disabled".into())
                };
            }
            "is-active" => {
                return if sd.active.contains(&unit) {
                    Ok("active\n".into())
                } else {
                    Err("inactive".into())
                };
            }
            "list-units" => {
                let rows: Vec<String> = sd
                    .active
                    .iter()
                    .filter(|u| glob_match(&unit, u))
                    .map(|u| format!("{u} loaded active running x"))
                    .collect();
                return Ok(rows.join("\n"));
            }
            _ => {}
        }
        self.calls.borrow_mut().push(args.join(" "));
        if self.break_systemctl.get() {
            return Err("bus unavailable".into());
        }
        let globbed = unit.contains('*');
        if matches!(verb, "stop" | "disable" | "enable" | "start")
            && !globbed
            && !is_loaded(&sd, &unit)
        {
            return Err(format!("Unit {unit} not found."));
        }
        match verb {
            "stop" => sd.active.retain(|u| !glob_match(&unit, u)),
            "disable" => {
                sd.enabled.remove(&unit);
            }
            "enable" => {
                sd.enabled.insert(unit.clone());
                if args.contains(&"--now") {
                    sd.active.insert(unit);
                }
            }
            "start" => {
                sd.active.insert(unit);
            }
            "daemon-reload" => {
                drop(sd);
                self.reload();
            }
            _ => {}
        }
        Ok(String::new())
    }

    fn run_with<T>(&self, exe: &str, f: impl FnOnce(&Env) -> T) -> T {
        let runner = |args: &[&str]| self.fake_systemctl(args);
        let jobs = |_: &Config| -> Option<usize> {
            if !self.jobs_reachable.get() {
                return None;
            }
            let mut v = self.jobs.borrow_mut();
            Some(if v.len() > 1 { v.remove(0) } else { v[0] })
        };
        let mountinfo = || self.mountinfo.borrow().clone();
        let sleep = |_: Duration| self.sleeps.set(self.sleeps.get() + 1);
        let env = Env {
            config_home: self.config(),
            data_home: self.data(),
            state_home: self.state(),
            systemctl: &runner,
            exe: PathBuf::from(exe),
            unfinished_jobs: &jobs,
            mountinfo: &mountinfo,
            sleep: &sleep,
            quiet_timeout: Duration::from_secs(120),
            poll: Duration::from_secs(2),
        };
        f(&env)
    }

    fn migrate(&self) -> Result<Vec<String>, MigrationFailure> {
        self.run_with("/new/bin/rclone-shell-ext", run)
    }

    /// systemctl calls that change something (the queries are not recorded).
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    /// Every file under the three roots, with its content.
    fn snapshot(&self) -> BTreeMap<String, String> {
        fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                if p.is_dir() {
                    out.insert(format!("{rel}/"), String::new());
                    walk(base, &p, out);
                } else {
                    out.insert(rel, std::fs::read_to_string(&p).unwrap_or_default());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.root, &self.root, &mut out);
        out
    }

    // ---- seeding old installs ----

    fn seed_units(&self, slug: &str, service: &str) {
        let hdr = format!("# Written by {slug}; remove it from the app's settings.\n");
        put(
            &self.units().join(format!("{slug}-offline.service")),
            &format!("{hdr}[Service]\nExecStart=/old/bin/{slug} --offline-keeper\n"),
        );
        put(
            &self.units().join(format!("{slug}-pin@.service")),
            &format!("{hdr}[Service]\nExecStart=/old/bin/{slug} --pin-mount %i\n"),
        );
        put(
            &self
                .units()
                .join(format!("{service}.d"))
                .join(format!("50-{slug}.conf")),
            "[Service]\nEnvironment=RCLONE_UMASK=077\n",
        );
        put(
            &self.units().join(service),
            "[Service]\nExecStart=rclone mount\n",
        );
    }

    fn seed_offline_data(&self, slug: &str) {
        put(
            &self
                .data()
                .join(slug)
                .join("offline")
                .join(ID)
                .join("a.txt"),
            "local copy",
        );
        let state = self.state().join(slug);
        put(&state.join("offline.json"), "{\"pins\":[]}");
        let flat_path = flat(&self.data().join(slug).join("offline"));
        for suffix in [
            ".path1.lst",
            ".path1.lst-old",
            ".path2.lst",
            ".path2.lst-old",
        ] {
            put(
                &state
                    .join("bisync")
                    .join(format!("gdrive_..{flat_path}_{ID}{suffix}")),
                &format!("listing {suffix}"),
            );
        }
        put(&state.join("bisync/keeper-filters.txt"), "- *.partial\n");
        let cache = state
            .join("pin-cache")
            .join(ID)
            .join("vfs/local")
            .join(slashless(&self.data().join(slug)))
            .join("offline");
        put(&cache.join("dirty.bin"), "cached");
    }

    fn seed_plasma(&self, offline: bool) {
        let enabled = if offline { "true" } else { "false" };
        put(
            &self.config().join("rclone-plasmarc"),
            &format!(
                "[General]\nrc_addr=127.0.0.1:5599\nservice=rclone-gdrive.service\n\
                 offline_enabled={enabled}\npins=wallpaper\nsync_interval_secs=120\n"
            ),
        );
        if offline {
            self.seed_units("rclone-plasma", "rclone-gdrive.service");
            self.seed_offline_data("rclone-plasma");
            self.enable_old("rclone-plasma-offline.service");
            self.activate("rclone-plasma-offline.service");
            self.activate(&format!("rclone-plasma-pin@{ID}.service"));
            self.activate("rclone-gdrive.service");
            self.reload();
        }
    }

    fn seed_cosmic(&self, offline: bool) {
        let dir = self
            .config()
            .join("cosmic/com.ca-peterson.CosmicRcloneUi/v1");
        put(&dir.join("rc_addr"), "\"127.0.0.1:5588\"");
        put(
            &dir.join("offline_enabled"),
            if offline { "true" } else { "false" },
        );
        put(&dir.join("pins"), "[\n    \"Docs\",\n]");
        put(&dir.join("context_menu"), "false");
        if offline {
            self.seed_units("cosmic-rclone-ui", "rclone-gdrive.service");
            self.seed_offline_data("cosmic-rclone-ui");
            self.enable_old("cosmic-rclone-ui-offline.service");
            self.activate("cosmic-rclone-ui-offline.service");
            self.activate("rclone-gdrive.service");
            self.reload();
        }
    }

    fn seed_panel(&self) {
        put(
            &self
                .config()
                .join("cosmic/com.system76.CosmicPanel.Panel/v1/plugins_wings"),
            "Some(([\"com.system76.CosmicAppletTime\"], [\"com.ca-peterson.CosmicRcloneUi\", \"com.ca-peterson.CosmicRcloneUiDev\"]))",
        );
        put(
            &self
                .config()
                .join("cosmic/com.system76.CosmicPanel.Dock/v1/plugins_center"),
            "Some([\"com.ca-peterson.CosmicRcloneUi\"])",
        );
        put(
            &self
                .config()
                .join("cosmic/com.system76.CosmicPanel.Panel/v1/plugins_center"),
            "Some([\"com.system76.CosmicAppletTime\"])",
        );
    }

    fn new_config(&self) -> Config {
        config::parse_toml(
            &std::fs::read_to_string(self.config().join("rclone-shell-ext/config.toml")).unwrap(),
        )
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn nothing_old_only_writes_the_marker() {
    let sb = Sandbox::new();
    sb.migrate().unwrap();
    assert!(sb.state().join("rclone-shell-ext/migrated-v1").exists());
    assert_eq!(sb.calls().len(), 0);
    assert!(!sb.config().join("rclone-shell-ext").exists());
    // And again: cheap and silent.
    assert_eq!(sb.migrate().unwrap(), Vec::<String>::new());
}

#[test]
fn plasma_only_moves_everything_and_keeps_the_baseline() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    let old_flat = flat(&sb.data().join("rclone-plasma/offline"));
    let listing = |flat: &str, s: &str| format!("gdrive_..{flat}_{ID}{s}");

    let lines = sb.migrate().unwrap();
    assert!(lines[0].contains("old Plasma install"), "{lines:?}");

    // Settings carried over into the one TOML file; the old file is not deleted.
    let cfg = sb.new_config();
    assert_eq!(cfg.rc_addr, "127.0.0.1:5599");
    assert!(cfg.offline_enabled);
    assert_eq!(cfg.pins, ["wallpaper"]);
    assert_eq!(cfg.sync_interval_secs, 120);
    assert!(sb.config().join("rclone-plasmarc").exists());

    // Data and state were renamed, not copied.
    assert!(!sb.data().join("rclone-plasma").exists());
    assert!(!sb.state().join("rclone-plasma").exists());
    assert_eq!(
        std::fs::read_to_string(
            sb.data()
                .join(format!("rclone-shell-ext/offline/{ID}/a.txt"))
        )
        .unwrap(),
        "local copy"
    );
    assert!(sb.state().join("rclone-shell-ext/offline.json").exists());

    // The bisync listings carry the new data path in their names, contents untouched, so bisync
    // finds its baseline and needs no resync.
    let new_flat = flat(&sb.data().join("rclone-shell-ext/offline"));
    assert_ne!(old_flat, new_flat);
    let bisync = names(&sb.state().join("rclone-shell-ext/bisync"));
    for s in [
        ".path1.lst",
        ".path1.lst-old",
        ".path2.lst",
        ".path2.lst-old",
    ] {
        assert!(bisync.contains(&listing(&new_flat, s)), "{bisync:?}");
        assert!(!bisync.contains(&listing(&old_flat, s)), "{bisync:?}");
        assert_eq!(
            std::fs::read_to_string(
                sb.state()
                    .join("rclone-shell-ext/bisync")
                    .join(listing(&new_flat, s))
            )
            .unwrap(),
            format!("listing {s}")
        );
    }
    assert!(bisync.contains(&"keeper-filters.txt".to_string()));
    // The pin mounts' vfs cache followed too.
    assert!(sb
        .state()
        .join("rclone-shell-ext/pin-cache")
        .join(ID)
        .join("vfs/local")
        .join(slashless(&sb.data().join("rclone-shell-ext/offline")))
        .join("dirty.bin")
        .exists());

    // Units: new names pointing at the shared binary; old ones gone; one drop-in.
    let keeper =
        std::fs::read_to_string(sb.units().join("rclone-shell-ext-offline.service")).unwrap();
    assert!(
        keeper.contains("ExecStart=/new/bin/rclone-shell-ext --offline-keeper\n"),
        "{keeper}"
    );
    let pin = std::fs::read_to_string(sb.units().join("rclone-shell-ext-pin@.service")).unwrap();
    assert!(
        pin.contains("ExecStart=/new/bin/rclone-shell-ext --pin-mount %i\n"),
        "{pin}"
    );
    assert!(!sb.units().join("rclone-plasma-offline.service").exists());
    assert!(!sb.units().join("rclone-plasma-pin@.service").exists());
    assert_eq!(
        names(&sb.units().join("rclone-gdrive.service.d")),
        ["50-rclone-shell-ext.conf"]
    );
    assert!(sb.units().join("rclone-gdrive.service").exists());

    // Stop first, then rewrite, reload, and bring the keeper back.
    let calls = sb.calls();
    assert_eq!(
        calls,
        [
            "stop rclone-plasma-offline.service",
            "disable rclone-plasma-offline.service",
            "reset-failed rclone-plasma-offline.service",
            "stop rclone-plasma-pin@*.service",
            "reset-failed rclone-plasma-pin@*.service",
            "daemon-reload",
            "enable rclone-shell-ext-offline.service",
            "start rclone-shell-ext-offline.service",
        ]
    );
    assert!(sb.state().join("rclone-shell-ext/migrated-v1").exists());
}

#[test]
fn cosmic_only_reads_ron_and_rewrites_the_panel_entry() {
    let sb = Sandbox::new();
    sb.seed_cosmic(true);
    sb.seed_panel();
    sb.migrate().unwrap();
    let cfg = sb.new_config();
    assert_eq!(cfg.rc_addr, "127.0.0.1:5588");
    assert_eq!(cfg.pins, ["Docs"]);
    assert!(!cfg.context_menu);
    assert!(cfg.offline_enabled);
    assert!(sb.data().join("rclone-shell-ext/offline").join(ID).exists());
    assert!(!sb.data().join("cosmic-rclone-ui").exists());
    assert!(sb.units().join("rclone-shell-ext-offline.service").exists());
    // Panel: the applet's id is replaced everywhere it is listed; the dev id and other applets
    // are left alone.
    let read = |p: &str| std::fs::read_to_string(sb.config().join("cosmic").join(p)).unwrap();
    let wings = read("com.system76.CosmicPanel.Panel/v1/plugins_wings");
    assert!(
        wings.contains("\"com.ca-peterson.rclone-shell-ext.Cosmic\""),
        "{wings}"
    );
    assert!(
        !wings.contains("\"com.ca-peterson.CosmicRcloneUi\""),
        "{wings}"
    );
    assert!(
        wings.contains("\"com.ca-peterson.CosmicRcloneUiDev\""),
        "{wings}"
    );
    assert!(wings.contains("com.system76.CosmicAppletTime"));
    assert!(
        read("com.system76.CosmicPanel.Dock/v1/plugins_center").contains("rclone-shell-ext.Cosmic")
    );
    assert_eq!(
        read("com.system76.CosmicPanel.Panel/v1/plugins_center"),
        "Some([\"com.system76.CosmicAppletTime\"])"
    );
    // The old cosmic-config directory stays.
    assert!(sb
        .config()
        .join("cosmic/com.ca-peterson.CosmicRcloneUi/v1/rc_addr")
        .exists());
}

#[test]
fn settings_only_install_moves_just_the_config() {
    let sb = Sandbox::new();
    sb.seed_plasma(false);
    sb.migrate().unwrap();
    assert_eq!(sb.new_config().rc_addr, "127.0.0.1:5599");
    assert_eq!(sb.calls().len(), 0, "{:?}", sb.calls());
    assert!(!sb.units().exists());
}

#[test]
fn with_both_installs_the_one_with_the_enabled_keeper_wins() {
    let sb = Sandbox::new();
    sb.seed_cosmic(true);
    sb.seed_plasma(true);
    // The COSMIC keeper exists but isn't enabled; Plasma's is.
    sb.sd
        .borrow_mut()
        .enabled
        .remove("cosmic-rclone-ui-offline.service");
    sb.sd
        .borrow_mut()
        .active
        .remove("cosmic-rclone-ui-offline.service");
    let plan = vec![sb.run_with("/new/bin/rclone-shell-ext", |e| {
        plan(e).unwrap().to_string()
    })];
    assert!(plan[0].contains("old Plasma install"), "{plan:?}");
    assert!(
        plan[0].contains("old COSMIC install was also found"),
        "{plan:?}"
    );
    let lines = sb.migrate().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("COSMIC install was also found")),
        "{lines:?}"
    );
    // Plasma's pins and settings came over; COSMIC's data and units were not touched.
    assert_eq!(sb.new_config().pins, ["wallpaper"]);
    assert!(sb.data().join("cosmic-rclone-ui/offline").exists());
    assert!(sb.state().join("cosmic-rclone-ui/offline.json").exists());
    assert!(sb.units().join("cosmic-rclone-ui-offline.service").exists());
    assert!(!sb.units().join("rclone-plasma-offline.service").exists());
}

#[test]
fn two_enabled_keepers_is_ambiguous_and_changes_nothing() {
    let sb = Sandbox::new();
    sb.seed_cosmic(true);
    sb.seed_plasma(true);
    let before = sb.snapshot();
    let err = sb.migrate().unwrap_err().to_string();
    assert!(err.contains("isn't clear which one is in use"), "{err}");
    assert_eq!(sb.snapshot(), before);
    assert_eq!(sb.calls().len(), 0);
}

#[test]
fn an_unexpected_state_aborts_before_the_first_change() {
    // New data directory already holds something.
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    put(&sb.data().join("rclone-shell-ext/other.txt"), "someone's");
    let before = sb.snapshot();
    let err = sb.migrate().unwrap_err().to_string();
    assert!(err.contains("already exists and isn't empty"), "{err}");
    assert_eq!(sb.snapshot(), before);
    assert_eq!(sb.calls().len(), 0);

    // A unit the app didn't write.
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    put(
        &sb.units().join("rclone-plasma-offline.service"),
        "[Service]\nExecStart=/mine\n",
    );
    let before = sb.snapshot();
    let err = sb.migrate().unwrap_err().to_string();
    assert!(err.contains("isn't one this app wrote"), "{err}");
    assert_eq!(sb.snapshot(), before);

    // A setting that isn't valid RON.
    let sb = Sandbox::new();
    sb.seed_cosmic(false);
    put(
        &sb.config()
            .join("cosmic/com.ca-peterson.CosmicRcloneUi/v1/offline_enabled"),
        "maybe",
    );
    let before = sb.snapshot();
    assert!(sb
        .migrate()
        .unwrap_err()
        .to_string()
        .contains("offline_enabled"));
    assert_eq!(sb.snapshot(), before);

    // The shared binary can't be found, and units would need it.
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    let before = sb.snapshot();
    let err = sb.run_with("", run).unwrap_err().to_string();
    assert!(
        err.contains("can't find the `rclone-shell-ext` program"),
        "{err}"
    );
    assert_eq!(sb.snapshot(), before);
}

#[test]
fn an_empty_new_directory_in_the_way_is_fine() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    std::fs::create_dir_all(sb.data().join("rclone-shell-ext")).unwrap();
    std::fs::create_dir_all(sb.state().join("rclone-shell-ext")).unwrap();
    sb.migrate().unwrap();
    assert!(sb.data().join("rclone-shell-ext/offline").exists());
    assert!(sb.state().join("rclone-shell-ext/offline.json").exists());
}

#[test]
fn a_run_that_stopped_part_way_finishes_on_the_next_run() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    // The directories were renamed and the config written, then the process died: no listings
    // renamed, no units, no marker.
    std::fs::rename(
        sb.data().join("rclone-plasma"),
        sb.data().join("rclone-shell-ext"),
    )
    .unwrap();
    std::fs::rename(
        sb.state().join("rclone-plasma"),
        sb.state().join("rclone-shell-ext"),
    )
    .unwrap();
    put(
        &sb.config().join("rclone-shell-ext/config.toml"),
        "rc_addr = \"half:1\"\n",
    );
    sb.migrate().unwrap();
    let new_flat = flat(&sb.data().join("rclone-shell-ext/offline"));
    let bisync = names(&sb.state().join("rclone-shell-ext/bisync"));
    assert!(
        bisync.contains(&format!("gdrive_..{new_flat}_{ID}.path1.lst")),
        "{bisync:?}"
    );
    assert!(sb.units().join("rclone-shell-ext-offline.service").exists());
    assert!(!sb.units().join("rclone-plasma-offline.service").exists());
    assert!(sb.state().join("rclone-shell-ext/migrated-v1").exists());
    // The config that was already there is kept.
    assert_eq!(sb.new_config().rc_addr, "half:1");
}

#[test]
fn running_again_after_success_changes_nothing() {
    let sb = Sandbox::new();
    sb.seed_cosmic(true);
    sb.seed_panel();
    sb.migrate().unwrap();
    let (before, calls) = (sb.snapshot(), sb.calls().len());
    assert_eq!(sb.migrate().unwrap(), Vec::<String>::new());
    assert_eq!(sb.snapshot(), before);
    assert_eq!(sb.calls().len(), calls);
    let plan = vec![sb.run_with("/x", |e| plan(e).unwrap().to_string())];
    assert!(plan[0].contains("Already migrated"), "{plan:?}");
}

#[test]
fn dry_run_describes_the_plan_and_changes_nothing() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    let before = sb.snapshot();
    let text = sb.run_with("/new/bin/rclone-shell-ext", |e| {
        plan(e).unwrap().to_string()
    });
    assert_eq!(sb.snapshot(), before);
    assert_eq!(sb.calls().len(), 0);
    for want in [
        "old Plasma install",
        "systemctl --user stop rclone-plasma-offline.service",
        "rename",
        "rclone-shell-ext/config.toml",
        "rclone-shell-ext-offline.service",
        "systemctl --user daemon-reload",
        "migrated-v1",
    ] {
        assert!(text.contains(want), "missing {want:?} in:\n{text}");
    }
}

// ---- the old keeper, in-flight jobs, mounts ----

fn plasma_mountinfo(sb: &Sandbox) -> String {
    format!(
        "100 1 0:50 / {drive} rw - fuse.rclone gdrive: rw\n\
         101 100 0:51 / {drive}/wallpaper rw - fuse.rclone {src} rw\n\
         102 1 0:52 / /boot rw - ext4 /dev/sda1 rw\n",
        drive = sb.root.join("GoogleDrive").display(),
        src = sb.data().join("rclone-plasma/offline").join(ID).display(),
    )
}

#[test]
fn offline_mounts_are_found_by_source_or_by_place_in_the_drive() {
    let sb = Sandbox::new();
    let mi = plasma_mountinfo(&sb);
    let drive = sb.root.join("GoogleDrive").to_string_lossy().into_owned();
    let old = sb.data().join("rclone-plasma");
    // The pin mount is found; the Drive mount itself and other file systems are not.
    let found = offline_mount_left(&mi, &old, &drive).unwrap();
    assert!(found.ends_with("GoogleDrive/wallpaper"), "{found}");
    let drive_only = mi.lines().next().unwrap();
    assert_eq!(offline_mount_left(drive_only, &old, &drive), None);
    assert_eq!(
        offline_mount_left("1 1 0:1 / /boot rw - ext4 /dev/x rw", &old, &drive),
        None
    );
    // By source alone (mounted somewhere else) and by place alone (some other source).
    let by_source = format!(
        "1 1 0:1 / /mnt/x rw - fuse.rclone {}/offline/a rw",
        old.display()
    );
    assert!(offline_mount_left(&by_source, &old, &drive).is_some());
    let by_place = format!("1 1 0:1 / {drive}/Docs rw - fuse.rclone elsewhere rw");
    assert!(offline_mount_left(&by_place, &old, &drive).is_some());
    // Octal escapes in mount points.
    assert_eq!(unescape_octal("/a\\040b"), "/a b");
}

#[test]
fn waits_for_an_in_flight_sync_before_moving_anything() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    *sb.jobs.borrow_mut() = vec![2, 1, 0];
    sb.migrate().unwrap();
    assert_eq!(sb.sleeps.get(), 2, "polled until the jobs were gone");
    assert!(sb.state().join("rclone-shell-ext/migrated-v1").exists());
    // The stops happened before the first rename, and the wait between them.
    let calls = sb.calls();
    assert_eq!(calls[0], "stop rclone-plasma-offline.service");
}

#[test]
fn a_sync_that_never_ends_puts_the_old_keeper_back_and_changes_nothing() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    *sb.jobs.borrow_mut() = vec![1];
    let (files, sd_before) = (sb.snapshot(), sb.systemd());
    let err = sb.migrate().unwrap_err();
    assert_eq!(err.code, MigrateError::SyncInFlight, "{err}");
    assert!(err.message.contains("still running 1 sync"), "{err}");
    // 120 s at 2 s a poll.
    assert_eq!(sb.sleeps.get(), 59);
    // No file moved; the journal is gone; the old keeper is enabled and running again.
    assert_eq!(sb.snapshot(), files);
    let sd = sb.systemd();
    assert!(sd.enabled.contains("rclone-plasma-offline.service"));
    assert!(sd.active.contains("rclone-plasma-offline.service"));
    assert_eq!(sd.enabled.len(), sd_before.enabled.len());
}

#[test]
fn a_pin_mount_that_stays_puts_the_old_keeper_back() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    // The old config's mount path is `~/GoogleDrive`; point it at the sandbox.
    put(
        &sb.config().join("rclone-plasmarc"),
        &format!(
            "[General]\nmount_path={}\noffline_enabled=true\npins=wallpaper\n",
            sb.root.join("GoogleDrive").display()
        ),
    );
    *sb.mountinfo.borrow_mut() = plasma_mountinfo(&sb);
    let files = sb.snapshot();
    let err = sb.migrate().unwrap_err();
    assert_eq!(err.code, MigrateError::MountInUse, "{err}");
    assert_eq!(sb.snapshot(), files);
    assert!(sb
        .systemd()
        .active
        .contains("rclone-plasma-offline.service"));
}

#[test]
fn an_unreachable_rc_server_means_no_jobs() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    sb.jobs_reachable.set(false);
    sb.migrate().unwrap();
    assert_eq!(sb.sleeps.get(), 0);
}

#[test]
fn the_old_units_are_stopped_even_when_systemd_says_they_are_not_active() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    // "activating" (an auto-restarting keeper) is not "active" to is-active.
    sb.sd.borrow_mut().active.clear();
    sb.migrate().unwrap();
    let calls = sb.calls();
    for want in [
        "stop rclone-plasma-offline.service",
        "disable rclone-plasma-offline.service",
        "reset-failed rclone-plasma-offline.service",
        "stop rclone-plasma-pin@*.service",
    ] {
        assert!(calls.contains(&want.to_string()), "{want}: {calls:?}");
    }
    assert!(sb
        .systemd()
        .enabled
        .contains("rclone-shell-ext-offline.service"));
}

#[test]
fn a_keeper_that_was_not_running_is_enabled_but_not_started() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    sb.sd.borrow_mut().active.clear();
    sb.migrate().unwrap();
    let sd = sb.systemd();
    assert!(sd.enabled.contains("rclone-shell-ext-offline.service"));
    assert!(sd.active.is_empty(), "{sd:?}");
}

#[test]
fn a_failing_stop_leaves_the_files_alone() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    sb.break_systemctl.set(true);
    let before = sb.snapshot();
    let err = sb.migrate().unwrap_err().to_string();
    assert!(
        err.contains("step 1") && err.contains("bus unavailable"),
        "{err}"
    );
    // Only the journal (written before the first change) differs.
    let mut after = sb.snapshot();
    assert!(after
        .remove("state/rclone-shell-ext-migrate-journal.json")
        .is_some());
    assert_eq!(after, before);
}

// ---- interrupted runs end where an uninterrupted one does ----

/// The files and systemd state, with the sandbox's own name (it is in bisync's file names)
/// replaced so two sandboxes can be compared.
fn end_state(sb: &Sandbox) -> (BTreeMap<String, String>, SystemdState) {
    let name = sb.root.file_name().unwrap().to_string_lossy().into_owned();
    let norm = |s: &str| {
        s.replace(&name, "SANDBOX")
            .replace(&name.replace('/', "_"), "SANDBOX")
    };
    let files = sb
        .snapshot()
        .into_iter()
        .map(|(k, v)| (norm(&k), norm(&v)))
        .collect();
    (files, sb.systemd())
}

fn assert_resume_matches(seed: impl Fn(&Sandbox)) {
    let whole = Sandbox::new();
    seed(&whole);
    whole.migrate().unwrap();
    let want = end_state(&whole);
    let steps = {
        let sb = Sandbox::new();
        seed(&sb);
        sb.run_with("/new/bin/rclone-shell-ext", |e| {
            plan(e).unwrap().steps.len()
        })
    };
    for k in 0..steps {
        let sb = Sandbox::new();
        seed(&sb);
        let err = sb
            .run_with("/new/bin/rclone-shell-ext", |e| run_limited(e, Some(k)))
            .unwrap_err();
        assert_eq!(err.message, "interrupted");
        // The journal is there as soon as anything may have changed.
        assert!(
            sb.state()
                .join("rclone-shell-ext-migrate-journal.json")
                .exists(),
            "interrupted after {k} steps"
        );
        sb.migrate().unwrap();
        let got = end_state(&sb);
        // The temp roots differ in name only through their content, which has no paths of
        // its own, so a straight comparison is fine.
        assert_eq!(
            got.1, want.1,
            "systemd state after interrupting at step {k}"
        );
        assert_eq!(got.0, want.0, "files after interrupting at step {k}");
    }
}

#[test]
fn a_plasma_run_interrupted_at_any_step_ends_like_an_uninterrupted_one() {
    assert_resume_matches(|sb| sb.seed_plasma(true));
}

#[test]
fn a_cosmic_run_interrupted_at_any_step_ends_like_an_uninterrupted_one() {
    assert_resume_matches(|sb| {
        sb.seed_cosmic(true);
        sb.seed_panel();
    });
}

#[test]
fn an_interrupted_run_with_a_stopped_mount_service_still_enables_but_does_not_start() {
    assert_resume_matches(|sb| {
        sb.seed_plasma(true);
        sb.sd.borrow_mut().active.remove("rclone-gdrive.service");
    });
}

#[test]
fn the_journal_records_codes_and_is_gone_after_success() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    let _ = sb.run_with("/new/bin/rclone-shell-ext", |e| run_limited(e, Some(1)));
    let text =
        std::fs::read_to_string(sb.state().join("rclone-shell-ext-migrate-journal.json")).unwrap();
    let j: Journal = serde_json::from_str(&text).unwrap();
    assert_eq!(j.source, "plasma");
    assert!(j.keeper_was_enabled && j.keeper_was_active && j.offline_enabled && j.had_units);
    assert_eq!(j.pins, [ID]);
    sb.migrate().unwrap();
    assert!(!sb
        .state()
        .join("rclone-shell-ext-migrate-journal.json")
        .exists());
}

// ---- pending, the guard, the lock, small things ----

#[test]
fn pending_follows_the_marker_and_the_old_install() {
    let sb = Sandbox::new();
    let st = |sb: &Sandbox| pending_in(&sb.config(), &sb.data(), &sb.state());
    assert_eq!(st(&sb), MigrationStatus::NotNeeded);
    sb.seed_plasma(true);
    assert_eq!(st(&sb), MigrationStatus::Pending);
    // A failed attempt is remembered as a code.
    let res = sb.run_with("/new/bin/rclone-shell-ext", |e| {
        run_locked(e, Duration::from_secs(1))
    });
    assert!(res.is_ok());
    assert_eq!(st(&sb), MigrationStatus::NotNeeded);

    let sb = Sandbox::new();
    sb.seed_plasma(true);
    *sb.jobs.borrow_mut() = vec![1];
    assert!(sb
        .run_with("/new/bin/rclone-shell-ext", |e| run_locked(
            e,
            Duration::from_secs(1)
        ))
        .is_err());
    let failed = st(&sb);
    assert_eq!(failed, MigrationStatus::Failed(MigrateError::SyncInFlight));
    assert_eq!(failed.code(), "sync-in-flight");
    assert_eq!(MigrationStatus::Pending.code(), "pending");
    assert_eq!(MigrationStatus::NotNeeded.code(), "none");
    assert_eq!(MigrationStatus::NotNeeded.label(), "");
    assert_ne!(failed.label(), "");
    // Asking changed nothing; a later success clears the failure.
    *sb.jobs.borrow_mut() = vec![0];
    sb.run_with("/new/bin/rclone-shell-ext", |e| {
        run_locked(e, Duration::from_secs(1))
    })
    .unwrap();
    assert_eq!(st(&sb), MigrationStatus::NotNeeded);
    assert!(!sb.state().join("rclone-shell-ext-migrate-error").exists());
}

#[test]
fn pending_does_not_change_anything() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    let (files, sd) = (sb.snapshot(), sb.systemd());
    let _ = pending_in(&sb.config(), &sb.data(), &sb.state());
    assert_eq!((sb.snapshot(), sb.systemd()), (files, sd));
    assert_eq!(sb.calls().len(), 0);
}

#[test]
fn offline_setup_is_refused_while_an_old_install_waits() {
    for st in [
        MigrationStatus::Pending,
        MigrationStatus::Failed(MigrateError::Conflict),
    ] {
        let err = guard_status(st).unwrap_err();
        assert_eq!(err.code, MigrateError::OldInstallPresent);
        assert!(err.message.contains("--migrate"), "{err}");
    }
    assert!(guard_status(MigrationStatus::NotNeeded).is_ok());
}

#[test]
fn the_lock_loser_waits_for_the_marker() {
    let sb = Sandbox::new();
    sb.run_with("/x", |env| {
        // Free: taken, and released on drop.
        let g = acquire_wait(env, Duration::from_secs(60)).unwrap();
        assert!(g.is_some() && env.lock_path().exists());
        // Held (fresh): waits, and sees the marker appear.
        let marker = env.marker();
        let sleep_then_marker = |_: Duration| {
            put(&marker, "done");
        };
        let env2 = Env {
            sleep: &sleep_then_marker,
            ..clone_env(env)
        };
        assert!(acquire_wait(&env2, Duration::from_secs(60))
            .unwrap()
            .is_none());
        drop(g);
        assert!(!env.lock_path().exists());
    });
    // Held, and the marker never comes: a timeout with its code.
    let sb = Sandbox::new();
    sb.run_with("/x", |env| {
        put(&env.lock_path(), "");
        let err = acquire_wait(env, Duration::from_secs(60)).unwrap_err();
        assert_eq!(err.code, MigrateError::LockTimeout);
        assert_eq!(sb.sleeps.get(), 30);
    });
}

fn clone_env<'a>(e: &Env<'a>) -> Env<'a> {
    Env {
        config_home: e.config_home.clone(),
        data_home: e.data_home.clone(),
        state_home: e.state_home.clone(),
        systemctl: e.systemctl,
        exe: e.exe.clone(),
        unfinished_jobs: e.unfinished_jobs,
        mountinfo: e.mountinfo,
        sleep: e.sleep,
        quiet_timeout: e.quiet_timeout,
        poll: e.poll,
    }
}

#[test]
fn the_dev_build_refuses_to_migrate() {
    let err = refuse_dev(true).unwrap_err();
    assert_eq!(err.code, MigrateError::DevBuild);
    assert!(refuse_dev(false).is_ok());
}

#[test]
fn error_codes_round_trip() {
    for e in [
        MigrateError::Ambiguous,
        MigrateError::Conflict,
        MigrateError::SyncInFlight,
        MigrateError::MountInUse,
        MigrateError::LockTimeout,
        MigrateError::OldInstallPresent,
        MigrateError::DevBuild,
        MigrateError::Failed,
    ] {
        assert_eq!(MigrateError::from_code(e.code()), e);
        assert_ne!(e.label(), "");
    }
    assert_eq!(MigrateError::from_code("???"), MigrateError::Failed);
}

#[test]
fn a_directory_that_cannot_be_removed_is_reported_not_swallowed() {
    let sb = Sandbox::new();
    sb.run_with("/x", |env| {
        let mut log = Vec::new();
        // Other drop-ins in it: expected, silent.
        let busy = sb.root.join("busy");
        put(&busy.join("other.conf"), "x");
        execute_step(env, &Step::RemoveDirIfEmpty(busy.clone()), &mut log).unwrap();
        assert!(busy.exists() && log.is_empty());
        // A file where a directory should be: a real error, logged.
        let file = sb.root.join("file");
        put(&file, "x");
        execute_step(env, &Step::RemoveDirIfEmpty(file), &mut log).unwrap();
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(log[0].contains("couldn't remove the directory"), "{log:?}");
    });
}

#[test]
fn the_flattening_is_the_one_the_keeper_module_documents() {
    assert_eq!(
        crate::bisync_name::flatten(Path::new("/a/b/c")),
        flat(Path::new("/a/b/c"))
    );
}
