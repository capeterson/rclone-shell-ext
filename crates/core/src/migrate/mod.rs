// SPDX-License-Identifier: GPL-3.0-only
//! One-time move from the old per-front-end identities to the shared one.
//!
//! The COSMIC applet used to be `cosmic-rclone-ui` (settings in cosmic-config) and the Plasma
//! front end `rclone-plasma` (settings in a `KConfig` file), each with its own data, state and
//! systemd units. Now both are `rclone-shell-ext` with one `config.toml`. This module finds the
//! old install, builds a [`Plan`] (what would change, nothing touched), and executes it.
//!
//! Everything goes through an [`Env`]: the three XDG roots, a `systemctl` runner and the path of
//! the shared binary the new units run. Nothing here reads `$HOME` on its own except
//! [`run_if_needed`], the entry point the binaries call, which builds an `Env` from the process.
//!
//! The bisync baseline matters most: bisync names its listing files after the local path, which
//! contains the old slug, so the listings are renamed to the new path's name; without that every
//! offline folder would need a `--resync`.
//!
//! Safety rules: the plan is built before anything changes and fails (changing nothing) on state it
//! doesn't understand; old config files are never deleted; a marker file written last stops it
//! from running again; every step is safe to repeat, so an interrupted run resumes by running
//! again.

mod readers;

pub use readers::parse_kconfig;

use crate::bisync_name::flatten as flat;
use crate::config::{self, Config, TomlStore};
use crate::identity::{APP_ID, COSMIC_DESKTOP_ID, SLUG};
use crate::rc::RcProbeError;
use crate::units;
use rclone_ui_i18n::fl;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// File in the new state directory that says the migration is done.
pub const MARKER: &str = "migrated-v1";
/// Set to anything to stop [`run_if_needed`] from doing anything (for tests that run binaries).
pub const SKIP_ENV: &str = "RCLONE_SHELL_EXT_NO_MIGRATE";

const OLD_COSMIC_SLUG: &str = "cosmic-rclone-ui";
const OLD_COSMIC_APP_ID: &str = "com.ca-peterson.CosmicRcloneUi";
const OLD_PLASMA_SLUG: &str = "rclone-plasma";
const OLD_PLASMA_CONFIG: &str = "rclone-plasmarc";
/// The COSMIC panel config directories and files that list applets by id.
const PANEL_DIRS: [&str; 2] = [
    "com.system76.CosmicPanel.Panel",
    "com.system76.CosmicPanel.Dock",
];
const PANEL_FILES: [&str; 2] = ["plugins_wings", "plugins_center"];
/// The old Plasma plasmoid id, and the file in `$XDG_CONFIG_HOME` where plasmashell lists its
/// applets (the panel entry and the system tray's lists).
const OLD_PLASMOID_ID: &str = "com.ca-peterson.rcloneplasma";
const PLASMA_APPLETSRC: &str = "plasma-org.kde.plasma.desktop-appletsrc";
/// bisync's name for a file is limited by the file system.
const FILE_NAME_MAX: usize = 255;

/// Why a migration did not happen. Stable codes (see [`MigrateError::code`]) are what gets
/// persisted and sent over D-Bus; [`MigrateError::label`] is the text for the current language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrateError {
    /// Both old installs have offline folders and it isn't clear which is in use.
    Ambiguous,
    /// A new data or state directory is in the way, or a unit isn't ours.
    Conflict,
    /// A sync started by the old keeper is still running inside rclone.
    SyncInFlight,
    /// An offline folder is still mounted.
    MountInUse,
    /// Another process holds the migration lock and didn't finish in time.
    LockTimeout,
    /// An old install is present and not yet migrated: refused to set up offline folders.
    OldInstallPresent,
    /// The dev build never migrates.
    DevBuild,
    /// Anything else (a step failed).
    Failed,
}

impl MigrateError {
    /// The stable code.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            MigrateError::Ambiguous => "ambiguous",
            MigrateError::Conflict => "conflict",
            MigrateError::SyncInFlight => "sync-in-flight",
            MigrateError::MountInUse => "mount-in-use",
            MigrateError::LockTimeout => "lock-timeout",
            MigrateError::OldInstallPresent => "old-install-present",
            MigrateError::DevBuild => "dev-build",
            MigrateError::Failed => "failed",
        }
    }

    /// The error for a code from [`code`](Self::code); unknown codes are `Failed`.
    #[must_use]
    pub fn from_code(code: &str) -> MigrateError {
        [
            MigrateError::Ambiguous,
            MigrateError::Conflict,
            MigrateError::SyncInFlight,
            MigrateError::MountInUse,
            MigrateError::LockTimeout,
            MigrateError::OldInstallPresent,
            MigrateError::DevBuild,
        ]
        .into_iter()
        .find(|e| e.code() == code.trim())
        .unwrap_or(MigrateError::Failed)
    }

    /// Short text for the current language.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            MigrateError::Ambiguous => fl!("migrate-label-ambiguous"),
            MigrateError::Conflict => fl!("migrate-label-conflict"),
            MigrateError::SyncInFlight => fl!("migrate-label-sync-in-flight"),
            MigrateError::MountInUse => fl!("migrate-label-mount-in-use"),
            MigrateError::LockTimeout => fl!("migrate-label-lock-timeout"),
            MigrateError::OldInstallPresent => fl!("migrate-label-old-install-present"),
            MigrateError::DevBuild => fl!("migrate-label-dev-build"),
            MigrateError::Failed => fl!("migrate-label-failed"),
        }
    }
}

/// A failed migration: the code for programs, the message for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFailure {
    /// Why.
    pub code: MigrateError,
    /// What to tell the user (current language).
    pub message: String,
}

impl MigrationFailure {
    fn new(code: MigrateError, message: String) -> Self {
        Self { code, message }
    }
}

impl std::fmt::Display for MigrationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MigrationFailure {}

impl From<String> for MigrationFailure {
    fn from(message: String) -> Self {
        Self::new(MigrateError::Failed, message)
    }
}

/// Whether a migration is waiting, without touching anything. See [`pending`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MigrationStatus {
    /// Nothing to do (marker present, no old install, dev build).
    #[default]
    NotNeeded,
    /// An old install is waiting to be moved.
    Pending,
    /// The last attempt failed with this error; running it again retries.
    Failed(MigrateError),
}

impl MigrationStatus {
    /// `none`, `pending`, or the error's code: what the D-Bus `migration` field carries.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            MigrationStatus::NotNeeded => "none",
            MigrationStatus::Pending => "pending",
            MigrationStatus::Failed(e) => e.code(),
        }
    }

    /// Finished text for the current language (empty when nothing is wrong).
    #[must_use]
    pub fn label(self) -> String {
        match self {
            MigrationStatus::NotNeeded => String::new(),
            MigrationStatus::Pending => fl!("migrate-label-pending"),
            MigrationStatus::Failed(e) => e.label(),
        }
    }
}

