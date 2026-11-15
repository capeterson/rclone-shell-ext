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

use crate::config::{self, Config};
use crate::identity::{COSMIC_DESKTOP_ID, SLUG};
use crate::units;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
/// bisync's name for a file is limited by the file system.
const FILE_NAME_MAX: usize = 255;

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
}

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
    active_pins: Vec<String>,
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
    new_data_exists: bool,
    new_data_empty: bool,
    new_state_exists: bool,
    new_state_empty: bool,
    /// COSMIC panel config files that list the old applet, with their text.
    panels: Vec<(PathBuf, String)>,
}

fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_none())
}

fn read_opt(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

/// bisync's name for a path: no leading slash, separators flattened.
fn flat(p: &Path) -> String {
    p.to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "_")
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
    let active_pins = if pin_template.is_some() {
        let glob = format!("{}-pin@*.service", origin.slug());
        env.sysctl(&["list-units", "--plain", "--no-legend", &glob])
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|n| n.ends_with(".service") && n.contains("-pin@"))
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
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
        active_pins,
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
    let old = [Origin::Cosmic, Origin::Plasma]
        .into_iter()
        .map(|o| gather_old(env, o))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(OldFacts::present)
        .collect();
    let (data, state) = (env.new_data(), env.new_state());
    Ok(Facts {
        marker: env.marker().exists(),
        old,
        new_config: config::config_path_in(&env.config_home, SLUG).exists(),
        new_data_exists: data.exists(),
        new_data_empty: is_empty_dir(&data),
        new_state_exists: state.exists(),
        // The marker's own directory counts as empty for this purpose only before it is written.
        new_state_empty: is_empty_dir(&state),
        panels,
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
    /// Remove a file (nothing to do if it is gone).
    Remove(PathBuf),
    /// Remove a directory if it is empty.
    RemoveDirIfEmpty(PathBuf),
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
            Step::Systemctl { args, best_effort } => write!(
                f,
                "systemctl --user {}{}",
                args.join(" "),
                if *best_effort {
                    "  (ignored if it fails)"
                } else {
                    ""
                }
            ),
            Step::Rename { from, to } => write!(f, "rename {} -> {}", from.display(), to.display()),
            Step::Write { path, .. } => write!(f, "write {}", path.display()),
            Step::Remove(p) => write!(f, "remove {}", p.display()),
            Step::RemoveDirIfEmpty(p) => write!(f, "remove {} if empty", p.display()),
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
}

impl std::fmt::Display for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.done {
            return writeln!(f, "Already migrated: nothing to do.");
        }
        match self.source {
            Some(o) => writeln!(
                f,
                "Migrating the old {} install to rclone-shell-ext:",
                o.label()
            )?,
            None => writeln!(f, "No old install found; only the marker file is written:")?,
        }
        for n in &self.notes {
            writeln!(f, "  note: {n}")?;
        }
        for (i, s) in self.steps.iter().enumerate() {
            writeln!(f, "  {:>2}. {s}", i + 1)?;
        }
        Ok(())
    }
}

/// The old install to migrate, and the ones to leave alone.
fn choose(old: &[OldFacts]) -> Result<Option<(&OldFacts, Vec<&OldFacts>)>, String> {
    let pick: Option<&OldFacts> = match old {
        [] => None,
        [one] => Some(one),
        many => {
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
                            return Err(
                                "both the old COSMIC and the old Plasma installs have offline \
                                 folders set up, and it isn't clear which one is in use; turn \
                                 offline folders off in the one you don't use, then try again"
                                    .into(),
                            )
                        }
                    }
                }
            }
        }
    };
    Ok(pick.map(|p| (p, old.iter().filter(|o| o.origin != p.origin).collect())))
}

fn conflict(what: &Path) -> String {
    format!(
        "{} already exists and isn't empty, so the old data can't be moved there; move it aside \
         or remove it, then try again. Nothing was changed.",
        what.display()
    )
}

