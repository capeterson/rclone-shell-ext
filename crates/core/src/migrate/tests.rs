// SPDX-License-Identifier: GPL-3.0-only
//! Migration tests: every one runs against a temp directory with a fake `systemctl`; nothing
//! here can reach the real home directory or the real systemd.

use super::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);
const ID: &str = "05d3a45b469151e9";

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Sandbox {
    root: PathBuf,
    calls: RefCell<Vec<String>>,
    enabled: RefCell<BTreeSet<String>>,
    active: RefCell<BTreeSet<String>>,
    pins_listed: RefCell<Vec<String>>,
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
            enabled: RefCell::default(),
            active: RefCell::default(),
            pins_listed: RefCell::default(),
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

    fn run_with(
        &self,
        exe: &str,
        f: impl FnOnce(&Env) -> Result<Vec<String>, String>,
    ) -> Result<Vec<String>, String> {
        let runner = |args: &[&str]| -> Result<String, String> {
            let line = args.join(" ");
            let unit = args.last().copied().unwrap_or_default().to_string();
            match args.first().copied() {
                Some("is-enabled") => {
                    return if self.enabled.borrow().contains(&unit) {
                        Ok("enabled\n".into())
                    } else {
                        Err("disabled".into())
                    };
                }
                Some("is-active") => {
                    return if self.active.borrow().contains(&unit) {
                        Ok("active\n".into())
                    } else {
                        Err("inactive".into())
                    };
                }
                Some("list-units") => {
                    let rows: Vec<String> = self
                        .pins_listed
                        .borrow()
                        .iter()
                        .map(|u| format!("{u} loaded active running x"))
                        .collect();
                    return Ok(rows.join("\n"));
                }
                _ => {}
            }
            self.calls.borrow_mut().push(line);
            Ok(String::new())
        };
        let env = Env {
            config_home: self.config(),
            data_home: self.data(),
            state_home: self.state(),
            systemctl: &runner,
            exe: PathBuf::from(exe),
        };
        f(&env)
    }

    fn migrate(&self) -> Result<Vec<String>, String> {
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
            self.enabled
                .borrow_mut()
                .insert("rclone-plasma-offline.service".into());
            self.active
                .borrow_mut()
                .insert("rclone-plasma-offline.service".into());
            self.pins_listed
                .borrow_mut()
                .push(format!("rclone-plasma-pin@{ID}.service"));
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
            self.enabled
                .borrow_mut()
                .insert("cosmic-rclone-ui-offline.service".into());
            self.active
                .borrow_mut()
                .insert("cosmic-rclone-ui-offline.service".into());
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
            "stop rclone-plasma-offline.service".to_string(),
            format!("stop rclone-plasma-pin@{ID}.service"),
            "disable rclone-plasma-offline.service".into(),
            "daemon-reload".into(),
            "enable --now rclone-shell-ext-offline.service".into(),
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
    sb.enabled
        .borrow_mut()
        .remove("cosmic-rclone-ui-offline.service");
    sb.active
        .borrow_mut()
        .remove("cosmic-rclone-ui-offline.service");
    let plan = sb
        .run_with("/new/bin/rclone-shell-ext", |e| {
            Ok(vec![plan(e).unwrap().to_string()])
        })
        .unwrap();
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
    let err = sb.migrate().unwrap_err();
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
    let err = sb.migrate().unwrap_err();
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
    let err = sb.migrate().unwrap_err();
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
    assert!(sb.migrate().unwrap_err().contains("offline_enabled"));
    assert_eq!(sb.snapshot(), before);

    // The shared binary can't be found, and units would need it.
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    let before = sb.snapshot();
    let err = sb.run_with("", run).unwrap_err();
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
    let plan = sb
        .run_with("/x", |e| Ok(vec![plan(e).unwrap().to_string()]))
        .unwrap();
    assert!(plan[0].contains("Already migrated"), "{plan:?}");
}

#[test]
fn dry_run_describes_the_plan_and_changes_nothing() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    let before = sb.snapshot();
    let text = sb
        .run_with("/new/bin/rclone-shell-ext", |e| {
            Ok(vec![plan(e).unwrap().to_string()])
        })
        .unwrap()
        .remove(0);
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

#[test]
fn a_keeper_that_was_not_running_is_not_started() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    sb.active.borrow_mut().clear();
    sb.pins_listed.borrow_mut().clear();
    sb.migrate().unwrap();
    assert_eq!(
        sb.calls(),
        [
            "disable rclone-plasma-offline.service",
            "daemon-reload",
            "enable rclone-shell-ext-offline.service"
        ]
    );
}

#[test]
fn a_failing_stop_leaves_the_files_alone() {
    let sb = Sandbox::new();
    sb.seed_plasma(true);
    let before = sb.snapshot();
    let runner = |args: &[&str]| -> Result<String, String> {
        match args[0] {
            "is-enabled" | "is-active" => Ok("enabled".into()),
            "list-units" => Ok(String::new()),
            _ => Err("bus unavailable".into()),
        }
    };
    let env = Env {
        config_home: sb.config(),
        data_home: sb.data(),
        state_home: sb.state(),
        systemctl: &runner,
        exe: PathBuf::from("/x/rclone-shell-ext"),
    };
    let err = run(&env).unwrap_err();
    assert!(
        err.contains("step 1") && err.contains("bus unavailable"),
        "{err}"
    );
    assert_eq!(sb.snapshot(), before);
}