/// A `systemctl --user` runner: arguments in, standard output out (an error for a non-zero exit).
pub type Systemctl<'a> = dyn Fn(&[&str]) -> Result<String, String> + 'a;

/// Everything the migration touches, injected.
pub struct Env<'a> {
    /// `$XDG_CONFIG_HOME`
    pub config_home: PathBuf,
    /// `$XDG_DATA_HOME`
    pub data_home: PathBuf,
    /// `$XDG_STATE_HOME`
    pub state_home: PathBuf,
    /// Runs `systemctl --user ARGS` (a fake in tests).
    pub systemctl: &'a Systemctl<'a>,
    /// The shared `rclone-shell-ext` binary the new units run (empty if it couldn't be found).
    pub exe: PathBuf,
    /// How many rc jobs the old install's rclone still runs. `NotRunning` (nothing listens, e.g.
    /// the mount service is down) means none; any other error means unknown, and unknown is
    /// never taken for "none".
    pub unfinished_jobs: &'a dyn Fn(&Config) -> Result<usize, RcProbeError>,
    /// The text of `/proc/self/mountinfo`.
    pub mountinfo: &'a dyn Fn() -> String,
    /// Sleeps between polls (a no-op counter in tests).
    pub sleep: &'a dyn Fn(Duration),
    /// How long to wait for jobs and mounts to go away before giving up.
    pub quiet_timeout: Duration,
    /// Time between polls while waiting.
    pub poll: Duration,
}

/// A foreground process that lost the migration lock waits for the winner this long beyond
/// [`Env::quiet_timeout`] (the winner may wait that long for a sync, and stop services).
const LOCK_EXTRA_WAIT: Duration = Duration::from_secs(180);
/// How often a waiting foreground process says so.
const LOCK_PROGRESS_EVERY: Duration = Duration::from_secs(10);

impl Env<'_> {
    fn unit_dir(&self) -> PathBuf {
        units::unit_dir_in(&self.config_home)
    }

    fn new_data(&self) -> PathBuf {
        self.data_home.join(SLUG)
    }

    fn new_state(&self) -> PathBuf {
        self.state_home.join(SLUG)
    }

    fn marker(&self) -> PathBuf {
        self.new_state().join(MARKER)
    }

    /// Sits beside both state directories, so it survives their renames.
    fn journal_path(&self) -> PathBuf {
        self.state_home.join(format!("{SLUG}-migrate-journal.json"))
    }

    fn lock_path(&self) -> PathBuf {
        self.state_home.join(format!("{SLUG}-migrate.lock"))
    }

    /// The code of the last failure, for [`pending`].
    fn error_path(&self) -> PathBuf {
        self.state_home.join(format!("{SLUG}-migrate-error"))
    }

    fn sysctl(&self, args: &[&str]) -> Result<String, String> {
        (self.systemctl)(args)
    }
}

/// Which old front end an install belonged to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The COSMIC applet (`cosmic-rclone-ui`).
    Cosmic,
    /// The Plasma front end (`rclone-plasma`).
    Plasma,
}

impl Origin {
    fn code(self) -> &'static str {
        match self {
            Origin::Cosmic => "cosmic",
            Origin::Plasma => "plasma",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Origin::Cosmic => OLD_COSMIC_SLUG,
            Origin::Plasma => OLD_PLASMA_SLUG,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Origin::Cosmic => "COSMIC",
            Origin::Plasma => "Plasma",
        }
    }

    fn keeper_unit(self) -> String {
        format!("{}-offline.service", self.slug())
    }

    fn pin_template(self) -> String {
        format!("{}-pin@.service", self.slug())
    }

    fn dropin_name(self) -> String {
        format!("50-{}.conf", self.slug())
    }
}

/// What was found of one old install.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // independent facts, not a state machine
struct OldFacts {
    origin: Origin,
    config: Option<Config>,
    config_mtime: Option<SystemTime>,
    keeper_unit: Option<String>,
    pin_template: Option<String>,
    dropin: Option<PathBuf>,
    keeper_enabled: bool,
    keeper_active: bool,
    data_dir: bool,
    state_dir: bool,
    /// File names in the old bisync working directory.
    bisync_files: Vec<String>,
    /// `pin-cache/<id>/{vfs,vfsMeta}/local` directories (relative to the state directory) that
    /// hold a subdirectory named after the old data path.
    cache_dirs: Vec<PathBuf>,
}

impl OldFacts {
    fn present(&self) -> bool {
        self.config.is_some()
            || self.data_dir
            || self.state_dir
            || self.keeper_unit.is_some()
            || self.pin_template.is_some()
    }

    fn has_offline(&self) -> bool {
        self.config.as_ref().is_some_and(|c| c.offline_enabled) || self.keeper_unit.is_some()
    }

    fn has_units(&self) -> bool {
        self.keeper_unit.is_some() || self.pin_template.is_some() || self.dropin.is_some()
    }
}

/// What the file system and systemd looked like when the plan was made.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // independent facts, not a state machine
struct Facts {
    marker: bool,
    old: Vec<OldFacts>,
    new_config: bool,
    /// The new `config.toml` exists but isn't valid TOML (merging into it would lose it).
    new_config_corrupt: bool,
    new_data_exists: bool,
    new_data_empty: bool,
    new_state_exists: bool,
    new_state_empty: bool,
    /// COSMIC panel config files that list the old applet, with their text.
    panels: Vec<(PathBuf, String)>,
    /// plasmashell's applet list, if it names the old plasmoid.
    plasma_applets: Option<PathBuf>,
    /// What an earlier, interrupted run recorded before it changed anything.
    journal: Option<Journal>,
}

/// What the old install looked like before the first change, written before it (next to the
/// state directories, so it survives their renames). A resumed run takes these facts from here,
/// not from systemd, which by then shows the half-migrated state. Only codes and flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // independent facts, not a state machine
struct Journal {
    /// `cosmic` or `plasma`.
    source: String,
    keeper_was_enabled: bool,
    keeper_was_active: bool,
    /// Offline folders were switched on (so the new keeper must end up enabled).
    offline_enabled: bool,
    /// The old install had unit files (so new ones are written).
    had_units: bool,
    /// The old install's mount service (its keeper is only started when this is running).
    #[serde(default)]
    service: String,
}

impl Journal {
    fn service(&self) -> String {
        if self.service.is_empty() {
            Config::default().service
        } else {
            self.service.clone()
        }
    }

    fn origin(&self) -> Option<Origin> {
        [Origin::Cosmic, Origin::Plasma]
            .into_iter()
            .find(|o| o.code() == self.source)
    }
}

fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_none())
}

fn read_opt(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

/// A path without its leading slash, for joining under another directory.
fn slashless(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().trim_start_matches('/'))
}

fn gather_old(env: &Env, origin: Origin) -> Result<OldFacts, String> {
    let (config, config_mtime) = match origin {
        Origin::Cosmic => {
            let dir = env
                .config_home
                .join("cosmic")
                .join(OLD_COSMIC_APP_ID)
                .join("v1");
            readers::read_cosmic(&dir)?.map_or((None, None), |(c, m)| (Some(c), m))
        }
        Origin::Plasma => {
            let file = env.config_home.join(OLD_PLASMA_CONFIG);
            match read_opt(&file) {
                Some(text) => (
                    Some(readers::parse_kconfig(&text)),
                    std::fs::metadata(&file).and_then(|m| m.modified()).ok(),
                ),
                None => (None, None),
            }
        }
    };
    let unit_dir = env.unit_dir();
    let keeper_unit = read_opt(&unit_dir.join(origin.keeper_unit()));
    let pin_template = read_opt(&unit_dir.join(origin.pin_template()));
    let service = config.clone().unwrap_or_default().service;
    let dropin = Some(
        unit_dir
            .join(format!("{service}.d"))
            .join(origin.dropin_name()),
    )
    .filter(|p| p.is_file());
    let keeper_enabled = keeper_unit.is_some()
        && env
            .sysctl(&["is-enabled", &origin.keeper_unit()])
            .is_ok_and(|o| o.trim().starts_with("enabled"));
    let keeper_active =
        keeper_unit.is_some() && env.sysctl(&["is-active", &origin.keeper_unit()]).is_ok();
    let old_data = env.data_home.join(origin.slug());
    let old_state = env.state_home.join(origin.slug());
    // After an interrupted run the old state directory may already be the new one.
    let state_now = if old_state.is_dir() {
        old_state.clone()
    } else {
        env.new_state()
    };
    let bisync_files = std::fs::read_dir(state_now.join("bisync")).map_or_else(
        |_| Vec::new(),
        |rd| {
            let mut v: Vec<String> = rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        },
    );
    let mut cache_dirs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(state_now.join("pin-cache")) {
        for id in rd.flatten() {
            for kind in ["vfs", "vfsMeta"] {
                let prefix = Path::new("pin-cache")
                    .join(id.file_name())
                    .join(kind)
                    .join("local");
                if state_now.join(&prefix).join(slashless(&old_data)).is_dir() {
                    cache_dirs.push(prefix);
                }
            }
        }
        cache_dirs.sort();
    }
    Ok(OldFacts {
        origin,
        config,
        config_mtime,
        keeper_unit,
        pin_template,
        dropin,
        keeper_enabled,
        keeper_active,
        data_dir: old_data.is_dir(),
        state_dir: old_state.is_dir(),
        bisync_files,
        cache_dirs,
    })
}

fn gather(env: &Env) -> Result<Facts, String> {
    let mut panels = Vec::new();
    for dir in PANEL_DIRS {
        for file in PANEL_FILES {
            let path = env
                .config_home
                .join("cosmic")
                .join(dir)
                .join("v1")
                .join(file);
            if let Some(text) = read_opt(&path) {
                if text.contains(&format!("\"{OLD_COSMIC_APP_ID}\"")) {
                    panels.push((path, text));
                }
            }
        }
    }
    let applets = env.config_home.join(PLASMA_APPLETSRC);
    let plasma_applets = read_opt(&applets)
        .is_some_and(|t| rename_plasmoid_id(&t) != t)
        .then_some(applets);
    let old = [Origin::Cosmic, Origin::Plasma]
        .into_iter()
        .map(|o| gather_old(env, o))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(OldFacts::present)
        .collect();
    let (data, state) = (env.new_data(), env.new_state());
    let new_config_path = config::config_path_in(&env.config_home, SLUG);
    Ok(Facts {
        marker: env.marker().exists(),
        old,
        new_config: new_config_path.exists(),
        new_config_corrupt: read_opt(&new_config_path)
            .is_some_and(|t| toml::from_str::<toml::value::Table>(&t).is_err()),
        new_data_exists: data.exists(),
        new_data_empty: is_empty_dir(&data),
        new_state_exists: state.exists(),
        // The marker's own directory counts as empty for this purpose only before it is written.
        new_state_empty: is_empty_dir(&state),
        panels,
        plasma_applets,
        journal: read_opt(&env.journal_path()).and_then(|t| serde_json::from_str(&t).ok()),
    })
}

/// One thing the migration does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// `systemctl --user ARGS`; a failure is only noted when `best_effort`.
    Systemctl {
        /// The arguments after `--user`.
        args: Vec<String>,
        /// Carry on if it fails.
        best_effort: bool,
    },
    /// Wait until rclone has no unfinished jobs and no offline folder is mounted; if that
    /// doesn't happen in time the old keeper is put back and the migration stops.
    WaitQuiet {
        /// The old install's settings (where its rclone listens, where the Drive is mounted).
        cfg: Box<Config>,
        /// The old data directory (an offline folder's mount has it as its source).
        old_data: PathBuf,
    },
    /// Rename a file or directory (nothing to do if it is already gone and `to` exists).
    Rename {
        /// Where it is now.
        from: PathBuf,
        /// Where it goes.
        to: PathBuf,
    },
    /// Write a file (atomically).
    Write {
        /// The file.
        path: PathBuf,
        /// What it holds.
        content: String,
    },
    /// Merge the old settings into the new `config.toml` under its lock (the old values win for
    /// the settings both front ends had; a missing file becomes the old settings as they were).
    MergeConfig {
        /// The new config file.
        path: PathBuf,
        /// The old install's settings.
        old: Box<Config>,
    },
    /// Rename the old plasmoid id to the new one in plasmashell's applet list (exact entries
    /// only). Read again when it runs, since plasmashell rewrites the file itself; a failure is
    /// reported but doesn't stop the migration.
    RenamePlasmoid(PathBuf),
    /// Remove a file (nothing to do if it is gone).
    Remove(PathBuf),
    /// Remove a directory if it is empty.
    RemoveDirIfEmpty(PathBuf),
    /// Enable the new keeper, and start it if the mount service (`service`) is running.
    EnableKeeper {
        /// The mount's systemd unit.
        service: String,
    },
}