/// Work out what to do. Pure: it only looks at `facts`.
fn build_plan(env: &Env, facts: &Facts) -> Result<Plan, String> {
    if facts.marker {
        return Ok(Plan {
            done: true,
            ..Plan::default()
        });
    }
    let mut plan = Plan::default();
    let mut steps = Vec::new();
    if let Some((old, others)) = choose(&facts.old)? {
        plan.source = Some(old.origin);
        for other in others {
            plan.notes.push(format!(
                "an old {} install was also found; it is left untouched",
                other.origin.label()
            ));
        }
        old_install_steps(env, facts, old, &mut plan, &mut steps)?;
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
    plan: &mut Plan,
    steps: &mut Vec<Step>,
) -> Result<(), String> {
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
                return Err(format!(
                    "{} isn't one this app wrote, so it won't be replaced; remove it yourself \
                     if it is no longer needed. Nothing was changed.",
                    unit_dir.join(unit).display()
                ));
            }
        }
    }
    if old.has_units() && env.exe.as_os_str().is_empty() {
        return Err(format!(
            "can't find the `{SLUG}` program (next to this one or on PATH), which the new \
             services must run. Nothing was changed."
        ));
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
            return Err(format!(
                "bisync's working file {new_name} would be too long for the file system under \
                 the new name. Nothing was changed."
            ));
        }
        if old.bisync_files.contains(new_name) && old.state_dir {
            return Err(format!(
                "bisync's working files already include {new_name}. Nothing was changed."
            ));
        }
    }

    // 1. Stop the old services so nothing writes while things move (the keeper first, or it
    // would start the pin mounts again).
    if old.keeper_active {
        steps.push(Step::sysctl(&["stop", &origin.keeper_unit()], false));
    }
    for pin in &old.active_pins {
        steps.push(Step::sysctl(&["stop", pin], false));
    }
    if old.keeper_enabled {
        steps.push(Step::sysctl(&["disable", &origin.keeper_unit()], true));
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
        if facts.new_config {
            plan.notes
                .push("a new config.toml already exists; it is kept as is".into());
        } else {
            steps.push(Step::Write {
                path: config::config_path_in(&env.config_home, SLUG),
                content: toml::to_string_pretty(cfg).map_err(|e| e.to_string())?,
            });
        }
        plan.notes
            .push("the old config file is left in place (delete it when you are happy)".into());
    }

    // 4. The units: new ones in, old ones out.
    if old.has_units() {
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
        steps.push(Step::Remove(unit_dir.join(origin.keeper_unit())));
        steps.push(Step::Remove(unit_dir.join(origin.pin_template())));
        if let Some(d) = &old.dropin {
            steps.push(Step::Remove(d.clone()));
            if let Some(dir) = d.parent() {
                steps.push(Step::RemoveDirIfEmpty(dir.to_path_buf()));
            }
        }
        steps.push(Step::sysctl(&["daemon-reload"], false));
        if old.keeper_enabled {
            let mut args = vec!["enable"];
            if old.keeper_active {
                args.push("--now");
            }
            let name = units::keeper_unit_name();
            args.push(&name);
            steps.push(Step::sysctl(&args, false));
        } else if old.keeper_active {
            steps.push(Step::sysctl(&["start", &units::keeper_unit_name()], false));
        }
    }
    Ok(())
}

/// Look at the system and say what [`run`] would do. Changes nothing (it only asks systemd
/// questions that don't change anything).
///
/// # Errors
/// A message if the old install is in a state the migration won't touch.
pub fn plan(env: &Env) -> Result<Plan, String> {
    build_plan(env, &gather(env)?)
}

/// Carry out one step.
fn execute_step(env: &Env, step: &Step, log: &mut Vec<String>) -> Result<(), String> {
    match step {
        Step::Systemctl { args, best_effort } => {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            match env.sysctl(&refs) {
                Err(e) if !best_effort => return Err(e),
                Err(e) => log.push(format!("ignored: {e}")),
                Ok(_) => {}
            }
        }
        Step::Rename { from, to } => match std::fs::rename(from, to) {
            Ok(()) => {}
            // Moved by an earlier, interrupted run.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && to.exists() => {}
            // An old cache directory that was never created is not a problem.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log.push(format!("skipped {}: not there", from.display()));
            }
            Err(e) => return Err(format!("{} -> {}: {e}", from.display(), to.display())),
        },
        Step::Write { path, content } => {
            config::write_atomic(path, content).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Step::Remove(p) => match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", p.display())),
        },
        Step::RemoveDirIfEmpty(p) => {
            let _ = std::fs::remove_dir(p);
        }
    }
    Ok(())
}

