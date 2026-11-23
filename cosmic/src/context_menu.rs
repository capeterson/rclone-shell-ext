// SPDX-License-Identifier: GPL-3.0-only
//! Registers our entries in COSMIC Files' right-click menu: "rclone…" and "Back up to Drive".
//!
//! COSMIC Files has no extension API, only a `context_actions` list in its own config, rendered
//! flat. "rclone…" runs `rclone-shell-ext-cosmic --menu %F` (see `menu.rs`), where everything else
//! lives; "Back up to Drive" (folders only) runs `--backup %F`, which notifies with the result.
//! The user's other entries are left alone.

use cosmic::cosmic_config::{Config as CosmicConfig, ConfigGet, ConfigSet};
use rclone_ui_i18n::fl;
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILES_APP_ID: &str = "com.system76.CosmicFiles";
const FILES_CONFIG_VERSION: u64 = 1;
const KEY: &str = "context_actions";

/// What the "rclone…" entry runs after the executable.
const MENU_ARGS: &str = "--menu %F";
/// What the backup entry runs after the executable.
const BACKUP_ARGS: &str = "--backup %F";

/// Mirrors cosmic-files' `ContextActionSelection`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Selection {
    #[default]
    #[serde(alias = "any")]
    Any,
    #[serde(alias = "files")]
    Files,
    #[serde(alias = "folders")]
    Folders,
}

/// Mirrors cosmic-files' `ContextActionPreset`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    pub confirm: bool,
    pub selection: Selection,
    pub steps: Vec<String>,
}

/// The menu label. The dev copy gets its own so it doesn't replace the real applet's entry.
pub fn entry_name() -> &'static str {
    if crate::DEV {
        "rclone (dev)…"
    } else {
        "rclone…"
    }
}

/// Is `p` one of ours? `program` is this binary's file name.
///
/// The "rclone…" entry goes by its name, including the labels earlier versions used (a
/// capitalized "Rclone…", and the prototypes' "rclone: …" entries). The backup entry's name is
/// translated and would change with the locale, so it goes by what it runs: this program's
/// `--backup`, from whichever directory. The dev copy is a different program (`…-dev`), so the
/// two copies keep their own entries.
fn is_ours(p: &Preset, program: &str) -> bool {
    let (legacy, capitalized) = if crate::DEV {
        ("rclone (dev): ", "Rclone (dev)…")
    } else {
        ("rclone: ", "Rclone…")
    };
    let name = p.name.as_str();
    if name == entry_name() || name == capitalized || name.starts_with(legacy) {
        return true;
    }
    let [step] = p.steps.as_slice() else {
        return false;
    };
    step.strip_suffix(BACKUP_ARGS)
        .and_then(|exe| exe.strip_suffix(' '))
        .is_some_and(|exe| {
            Path::new(&unquote(exe))
                .file_name()
                .is_some_and(|f| f == program)
        })
}