impl Step {
    fn sysctl(args: &[&str], best_effort: bool) -> Step {
        Step::Systemctl {
            args: args.iter().map(ToString::to_string).collect(),
            best_effort,
        }
    }
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Systemctl { args, best_effort } => {
                let args = args.join(" ");
                if *best_effort {
                    f.write_str(&fl!("migrate-step-systemctl-best-effort", args = args))
                } else {
                    f.write_str(&fl!("migrate-step-systemctl", args = args))
                }
            }
            Step::WaitQuiet { .. } => f.write_str(&fl!("migrate-step-wait-quiet")),
            Step::EnableKeeper { .. } => f.write_str(&fl!("migrate-step-enable-keeper")),
            Step::Rename { from, to } => f.write_str(&fl!(
                "migrate-step-rename",
                from = from.display().to_string(),
                to = to.display().to_string()
            )),
            Step::Write { path, .. }
            | Step::MergeConfig { path, .. }
            | Step::RenamePlasmoid(path) => f.write_str(&fl!(
                "migrate-step-write",
                path = path.display().to_string()
            )),
            Step::Remove(p) => {
                f.write_str(&fl!("migrate-step-remove", path = p.display().to_string()))
            }
            Step::RemoveDirIfEmpty(p) => f.write_str(&fl!(
                "migrate-step-remove-dir",
                path = p.display().to_string()
            )),
        }
    }
}

/// What a migration would do, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// The old install the settings and offline folders come from, if any.
    pub source: Option<Origin>,
    /// Things worth telling the user (another old install left alone, old config kept).
    pub notes: Vec<String>,
    /// The steps; the last one writes the marker.
    pub steps: Vec<Step>,
    /// The marker already exists: nothing to do.
    pub done: bool,
    /// The journal to use; `journal_new` says it must be written before the first step.
    journal: Option<Journal>,
    journal_new: bool,
}

impl std::fmt::Display for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.done {
            return writeln!(f, "{}", fl!("migrate-already-done"));
        }
        match self.source {
            Some(o) => writeln!(f, "{}", fl!("migrate-plan-header", origin = o.label()))?,
            None => writeln!(f, "{}", fl!("migrate-plan-header-none"))?,
        }
        for n in &self.notes {
            writeln!(f, "  {}", fl!("migrate-plan-note", note = n.as_str()))?;
        }
        for (i, s) in self.steps.iter().enumerate() {
            writeln!(f, "  {:>2}. {s}", i + 1)?;
        }
        Ok(())
    }
}

/// The old install to migrate, and the ones to leave alone. An interrupted run's journal says
/// which one it was.
fn choose<'a>(
    old: &'a [OldFacts],
    journal: Option<&Journal>,
) -> Result<Option<(&'a OldFacts, Vec<&'a OldFacts>)>, MigrationFailure> {
    let from_journal = journal
        .and_then(Journal::origin)
        .and_then(|o| old.iter().find(|f| f.origin == o));
    let pick: Option<&OldFacts> = match (from_journal, old) {
        (Some(j), _) => Some(j),
        (None, []) => None,
        (None, [one]) => Some(one),
        (None, many) => {
            let with_offline: Vec<&OldFacts> = many.iter().filter(|o| o.has_offline()).collect();
            match with_offline.as_slice() {
                [] => many.iter().max_by_key(|o| o.config_mtime),
                [one] => Some(*one),
                several => {
                    let enabled: Vec<&OldFacts> = several
                        .iter()
                        .copied()
                        .filter(|o| o.keeper_enabled)
                        .collect();
                    match enabled.as_slice() {
                        [one] => Some(*one),
                        _ => {
                            return Err(MigrationFailure::new(
                                MigrateError::Ambiguous,
                                fl!("migrate-error-ambiguous"),
                            ))
                        }
                    }
                }
            }
        }
    };
    Ok(pick.map(|p| (p, old.iter().filter(|o| o.origin != p.origin).collect())))
}

fn conflict(what: &Path) -> MigrationFailure {
    MigrationFailure::new(
        MigrateError::Conflict,
        fl!("migrate-error-conflict", path = what.display().to_string()),
    )
}

/// Work out what to do. Pure: it only looks at `facts`.
fn build_plan(env: &Env, facts: &Facts) -> Result<Plan, MigrationFailure> {
    if facts.marker {
        return Ok(Plan {
            done: true,
            ..Plan::default()
        });
    }
    let mut plan = Plan::default();
    let mut steps = Vec::new();
    if let Some((old, others)) = choose(&facts.old, facts.journal.as_ref())? {
        plan.source = Some(old.origin);
        for other in others {
            plan.notes.push(fl!(
                "migrate-note-other-left",
                origin = other.origin.label()
            ));
        }
        let journal = facts.journal.clone().unwrap_or_else(|| Journal {
            source: old.origin.code().into(),
            keeper_was_enabled: old.keeper_enabled,
            keeper_was_active: old.keeper_active,
            offline_enabled: old.config.as_ref().is_some_and(|c| c.offline_enabled)
                || old.keeper_enabled,
            had_units: old.has_units(),
            service: old.config.clone().unwrap_or_default().service,
        });
        plan.journal_new = facts.journal.is_none();
        old_install_steps(env, facts, old, &journal, &mut plan, &mut steps)?;
        plan.journal = Some(journal);
    }
    // Panel entries are renamed whichever install was chosen: the old applet id is gone.
    for (path, text) in &facts.panels {
        steps.push(Step::Write {
            path: path.clone(),
            content: text.replace(
                &format!("\"{OLD_COSMIC_APP_ID}\""),
                &format!("\"{COSMIC_DESKTOP_ID}\""),
            ),
        });
    }
    if let Some(path) = &facts.plasma_applets {
        steps.push(Step::RenamePlasmoid(path.clone()));
    }
    steps.push(Step::Write {
        path: env.marker(),
        content: "migrated from the old per-front-end identities\n".into(),
    });
    plan.steps = steps;
    Ok(plan)
}