/// Plan and execute the migration. Returns the lines to show the user.
///
/// # Errors
/// A message if the plan couldn't be made (nothing was changed) or a step failed (running it
/// again resumes where it stopped).
pub fn run(env: &Env) -> Result<Vec<String>, String> {
    let plan = plan(env)?;
    if plan.done {
        return Ok(vec![]);
    }
    let mut log = Vec::new();
    for (i, step) in plan.steps.iter().enumerate() {
        execute_step(env, step, &mut log).map_err(|e| {
            format!(
                "step {} of {} ({step}) failed: {e}. The old install may be partly moved; run \
                 `{SLUG} --migrate` again to finish.",
                i + 1,
                plan.steps.len()
            )
        })?;
    }
    let mut out = vec![format!(
        "rclone-shell-ext: migrated {}.",
        plan.source
            .map_or("(nothing to move)".to_string(), |o| format!(
                "the old {} install",
                o.label()
            ))
    )];
    out.extend(plan.notes.iter().map(|n| format!("rclone-shell-ext: {n}")));
    out.extend(log.into_iter().map(|l| format!("rclone-shell-ext: {l}")));
    Ok(out)
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(config::expand(fallback)), PathBuf::from)
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
    let env = Env {
        config_home: config::xdg_config_home(),
        data_home: xdg_dir("XDG_DATA_HOME", "~/.local/share"),
        state_home: xdg_dir("XDG_STATE_HOME", "~/.local/state"),
        systemctl: &runner,
        exe: units::shared_exe().unwrap_or_default(),
    };
    f(&env)
}

/// Every binary calls this first thing. Cheap (one `stat`) once the marker exists, does nothing
/// for the dev build and in tests, and reports problems on stderr without stopping the program.
pub fn run_if_needed() {
    if cfg!(test) || std::env::var_os(SKIP_ENV).is_some() || crate::identity::identity().is_dev() {
        return;
    }
    let marker = xdg_dir("XDG_STATE_HOME", "~/.local/state")
        .join(SLUG)
        .join(MARKER);
    if marker.exists() {
        return;
    }
    // Two binaries starting together (applet and daemon) must not both migrate.
    let lock = xdg_dir("XDG_STATE_HOME", "~/.local/state").join(format!("{SLUG}-migrate.lock"));
    if !acquire(&lock) {
        return;
    }
    match with_process_env(run) {
        Ok(lines) => lines.iter().for_each(|l| eprintln!("{l}")),
        Err(e) => eprintln!("rclone-shell-ext: migrating the old install failed: {e}"),
    }
    let _ = std::fs::remove_file(&lock);
}

/// Take the lock file. One nobody has touched for ten minutes belongs to a process that died.
fn acquire(lock: &Path) -> bool {
    if let Some(dir) = lock.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let create = || {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock)
            .is_ok()
    };
    if create() {
        return true;
    }
    let dead = std::fs::metadata(lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_none_or(|age| age.as_secs() >= 600);
    dead && {
        let _ = std::fs::remove_file(lock);
        create()
    }
}

/// `--migrate [--dry-run]`: print the plan, or carry it out. Returns the text to show.
///
/// # Errors
/// The message to print on stderr (exit code 1).
pub fn command(dry_run: bool) -> Result<String, String> {
    with_process_env(|env| {
        if dry_run {
            plan(env).map(|p| p.to_string())
        } else {
            let plan = plan(env)?;
            if plan.done {
                return Ok(plan.to_string());
            }
            let mut text = String::new();
            for line in run(env)? {
                let _ = writeln!(text, "{line}");
            }
            Ok(text.trim_end().to_string())
        }
    })
}

#[cfg(test)]
mod tests;