/// Quote the executable for an exec-style command line.
fn quote(exe: &str) -> String {
    if exe.contains([' ', '"', '\\']) {
        format!("\"{}\"", exe.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        exe.to_string()
    }
}

/// The executable as [`quote`] wrote it, unquoted again.
fn unquote(exe: &str) -> String {
    let Some(inner) = exe.strip_prefix('"').and_then(|e| e.strip_suffix('"')) else {
        return exe.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

/// The "rclone…" entry, for any selection.
pub fn entry(exe: &str) -> Preset {
    Preset {
        name: entry_name().to_string(),
        confirm: false,
        selection: Selection::Any,
        steps: vec![format!("{} {MENU_ARGS}", quote(exe))],
    }
}

/// The "Back up to Drive" entry, for folders.
pub fn backup_entry(exe: &str) -> Preset {
    Preset {
        name: fl!("backup-menu-add"),
        confirm: false,
        selection: Selection::Folders,
        steps: vec![format!("{} {BACKUP_ARGS}", quote(exe))],
    }
}

/// Every entry we install, running `exe`.
fn entries(exe: &str) -> Vec<Preset> {
    vec![entry(exe), backup_entry(exe)]
}

/// `existing` with our entries replaced by `ours`; everything else is kept as is.
pub fn merge(existing: Vec<Preset>, ours: Vec<Preset>, program: &str) -> Vec<Preset> {
    let mut out: Vec<Preset> = existing
        .into_iter()
        .filter(|p| !is_ours(p, program))
        .collect();
    out.extend(ours);
    out
}

/// The list to write, or `None` when `existing` already has exactly our entries.
fn apply(existing: &[Preset], ours: Vec<Preset>, program: &str) -> Option<Vec<Preset>> {
    let merged = merge(existing.to_vec(), ours, program);
    (merged != existing).then_some(merged)
}

fn write_merged_to(ctx: &CosmicConfig, ours: Vec<Preset>, program: &str) {
    let existing: Vec<Preset> = ctx.get(KEY).unwrap_or_default();
    if let Some(merged) = apply(&existing, ours, program) {
        let _ = ctx.set(KEY, merged);
    }
}

/// Write `ours(exe)` in place of this binary's entries.
fn write_merged(ours: impl FnOnce(&str) -> Vec<Preset>) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let program = exe
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Ok(ctx) = CosmicConfig::new(FILES_APP_ID, FILES_CONFIG_VERSION) {
        write_merged_to(&ctx, ours(&exe.to_string_lossy()), &program);
    }
}

/// Make sure the entries exist and point at this binary. Cheap when nothing changed.
pub fn install() {
    write_merged(entries);
}

/// Remove our entries (`--menu-remove`).
pub fn remove() {
    write_merged(|_| vec![]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The binary the tests install, and its file name.
    const EXE: &str = "/x/rcs";
    const PROGRAM: &str = "rcs";

    fn named(n: &str) -> Preset {
        Preset {
            name: n.into(),
            ..Preset::default()
        }
    }

    fn ours() -> Vec<Preset> {
        entries(EXE)
    }

    #[test]
    fn entries_run_the_menu_and_the_backup_with_the_selection() {
        assert_eq!(entry(EXE).steps, ["/x/rcs --menu %F"]);
        assert_eq!(entry("/my bin/rcs").steps, ["\"/my bin/rcs\" --menu %F"]);
        let backup = backup_entry(EXE);
        assert_eq!(backup.steps, ["/x/rcs --backup %F"]);
        assert_eq!(backup.selection, Selection::Folders);
        assert_eq!(backup.name, fl!("backup-menu-add"));
        assert_eq!(
            backup_entry("/my bin/rcs").steps,
            ["\"/my bin/rcs\" --backup %F"]
        );
    }

    #[test]
    fn quoting_round_trips() {
        for exe in ["/x/rcs", "/my bin/rcs", "/a \"b\"/c\\d"] {
            assert_eq!(unquote(&quote(exe)), exe);
        }
    }

    #[test]
    fn merge_replaces_ours_and_keeps_the_users() {
        let theirs = named("Compress");
        let out = merge(
            vec![named(entry_name()), theirs.clone(), backup_entry(EXE)],
            ours(),
            PROGRAM,
        );
        let mut want = vec![theirs];
        want.extend(ours());
        assert_eq!(out, want);
    }

    #[test]
    fn the_backup_entry_is_recognised_by_what_it_runs_not_its_label() {
        // Written under another locale, or by a build at another path.
        let other_locale = Preset {
            name: "Auf Drive sichern".into(),
            ..backup_entry("/old dir/rcs")
        };
        assert_eq!(merge(vec![other_locale], ours(), PROGRAM), ours());
        // Another program's backup entry (the dev copy, or the real one seen from it) stays.
        let other = backup_entry("/x/rcs-dev");
        assert_eq!(
            merge(vec![other.clone()], vec![], PROGRAM),
            vec![other.clone()]
        );
        // So does the user's own entry that happens to share the label.
        let theirs = Preset {
            steps: vec!["my-backup %F".into()],
            ..backup_entry(EXE)
        };
        assert_eq!(merge(vec![theirs.clone()], vec![], PROGRAM), vec![theirs]);
    }

    #[test]
    fn merge_drops_prototype_entries() {
        let legacy = if crate::DEV {
            "rclone (dev): Synced"
        } else {
            "rclone: Synced"
        };
        assert_eq!(merge(vec![named(legacy)], vec![], PROGRAM), vec![]);
    }

    #[test]
    fn the_old_capitalized_entry_is_replaced_not_duplicated() {
        let old = if crate::DEV {
            "Rclone (dev)…"
        } else {
            "Rclone…"
        };
        assert_eq!(merge(vec![named(old)], ours(), PROGRAM), ours());
        assert!(entry(EXE).name.starts_with("rclone"));
    }

    #[test]
    fn the_old_one_entry_config_gains_the_backup_entry() {
        assert_eq!(
            apply(&[named("Compress"), entry(EXE)], ours(), PROGRAM),
            Some(vec![named("Compress"), entry(EXE), backup_entry(EXE)])
        );
    }

    #[test]
    fn merge_is_idempotent() {
        let once = merge(vec![], ours(), PROGRAM);
        assert_eq!(once.len(), 2);
        assert_eq!(merge(once.clone(), ours(), PROGRAM), once);
    }

    #[test]
    fn apply_does_nothing_when_already_installed() {
        let mut existing = vec![named("Compress")];
        existing.extend(ours());
        assert_eq!(apply(&existing, ours(), PROGRAM), None);
    }

    #[test]
    fn apply_updates_a_stale_path_and_installs_when_missing() {
        let stale = entries("/old/rcs");
        assert_eq!(
            apply(&stale, entries("/new/rcs"), PROGRAM),
            Some(entries("/new/rcs"))
        );
        assert_eq!(apply(&[], ours(), PROGRAM), Some(ours()));
    }

    #[test]
    fn apply_removes_only_ours() {
        let mut existing = vec![named("Compress")];
        existing.extend(ours());
        assert_eq!(
            apply(&existing, vec![], PROGRAM),
            Some(vec![named("Compress")])
        );
        assert_eq!(apply(&[named("Compress")], vec![], PROGRAM), None);
    }

    fn temp_config(name: &str) -> (std::path::PathBuf, CosmicConfig) {
        let dir = std::env::temp_dir().join(format!("rcs-ctxcfg-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let ctx = CosmicConfig::with_custom_path(FILES_APP_ID, FILES_CONFIG_VERSION, dir.clone())
            .unwrap();
        (dir, ctx)
    }

    #[test]
    fn writes_the_entries_to_disk_in_files_config_format() {
        let (dir, ctx) = temp_config("write");
        write_merged_to(&ctx, ours(), PROGRAM);
        let on_disk: Vec<Preset> = ctx.get(KEY).unwrap();
        assert_eq!(on_disk, ours());
        // COSMIC Files reads this file with its own struct, so the field names and the enum
        // spelling must be exactly these.
        let file = dir.join("cosmic").join(FILES_APP_ID).join("v1").join(KEY);
        let text = std::fs::read_to_string(file).unwrap();
        for want in [
            "name:",
            "confirm:",
            "selection: Any",
            "selection: Folders",
            "steps:",
            "rclone…",
            "--backup %F",
        ] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disk_round_trip_keeps_the_users_entries_and_drops_prototypes() {
        let (dir, ctx) = temp_config("roundtrip");
        let theirs = Preset {
            name: "Compress".into(),
            confirm: true,
            selection: Selection::Folders,
            steps: vec!["tar czf %f.tgz %f".into()],
        };
        let legacy = if crate::DEV {
            "rclone (dev): Synced"
        } else {
            "rclone: Synced"
        };
        ctx.set(KEY, vec![named(legacy), theirs.clone()]).unwrap();

        write_merged_to(&ctx, ours(), PROGRAM);
        let after: Vec<Preset> = ctx.get(KEY).unwrap();
        let mut want = vec![theirs.clone()];
        want.extend(ours());
        assert_eq!(after, want);

        // Installing again changes nothing; removing takes only ours.
        write_merged_to(&ctx, ours(), PROGRAM);
        assert_eq!(ctx.get::<Vec<Preset>>(KEY).unwrap(), after);
        write_merged_to(&ctx, vec![], PROGRAM);
        assert_eq!(ctx.get::<Vec<Preset>>(KEY).unwrap(), vec![theirs]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