#[allow(clippy::too_many_lines)] // one straight sequence; splitting it would hide the order
fn old_install_steps(
    env: &Env,
    facts: &Facts,
    old: &OldFacts,
    journal: &Journal,
    plan: &mut Plan,
    steps: &mut Vec<Step>,
) -> Result<(), MigrationFailure> {
    let origin = old.origin;
    let unit_dir = env.unit_dir();
    let old_data = env.data_home.join(origin.slug());
    let old_state = env.state_home.join(origin.slug());
    let (new_data, new_state) = (env.new_data(), env.new_state());

    // Refuse before changing anything.
    if old.data_dir && facts.new_data_exists && !facts.new_data_empty {
        return Err(conflict(&new_data));
    }
    if old.state_dir && facts.new_state_exists && !facts.new_state_empty {
        return Err(conflict(&new_state));
    }
    for (unit, text) in [
        (origin.keeper_unit(), &old.keeper_unit),
        (origin.pin_template(), &old.pin_template),
    ] {
        if let Some(text) = text {
            if !text.starts_with(&format!("# Written by {}", origin.slug())) {
                return Err(MigrationFailure::new(
                    MigrateError::Conflict,
                    fl!(
                        "migrate-error-unit-not-ours",
                        path = unit_dir.join(unit).display().to_string()
                    ),
                ));
            }
        }
    }
    let units_wanted = old.has_units() || journal.had_units;
    if units_wanted && env.exe.as_os_str().is_empty() {
        return Err(fl!("migrate-error-no-exe").into());
    }
    let (old_flat, new_flat) = (
        format!("{}_", flat(&old_data.join("offline"))),
        format!("{}_", flat(&new_data.join("offline"))),
    );
    let listing_moves: Vec<(String, String)> = old
        .bisync_files
        .iter()
        .filter(|n| n.contains(&old_flat))
        .map(|n| (n.clone(), n.replace(&old_flat, &new_flat)))
        .collect();
    for (_, new_name) in &listing_moves {
        if new_name.len() > FILE_NAME_MAX {
            return Err(fl!("migrate-error-name-too-long", name = new_name.as_str()).into());
        }
        if old.bisync_files.contains(new_name) && old.state_dir {
            return Err(fl!("migrate-error-listing-exists", name = new_name.as_str()).into());
        }
    }

    // 1. Stop the old services so nothing writes while things move (the keeper first, or it
    // would start the pin mounts again). Unconditional: a keeper that is "activating" or about
    // to be restarted by systemd is not "active" but must stop too.
    if old.keeper_unit.is_some() {
        let keeper = origin.keeper_unit();
        steps.push(Step::sysctl(&["stop", &keeper], false));
        steps.push(Step::sysctl(&["disable", &keeper], true));
        steps.push(Step::sysctl(&["reset-failed", &keeper], true));
    }
    if old.pin_template.is_some() {
        let pins = format!("{}-pin@*.service", origin.slug());
        steps.push(Step::sysctl(&["stop", &pins], false));
        steps.push(Step::sysctl(&["reset-failed", &pins], true));
    }
    // 1b. The keeper's bisync is an async job inside the mount's rclone, which stopping the
    // unit doesn't stop; and pin mounts can outlive their unit for a moment.
    if old.data_dir || old.state_dir {
        steps.push(Step::WaitQuiet {
            cfg: Box::new(old.config.clone().unwrap_or_default()),
            old_data: old_data.clone(),
        });
    }

    // 2. Move the data and state, then the baseline files whose names contain the data path.
    if old.data_dir {
        steps.push(Step::Rename {
            from: old_data,
            to: new_data,
        });
    }
    if old.state_dir {
        steps.push(Step::Rename {
            from: old_state.clone(),
            to: new_state.clone(),
        });
    }
    let state_after = new_state;
    for (from, to) in listing_moves {
        steps.push(Step::Rename {
            from: state_after.join("bisync").join(from),
            to: state_after.join("bisync").join(to),
        });
    }
    // The pin mounts' vfs caches are keyed by the same path.
    for prefix in &old.cache_dirs {
        let base = state_after.join(prefix);
        steps.push(Step::Rename {
            from: base.join(slashless(&env.data_home.join(origin.slug()))),
            to: base.join(slashless(&env.new_data())),
        });
    }

    // 3. The settings.
    if let Some(cfg) = &old.config {
        let path = config::config_path_in(&env.config_home, SLUG);
        if facts.new_config_corrupt {
            return Err(conflict(&path));
        }
        if facts.new_config {
            plan.notes.push(fl!("migrate-note-config-merged"));
        }
        steps.push(Step::MergeConfig {
            path,
            old: Box::new(cfg.clone()),
        });
        plan.notes.push(fl!("migrate-note-old-config-left"));
    }

    // 4. The units: new ones in, old ones out. Written every time (same content), so a run
    // that resumes after the old ones are gone still ends with the new ones in place.
    if units_wanted {
        let cfg = old.config.clone().unwrap_or_default();
        let exe = env.exe.to_string_lossy();
        steps.push(Step::Write {
            path: unit_dir.join(units::keeper_unit_name()),
            content: units::keeper_unit(&exe, &cfg.service),
        });
        steps.push(Step::Write {
            path: unit_dir.join(units::pin_template_name()),
            content: units::pin_unit(&exe, &cfg.service),
        });
        steps.push(Step::Write {
            path: units::dropin_path_in(&unit_dir, &cfg.service),
            content: units::dropin(),
        });
        if old.keeper_unit.is_some() {
            steps.push(Step::Remove(unit_dir.join(origin.keeper_unit())));
        }
        if old.pin_template.is_some() {
            steps.push(Step::Remove(unit_dir.join(origin.pin_template())));
        }
        if let Some(d) = &old.dropin {
            steps.push(Step::Remove(d.clone()));
            if let Some(dir) = d.parent() {
                steps.push(Step::RemoveDirIfEmpty(dir.to_path_buf()));
            }
        }
        steps.push(Step::sysctl(&["daemon-reload"], false));
        // From the journal, not from systemd now: a resumed run sees the half-moved state.
        if journal.offline_enabled {
            steps.push(Step::EnableKeeper {
                service: cfg.service.clone(),
            });
        } else if journal.keeper_was_active {
            steps.push(Step::sysctl(&["start", &units::keeper_unit_name()], false));
        }
    }
    Ok(())
}

/// Look at the system and say what [`run`] would do. Changes nothing (it only asks systemd
/// questions that don't change anything).
///
/// # Errors
/// A failure if the old install is in a state the migration won't touch.
pub fn plan(env: &Env) -> Result<Plan, MigrationFailure> {
    build_plan(env, &gather(env)?)
}

