//! Renaming the old Plasma plasmoid id in plasmashell's applet list.
//!
//! plasmashell keeps its panel layout in memory and writes it back over
//! `plasma-org.kde.plasma.desktop-appletsrc` when it likes, so this must only ever touch the file
//! while plasmashell is stopped. That is why it is its own command (`--plasma-panel-rename`, run
//! by the install recipe between stopping and starting the shell) and not a migration step.

use crate::config;
use crate::identity::APP_ID;
use std::path::{Path, PathBuf};

/// The old Plasma plasmoid id.
pub const OLD_PLASMOID_ID: &str = "com.ca-peterson.rcloneplasma";
/// The file in `$XDG_CONFIG_HOME` where plasmashell lists its applets.
pub const APPLETSRC: &str = "plasma-org.kde.plasma.desktop-appletsrc";
/// Applet or containment plugins whose child applets are system tray entries.
const TRAY_PLUGINS: [&str; 2] = [
    "org.kde.plasma.systemtray",
    "org.kde.plasma.private.systemtray",
];

struct Section {
    /// The `[a][b]` header split into its groups (empty for text before the first header).
    path: Vec<String>,
    /// The header line and the lines below it, line endings and trailing blank lines included.
    lines: Vec<String>,
}

fn parse(text: &str) -> Vec<Section> {
    let mut out = vec![Section {
        path: Vec::new(),
        lines: Vec::new(),
    }];
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        if body.starts_with('[') && body.ends_with(']') {
            let path = body[1..body.len() - 1]
                .split("][")
                .map(str::to_string)
                .collect();
            out.push(Section {
                path,
                lines: Vec::new(),
            });
        }
        out.last_mut()
            .expect("never empty")
            .lines
            .push(line.to_string());
    }
    out
}

fn plugin_of(section: &Section) -> Option<&str> {
    section
        .lines
        .iter()
        .find_map(|l| l.trim_end_matches(['\n', '\r']).strip_prefix("plugin="))
}

/// `key=a,b,<old>` with the old id replaced by the new one and repeats dropped (first wins), if
/// the line names the old id as a whole entry; line endings are kept.
fn rename_in_line(line: &str) -> Option<String> {
    let body = line.trim_end_matches(['\n', '\r']);
    let ending = &line[body.len()..];
    let (key, value) = body.split_once('=')?;
    if !value.split(',').any(|v| v == OLD_PLASMOID_ID) {
        return None;
    }
    let mut seen: Vec<&str> = Vec::new();
    for v in value.split(',') {
        let v = if v == OLD_PLASMOID_ID { APP_ID } else { v };
        if !seen.contains(&v) {
            seen.push(v);
        }
    }
    Some(format!("{key}={}{ending}", seen.join(",")))
}

/// The applet groups a rewrite drops: old-id system tray entries that already have a new-id
/// sibling in the same tray. A tray is any group (an applet or a containment) whose plugin is a
/// system tray plugin; its entries are the `[..][Applets][N]` groups below it. Plasma versions
/// differ in whether the tray is nested in the panel or a containment of its own.
fn duplicate_tray_entries(sections: &[Section]) -> Vec<Vec<String>> {
    let mut drop = Vec::new();
    for tray in sections
        .iter()
        .filter(|s| plugin_of(s).is_some_and(|p| TRAY_PLUGINS.contains(&p)))
    {
        let entry_len = tray.path.len() + 2;
        let entries: Vec<&Section> = sections
            .iter()
            .filter(|s| {
                s.path.len() == entry_len
                    && s.path.starts_with(&tray.path)
                    && s.path[tray.path.len()] == "Applets"
            })
            .collect();
        if entries.iter().any(|e| plugin_of(e) == Some(APP_ID)) {
            drop.extend(
                entries
                    .iter()
                    .filter(|e| plugin_of(e) == Some(OLD_PLASMOID_ID))
                    .map(|e| e.path.clone()),
            );
        }
    }
    drop
}

/// The applet list with the old plasmoid id gone: `plugin=` and comma lists (`extraItems=`,
/// `knownItems=`) get the new id, whole entries only so near misses stay; a system tray entry
/// that already has a new-id twin (plasmashell adds one when the new plasmoid is installed) is
/// removed with its sub-groups instead. Idempotent. `None` when there is nothing to change.
#[must_use]
pub fn rewrite(text: &str) -> Option<String> {
    let sections = parse(text);
    let drop = duplicate_tray_entries(&sections);
    let out: String = sections
        .iter()
        .filter(|s| !drop.iter().any(|d| s.path.starts_with(d)))
        .flat_map(|s| s.lines.iter())
        .map(|l| rename_in_line(l).unwrap_or_else(|| l.clone()))
        .collect();
    (out != text).then_some(out)
}