/// The octal escapes (`\040` for a space) `/proc/self/mountinfo` uses in paths.
fn unescape_octal(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            let v = (u32::from(b[i + 1] - b'0') << 6)
                | (u32::from(b[i + 2] - b'0') << 3)
                | u32::from(b[i + 3] - b'0');
            out.push(u8::try_from(v & 0xff).unwrap_or(b'?'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Is a FUSE mount still sitting on the old offline data: one whose source is under `old_data`
/// or one mounted somewhere inside the Drive mount (`mount_path`), which is where pins go?
fn offline_mount_left(mountinfo: &str, old_data: &Path, mount_path: &str) -> Option<String> {
    let old_data = old_data.to_string_lossy();
    let drive = mount_path.trim_end_matches('/');
    mountinfo.lines().find_map(|l| {
        let (left, right) = l.split_once(" - ")?;
        let mountpoint = unescape_octal(left.split_whitespace().nth(4)?);
        let mut r = right.split_whitespace();
        let (fstype, source) = (r.next()?, unescape_octal(r.next()?));
        let ours = fstype.starts_with("fuse")
            && (source.contains(old_data.as_ref())
                || (!drive.is_empty() && mountpoint.starts_with(&format!("{drive}/"))));
        ours.then_some(mountpoint)
    })
}

/// Wait for rclone's jobs and the pin mounts to be gone. Only "nothing listens" counts as no
/// jobs; an rc server that answers badly (401, timeout, garbage) is still running something we
/// can't see, so that is waited out too and ends as [`MigrateError::SyncInFlight`].
fn wait_quiet(env: &Env, cfg: &Config, old_data: &Path) -> Result<(), MigrationFailure> {
    let mount_path = config::expand(&cfg.mount_path);
    let tries = (env.quiet_timeout.as_millis() / env.poll.as_millis().max(1)).max(1);
    let (mut jobs, mut unknown, mut mount) = (0, false, None);
    for i in 0..tries {
        (jobs, unknown) = match (env.unfinished_jobs)(cfg) {
            Ok(n) => (n, false),
            Err(RcProbeError::NotRunning) => (0, false),
            Err(RcProbeError::Other(_)) => (0, true),
        };
        mount = offline_mount_left(&(env.mountinfo)(), old_data, &mount_path);
        if jobs == 0 && !unknown && mount.is_none() {
            return Ok(());
        }
        if i + 1 < tries {
            (env.sleep)(env.poll);
        }
    }
    Err(if jobs > 0 {
        MigrationFailure::new(
            MigrateError::SyncInFlight,
            fl!("migrate-error-sync-in-flight", count = jobs),
        )
    } else if unknown {
        MigrationFailure::new(
            MigrateError::SyncInFlight,
            fl!("migrate-error-sync-unknown"),
        )
    } else {
        MigrationFailure::new(
            MigrateError::MountInUse,
            fl!(
                "migrate-error-mount-in-use",
                path = mount.unwrap_or_default()
            ),
        )
    })
}

/// Put the old keeper back the way the journal says it was, for a run that failed before it
/// changed anything. Like `EnableKeeper`, the keeper is started when it was enabled and the mount
/// service is running (a keeper that was only enabled still gets started by the mount coming up);
/// it brings the pin mounts back itself. Returns what failed, so the caller can say so.
fn restore_old_keeper(env: &Env, origin: Origin, journal: &Journal) -> Vec<String> {
    let keeper = origin.keeper_unit();
    let mount_up = env.sysctl(&["is-active", &journal.service()]).is_ok();
    let mut failures = Vec::new();
    let mut run = |args: &[&str]| {
        if let Err(e) = env.sysctl(args) {
            failures.push(e);
        }
    };
    if journal.keeper_was_enabled {
        run(&["enable", &keeper]);
    }
    if journal.keeper_was_active || (journal.keeper_was_enabled && mount_up) {
        run(&["start", &keeper]);
    }
    failures
}

/// `plugin=<old id>` and comma-separated lists (`extraItems=a,b,<old id>`) in plasmashell's applet
/// list, with the old plasmoid id replaced by the new one. Whole entries only, so an id that
/// merely contains the old one is left alone; line endings are kept.
fn rename_plasmoid_id(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches(['\n', '\r']);
            let ending = &line[body.len()..];
            match body.split_once('=') {
                Some((key, value)) if value.split(',').any(|v| v == OLD_PLASMOID_ID) => {
                    let new: Vec<&str> = value
                        .split(',')
                        .map(|v| if v == OLD_PLASMOID_ID { APP_ID } else { v })
                        .collect();
                    format!("{key}={}{ending}", new.join(","))
                }
                _ => line.to_string(),
            }
        })
        .collect()
}

/// Apply [`rename_plasmoid_id`] to the file as it is now, keeping its permissions (it is `0600`).
fn rewrite_applets(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let new = rename_plasmoid_id(&text);
    if new == text {
        return Ok(());
    }
    let mode = std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o7777);
    config::write_atomic_mode(path, &new, mode).map_err(|e| e.to_string())
}

/// Carry out one step.
fn execute_step(env: &Env, step: &Step, log: &mut Vec<String>) -> Result<(), MigrationFailure> {
    match step {
        Step::Systemctl { args, best_effort } => {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            match env.sysctl(&refs) {
                Err(e) if !best_effort => return Err(e.into()),
                Err(e) => log.push(fl!("migrate-log-ignored", error = e)),
                Ok(_) => {}
            }
        }
        Step::WaitQuiet { cfg, old_data } => wait_quiet(env, cfg, old_data)?,
        Step::EnableKeeper { service } => {
            let keeper = units::keeper_unit_name();
            env.sysctl(&["enable", &keeper])?;
            if env.sysctl(&["is-active", service]).is_ok() {
                env.sysctl(&["start", &keeper])?;
            }
        }
        Step::Rename { from, to } => match std::fs::rename(from, to) {
            Ok(()) => {}
            // Moved by an earlier, interrupted run.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && to.exists() => {}
            // An old cache directory that was never created is not a problem.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log.push(fl!(
                    "migrate-log-skipped",
                    path = from.display().to_string()
                ));
            }
            Err(e) => return Err(format!("{} -> {}: {e}", from.display(), to.display()).into()),
        },
        Step::MergeConfig { path, old } => {
            let existed = path.exists();
            TomlStore::new(path.clone())
                .save_unguarded(&mut |c| {
                    if existed {
                        // A front end that ran meanwhile may have saved its own values; the
                        // old install's settings are what the user chose.
                        c.rc_addr.clone_from(&old.rc_addr);
                        c.env_file.clone_from(&old.env_file);
                        c.service.clone_from(&old.service);
                        c.mount_path.clone_from(&old.mount_path);
                        c.cache_dir.clone_from(&old.cache_dir);
                        c.offline_enabled = old.offline_enabled;
                        c.pins.clone_from(&old.pins);
                        c.sync_interval_secs = old.sync_interval_secs;
                    } else {
                        *c = Config::clone(old);
                    }
                })
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Step::RenamePlasmoid(path) => {
            if let Err(error) = rewrite_applets(path) {
                let line = fl!(
                    "migrate-log-panel-kept",
                    path = path.display().to_string(),
                    error = error
                );
                eprintln!("{line}");
                log.push(line);
            }
        }
        Step::Write { path, content } => {
            config::write_atomic(path, content).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Step::Remove(p) => match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", p.display()).into()),
        },
        Step::RemoveDirIfEmpty(p) => match std::fs::remove_dir(p) {
            Ok(()) => {}
            // Other drop-ins in it, or already gone: expected.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(e) => {
                let line = fl!(
                    "migrate-log-dir-kept",
                    path = p.display().to_string(),
                    error = e.to_string()
                );
                eprintln!("{line}");
                log.push(line);
            }
        },
    }
    Ok(())
}

/// Plan and execute the migration. Returns the lines to show the user.
///
/// # Errors
/// A failure if the plan couldn't be made (nothing was changed) or a step failed (running it
/// again resumes where it stopped).
pub fn run(env: &Env) -> Result<Vec<String>, MigrationFailure> {
    run_limited(env, None)
}