/// Whether the file names the old or the new plasmoid id at all (as a whole list entry).
#[must_use]
pub fn mentions_plasmoid(text: &str) -> bool {
    text.lines().any(|l| {
        l.split_once('=')
            .is_some_and(|(_, v)| v.split(',').any(|v| v == OLD_PLASMOID_ID || v == APP_ID))
    })
}

/// Whether a process called `plasmashell` owned by the current user is running, by `/proc`.
#[must_use]
pub fn plasmashell_running() -> bool {
    plasmashell_running_in(Path::new("/proc"), current_uid())
}

fn current_uid() -> u32 {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata("/proc/self").map_or(u32::MAX, |m| m.uid())
}

/// [`plasmashell_running`] over a `/proc`-shaped directory and a user id.
#[must_use]
pub fn plasmashell_running_in(proc: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(dir) = std::fs::read_dir(proc) else {
        return false;
    };
    dir.flatten().any(|e| {
        let p = e.path();
        e.file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
            && std::fs::read_to_string(p.join("comm")).is_ok_and(|c| c.trim() == "plasmashell")
            && std::fs::metadata(&p).is_ok_and(|m| m.uid() == uid)
    })
}

/// What [`apply`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The file is missing or already uses only the new id.
    Nothing(PathBuf),
    /// The file was (or, with `dry_run`, would be) rewritten.
    Changed(PathBuf),
}