/// [`run`], stopping with an error after `limit` steps (tests interrupt it this way).
fn run_limited(env: &Env, limit: Option<usize>) -> Result<Vec<String>, MigrationFailure> {
    let plan = plan(env)?;
    if plan.done {
        // A run that died between the marker and the cleanup.
        let _ = std::fs::remove_file(env.journal_path());
        return Ok(vec![]);
    }
    // Before the first change: what the old install was like, for a run that resumes.
    if let (Some(j), true) = (&plan.journal, plan.journal_new) {
        let text = serde_json::to_string_pretty(j).map_err(|e| e.to_string())?;
        config::write_atomic(&env.journal_path(), &text)
            .map_err(|e| format!("{}: {e}", env.journal_path().display()))?;
    }
    let mut log = Vec::new();
    // Until something is moved, written or removed the old install is intact apart from the
    // stopped services, so a failure puts those back instead of leaving the old keeper off.
    let mut changed = false;
    for (i, step) in plan.steps.iter().enumerate() {
        if limit.is_some_and(|l| i >= l) {
            return Err(String::from("interrupted").into());
        }
        changed |= !matches!(step, Step::Systemctl { .. } | Step::WaitQuiet { .. });
        let number: usize = i + 1;
        if let Err(e) = execute_step(env, step, &mut log) {
            if let (false, true, Some(o), Some(j)) = (
                changed,
                plan.journal_new,
                plan.source,
                plan.journal.as_ref(),
            ) {
                let failures = restore_old_keeper(env, o, j);
                // Forget the journal, so a later attempt starts from what is then true.
                let _ = std::fs::remove_file(env.journal_path());
                let message = if matches!(step, Step::WaitQuiet { .. }) {
                    e.message
                } else {
                    fl!(
                        "migrate-error-step-restored",
                        number = number,
                        total = plan.steps.len(),
                        step = step.to_string(),
                        error = e.message
                    )
                };
                let message = if failures.is_empty() {
                    message
                } else {
                    fl!(
                        "migrate-error-restore-failed",
                        message = message,
                        errors = failures.join("; ")
                    )
                };
                return Err(MigrationFailure::new(e.code, message));
            }
            return Err(MigrationFailure::new(
                e.code,
                fl!(
                    "migrate-error-step-failed",
                    number = number,
                    total = plan.steps.len(),
                    step = step.to_string(),
                    error = e.message
                ),
            ));
        }
    }
    let _ = std::fs::remove_file(env.journal_path());
    let _ = std::fs::remove_file(env.error_path());
    let line = |text: &str| fl!("migrate-line", text = text);
    let mut out = vec![line(&plan.source.map_or_else(
        || fl!("migrate-done-nothing"),
        |o| fl!("migrate-done-from", origin = o.label()),
    ))];
    out.extend(plan.notes.iter().map(|n| line(n)));
    out.extend(log.iter().map(|l| line(l)));
    Ok(out)
}

// ---- the lock, and who may migrate when ----

/// Holds the migration lock (an `flock` on a sidecar file in the state directory) until dropped.
/// The kernel releases it when the process exits, however it exits, so there is nothing to
/// clean up and no stale lock to steal. The file itself is never deleted.
#[derive(Debug)]
struct LockGuard(#[allow(dead_code)] std::fs::File);

/// How a process that wants to migrate treats a lock somebody else holds.
#[derive(Debug, Clone, Copy)]
enum Wait {
    /// Try once and give up (background threads: they show "pending" meanwhile).
    Poll,
    /// Wait up to `max` for the holder to finish; `progress` says so on stderr every ten seconds.
    Block { max: Duration, progress: bool },
}

/// What [`acquire_wait`] came back with.
#[derive(Debug)]
enum Lock {
    Held(LockGuard),
    /// The marker appeared meanwhile: nothing left to do.
    Done,
    /// Somebody else still holds it.
    Busy,
}

fn open_lock(env: &Env) -> Result<std::fs::File, MigrationFailure> {
    let path = env.lock_path();
    let io = |e: std::io::Error| MigrationFailure::from(format!("{}: {e}", path.display()));
    std::fs::create_dir_all(&env.state_home).map_err(io)?;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(io)
}

/// Take the lock, waiting as `wait` says.
fn acquire_wait(env: &Env, wait: Wait) -> Result<Lock, MigrationFailure> {
    use std::fs::TryLockError;
    let file = open_lock(env)?;
    let (max, progress) = match wait {
        Wait::Poll => (Duration::ZERO, false),
        Wait::Block { max, progress } => (max, progress),
    };
    let poll = env.poll.max(Duration::from_millis(1));
    let mut waited = Duration::ZERO;
    let mut since_note = LOCK_PROGRESS_EVERY;
    loop {
        if env.marker().exists() {
            return Ok(Lock::Done);
        }
        match file.try_lock() {
            Ok(()) => return Ok(Lock::Held(LockGuard(file))),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => {
                return Err(format!("{}: {e}", env.lock_path().display()).into())
            }
        }
        if waited >= max {
            return Ok(Lock::Busy);
        }
        if progress && since_note >= LOCK_PROGRESS_EVERY {
            eprintln!("{}", fl!("migrate-waiting-lock"));
            since_note = Duration::ZERO;
        }
        (env.sleep)(poll);
        waited += poll;
        since_note += poll;
    }
}

/// Remember why a migration failed, for [`pending`].
fn record_failure(env: &Env, e: &MigrationFailure) {
    let _ = config::write_atomic(&env.error_path(), e.code.code());
}

/// [`run`] under the lock. A process that finds the lock taken waits for the other to finish
/// (or, with [`Wait::Poll`], leaves it to the other) and re-checks the marker. The reason for a
/// failure is kept for [`pending`].
fn run_locked(env: &Env, wait: Wait) -> Result<Vec<String>, MigrationFailure> {
    let _guard = match acquire_wait(env, wait)? {
        Lock::Held(g) => g,
        Lock::Done => return Ok(vec![]),
        Lock::Busy if matches!(wait, Wait::Poll) => return Ok(vec![]),
        Lock::Busy => {
            return Err(MigrationFailure::new(
                MigrateError::LockTimeout,
                fl!("migrate-error-lock-timeout"),
            ))
        }
    };
    let result = run(env);
    match &result {
        Ok(_) => {
            let _ = std::fs::remove_file(env.error_path());
        }
        Err(e) => record_failure(env, e),
    }
    result
}

/// [`plan`], remembering a failure (conflict, ambiguous) for [`pending`] like a failed run.
fn planned(env: &Env) -> Result<Plan, MigrationFailure> {
    plan(env).inspect_err(|e| record_failure(env, e))
}

/// How long a foreground process waits for another migration to finish.
fn foreground_wait(env: &Env) -> Wait {
    Wait::Block {
        max: env.quiet_timeout + LOCK_EXTRA_WAIT,
        progress: true,
    }
}

/// Is an old install here that hasn't been migrated? Looks at files only.
fn old_install_present(config_home: &Path, data_home: &Path, state_home: &Path) -> bool {
    let units = units::unit_dir_in(config_home);
    config_home.join(OLD_PLASMA_CONFIG).exists()
        || config_home.join("cosmic").join(OLD_COSMIC_APP_ID).exists()
        || [OLD_COSMIC_SLUG, OLD_PLASMA_SLUG].iter().any(|s| {
            data_home.join(s).exists()
                || state_home.join(s).exists()
                || units.join(format!("{s}-offline.service")).exists()
                || units.join(format!("{s}-pin@.service")).exists()
        })
}

/// [`pending`] for explicit roots.
#[must_use]
pub fn pending_in(config_home: &Path, data_home: &Path, state_home: &Path) -> MigrationStatus {
    if state_home.join(SLUG).join(MARKER).exists()
        || !old_install_present(config_home, data_home, state_home)
    {
        return MigrationStatus::NotNeeded;
    }
    match read_opt(&state_home.join(format!("{SLUG}-migrate-error"))) {
        Some(code) => MigrationStatus::Failed(MigrateError::from_code(&code)),
        None => MigrationStatus::Pending,
    }
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(config::expand(fallback)), PathBuf::from)
}

fn skip() -> bool {
    cfg!(test)
        || crate::identity::is_test_run()
        || std::env::var_os(SKIP_ENV).is_some()
        || crate::identity::try_identity().is_none_or(crate::identity::Identity::is_dev)
}

/// Does an old install still wait to be migrated? Cheap (a few `stat`s) and changes nothing:
/// front ends call it from places that must not block (D-Bus activation, the applet's start-up).
#[must_use]
pub fn pending() -> MigrationStatus {
    if skip() {
        return MigrationStatus::NotNeeded;
    }
    pending_in(
        &config::xdg_config_home(),
        &xdg_dir("XDG_DATA_HOME", "~/.local/share"),
        &xdg_dir("XDG_STATE_HOME", "~/.local/state"),
    )
}

/// Refuse to switch offline folders on or pin anything while an old install is waiting to be
/// moved: the new keeper and the old one would fight over the same folders.
///
/// # Errors
/// [`MigrateError::OldInstallPresent`] with an explanation.
pub fn guard() -> Result<(), MigrationFailure> {
    guard_status(pending())
}

pub(crate) fn guard_status(status: MigrationStatus) -> Result<(), MigrationFailure> {
    match status {
        MigrationStatus::NotNeeded => Ok(()),
        _ => Err(MigrationFailure::new(
            MigrateError::OldInstallPresent,
            fl!("migrate-error-old-install-present"),
        )),
    }
}

/// Run `f` with an [`Env`] built from this process: the real XDG directories and the real
/// `systemctl --user`. Only the entry points below use it.
fn with_process_env<T>(f: impl FnOnce(&Env) -> T) -> T {
    let runner = |args: &[&str]| {
        let out = std::process::Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .map_err(|e| format!("systemctl: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!(
                "systemctl {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    };
    let jobs = crate::rc::unfinished_jobs;
    let mountinfo = || std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let env = Env {
        config_home: config::xdg_config_home(),
        data_home: xdg_dir("XDG_DATA_HOME", "~/.local/share"),
        state_home: xdg_dir("XDG_STATE_HOME", "~/.local/state"),
        systemctl: &runner,
        exe: units::shared_exe().unwrap_or_default(),
        unfinished_jobs: &jobs,
        mountinfo: &mountinfo,
        sleep: &std::thread::sleep,
        quiet_timeout: Duration::from_secs(120),
        poll: Duration::from_secs(2),
    };
    f(&env)
}

/// Migrate now if an old install is waiting, in the foreground, printing what happened on
/// stderr. For the command-line modes: the caller exits non-zero on `Err`. Does nothing for the
/// dev build and in tests. If another process is migrating, waits for it (with a progress line
/// every ten seconds) for up to the sync wait plus three minutes.
///
/// # Errors
/// Why the migration failed (already printed).
pub fn run_if_needed() -> Result<(), MigrationFailure> {
    migrate_now(true)
}

fn migrate_now(foreground: bool) -> Result<(), MigrationFailure> {
    if skip() || (pending() == MigrationStatus::NotNeeded && !xdg_state_marker_missing()) {
        return Ok(());
    }
    let result = with_process_env(|env| {
        let wait = if foreground {
            foreground_wait(env)
        } else {
            Wait::Poll
        };
        run_locked(env, wait)
    });
    match result {
        Ok(lines) => {
            for l in &lines {
                eprintln!("{l}");
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("{}", fl!("migrate-failed", error = e.message.as_str()));
            Err(e)
        }
    }
}

/// With no old install the marker is still written once (so later starts are one `stat`).
fn xdg_state_marker_missing() -> bool {
    !xdg_dir("XDG_STATE_HOME", "~/.local/state")
        .join(SLUG)
        .join(MARKER)
        .exists()
}

/// Start the migration on a background thread, if one is needed. For processes that must not
/// block on it (the Plasma daemon after it owns its bus name, the COSMIC applet): they show
/// [`pending`] meanwhile. If another process holds the migration lock this one leaves the work
/// to it. Errors are printed on stderr and kept for [`pending`].
pub fn spawn_background() -> Option<std::thread::JoinHandle<()>> {
    if pending() == MigrationStatus::NotNeeded {
        return None;
    }
    Some(std::thread::spawn(|| {
        let _ = migrate_now(false);
    }))
}

/// `--migrate [--dry-run]`: print the plan, or carry it out. Returns the text to show.
///
/// # Errors
/// The failure to print on stderr (exit code 1).
pub fn command(dry_run: bool) -> Result<String, MigrationFailure> {
    refuse_dev(crate::identity::identity().is_dev())?;
    with_process_env(|env| {
        if dry_run {
            plan(env).map(|p| p.to_string())
        } else {
            let plan = planned(env)?;
            if plan.done {
                return Ok(plan.to_string());
            }
            let mut text = String::new();
            for line in run_locked(env, foreground_wait(env))? {
                let _ = writeln!(text, "{line}");
            }
            Ok(text.trim_end().to_string())
        }
    })
}

/// The dev build keeps its own state and must never move the real install's.
fn refuse_dev(is_dev: bool) -> Result<(), MigrationFailure> {
    if is_dev {
        Err(MigrationFailure::new(
            MigrateError::DevBuild,
            fl!("migrate-error-dev-build"),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