/// Rewrite `path` (see [`rewrite`]) atomically, keeping its permissions. Refuses while
/// `running` says plasmashell is up, since it would write its own copy back; a dry run changes
/// nothing and does not need the shell stopped.
///
/// # Errors
/// The shell is running (`Err(None)`), or the file could not be read or written.
pub fn apply(
    path: &Path,
    dry_run: bool,
    running: &dyn Fn() -> bool,
) -> Result<Outcome, Option<String>> {
    use std::os::unix::fs::PermissionsExt as _;
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Outcome::Nothing(path.to_path_buf()));
    };
    let Some(new) = rewrite(&text) else {
        return Ok(Outcome::Nothing(path.to_path_buf()));
    };
    if dry_run {
        return Ok(Outcome::Changed(path.to_path_buf()));
    }
    if running() {
        return Err(None);
    }
    let mode = std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o7777);
    config::write_atomic_mode(path, &new, mode)
        .map_err(|e| Some(format!("{}: {e}", path.display())))?;
    Ok(Outcome::Changed(path.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "com.ca-peterson.rcloneplasma";
    const NEW: &str = "com.ca-peterson.rclone-shell-ext";

    /// Pre-migration file: the plasmoid on the panel and in the tray (nested layout).
    fn pre() -> String {
        format!(
            "[Containments][2][Applets][32]\nimmutability=1\nplugin={OLD}\n\n\
[Containments][2][Applets][32][Configuration]\npopupHeight=400\n\n\
[Containments][2][Applets][7]\nplugin=org.kde.plasma.systemtray\n\n\
[Containments][2][Applets][7][Applets][28]\nplugin=org.kde.plasma.mediacontroller\n\n\
[Containments][2][Applets][7][Applets][35]\nimmutability=1\nplugin={OLD}\n\n\
[Containments][2][Applets][7][Applets][35][Configuration]\nx=1\n\n\
[Containments][2][Applets][7][General]\nextraItems=org.kde.kdeconnect,{OLD}\nknownItems=org.kde.kdeconnect,{OLD},org.kde.plasma.volume\nnote={OLD}X,x{OLD}\n"
        )
    }

    /// What plasmashell left after the buggy install: a new tray entry beside the old ones.
    fn post_buggy() -> String {
        pre()
            .replace(
                "[Containments][2][Applets][7][General]",
                &format!("[Containments][2][Applets][7][Applets][36]\nimmutability=1\nplugin={NEW}\n\n[Containments][2][Applets][7][General]"),
            )
            .replace(
                &format!("extraItems=org.kde.kdeconnect,{OLD}"),
                &format!("extraItems=org.kde.kdeconnect,{OLD},{NEW}"),
            )
            .replace(
                &format!("knownItems=org.kde.kdeconnect,{OLD},org.kde.plasma.volume"),
                &format!("knownItems=org.kde.kdeconnect,{OLD},org.kde.plasma.volume,{NEW}"),
            )
    }

    #[test]
    fn pre_migration_file_is_renamed_by_whole_entries() {
        let out = rewrite(&pre()).unwrap();
        // No new-id tray entry: the old one is renamed, not dropped.
        assert!(out.contains(&format!(
            "[Containments][2][Applets][7][Applets][35]\nimmutability=1\nplugin={NEW}\n"
        )));
        assert!(out.contains(&format!(
            "[Containments][2][Applets][32]\nimmutability=1\nplugin={NEW}\n"
        )));
        assert!(out.contains(&format!("extraItems=org.kde.kdeconnect,{NEW}\n")));
        assert!(out.contains(&format!(
            "knownItems=org.kde.kdeconnect,{NEW},org.kde.plasma.volume\n"
        )));
        assert!(
            out.contains(&format!("note={OLD}X,x{OLD}\n")),
            "near misses stay"
        );
        assert_eq!(out.replace(NEW, OLD), pre(), "nothing else changed");
        assert_eq!(rewrite(&out), None, "idempotent");
    }

    #[test]
    fn post_buggy_install_drops_old_tray_entry_and_dedups() {
        let out = rewrite(&post_buggy()).unwrap();
        assert!(!out.contains("[Applets][35]"), "{out}");
        assert!(out.contains(&format!(
            "[Containments][2][Applets][7][Applets][36]\nimmutability=1\nplugin={NEW}\n"
        )));
        assert!(out.contains(&format!(
            "[Containments][2][Applets][32]\nimmutability=1\nplugin={NEW}\n"
        )));
        assert!(out.contains("[Containments][2][Applets][32][Configuration]"));
        assert!(out.contains("plugin=org.kde.plasma.mediacontroller"));
        assert!(out.contains(&format!("extraItems=org.kde.kdeconnect,{NEW}\n")));
        assert!(out.contains(&format!(
            "knownItems=org.kde.kdeconnect,{NEW},org.kde.plasma.volume\n"
        )));
        assert!(!out.contains(&format!("plugin={OLD}")));
        assert_eq!(rewrite(&out), None);
    }

    #[test]
    fn already_fixed_file_is_left_alone() {
        let fixed = rewrite(&post_buggy()).unwrap();
        assert_eq!(rewrite(&fixed), None);
        assert_eq!(rewrite(""), None);
        assert_eq!(rewrite("[Containments][1]\nplugin=org.kde.panel\n"), None);
    }

    #[test]
    fn separate_systemtray_containment_layout() {
        let text = format!(
            "[Containments][2][Applets][32]\nplugin={OLD}\n\n\
[Containments][9]\nplugin=org.kde.plasma.private.systemtray\n\n\
[Containments][9][Applets][1]\nplugin={OLD}\n\n\
[Containments][9][Applets][1][Configuration]\na=b\n\n\
[Containments][9][Applets][2]\nplugin={NEW}\n\n\
[Containments][9][General]\nextraItems={OLD},{NEW}\n"
        );
        let out = rewrite(&text).unwrap();
        assert!(!out.contains("[Containments][9][Applets][1]"), "{out}");
        assert!(out.contains(&format!("[Containments][9][Applets][2]\nplugin={NEW}\n")));
        assert!(out.contains(&format!("[Containments][2][Applets][32]\nplugin={NEW}\n")));
        assert!(out.contains(&format!("extraItems={NEW}\n")));
        assert_eq!(rewrite(&out), None);
    }

    #[test]
    fn crlf_endings_survive() {
        let out = rewrite(&format!("[A]\r\nplugin={OLD}\r\n")).unwrap();
        assert_eq!(out, format!("[A]\r\nplugin={NEW}\r\n"));
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rse-panel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn apply_refuses_while_plasmashell_runs_and_keeps_mode_otherwise() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = scratch("apply");
        let f = d.join(APPLETSRC);
        std::fs::write(&f, pre()).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(apply(&f, false, &|| true), Err(None));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), pre(), "untouched");
        assert_eq!(apply(&f, true, &|| true), Ok(Outcome::Changed(f.clone())));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), pre(), "dry run");
        assert_eq!(apply(&f, false, &|| false), Ok(Outcome::Changed(f.clone())));
        assert!(!std::fs::read_to_string(&f)
            .unwrap()
            .contains(&format!("plugin={OLD}")));
        assert_eq!(
            std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(apply(&f, false, &|| true), Ok(Outcome::Nothing(f.clone())));
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no temp files");
        assert!(matches!(
            apply(&d.join("missing"), false, &|| true),
            Ok(Outcome::Nothing(_))
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn plasmashell_is_found_in_a_proc_directory_by_comm_and_owner() {
        use std::os::unix::fs::MetadataExt as _;
        let d = scratch("proc");
        std::fs::create_dir_all(d.join("123")).unwrap();
        std::fs::write(d.join("123/comm"), "plasmashell\n").unwrap();
        std::fs::create_dir_all(d.join("124")).unwrap();
        std::fs::write(d.join("124/comm"), "bash\n").unwrap();
        let uid = std::fs::metadata(&d).unwrap().uid();
        assert!(plasmashell_running_in(&d, uid));
        assert!(!plasmashell_running_in(&d, uid + 1), "someone else's");
        std::fs::write(d.join("123/comm"), "other\n").unwrap();
        assert!(!plasmashell_running_in(&d, uid));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mentions_whole_entries_of_either_id() {
        assert!(mentions_plasmoid(&pre()));
        assert!(mentions_plasmoid(&format!("extraItems=a,{NEW}\n")));
        assert!(!mentions_plasmoid(&format!("note={OLD}X\n")));
    }
}
