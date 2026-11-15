// SPDX-License-Identifier: GPL-3.0-only
//! The systemd user units behind offline folders, written and removed by the app:
//!
//! - `<slug>-offline.service`: the keeper (`--offline-keeper`), started with the main mount;
//! - `<slug>-pin@<id>.service`: one in-place mount per offline folder (`--pin-mount <id>`);
//! - a drop-in for the main mount's unit that adds `--allow-other` (so folders can be mounted
//!   inside it), `--default-permissions` and a `077` umask (so other users still can't get in).
//!
//! All are bound to the main mount's unit, so they start and stop with it. Every unit runs the
//! shared `rclone-shell-ext` binary (see [`shared_exe`]), whichever front end installed it.

use crate::config::Config;
use crate::pin::app_dir_name;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn unit_dir() -> PathBuf {
    unit_dir_in(&crate::config::xdg_config_home())
}

/// The systemd user unit directory under `config_home`.
pub fn unit_dir_in(config_home: &Path) -> PathBuf {
    config_home.join("systemd/user")
}

/// The path of the shared binary the units run: the sibling of the running executable named
/// after the slug (`rclone-shell-ext`, or `-dev`), else the first match on `PATH`.
pub fn shared_exe() -> Result<PathBuf, String> {
    let cur = std::env::current_exe().map_err(|e| e.to_string())?;
    resolve_shared_exe(
        &cur,
        crate::identity::identity().slug,
        std::env::var_os("PATH").as_deref(),
    )
}

/// [`shared_exe`] with its inputs spelled out: the running executable, the binary's name and
/// the value of `PATH`.
pub fn resolve_shared_exe(
    cur_exe: &Path,
    name: &str,
    path_var: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, String> {
    let is_file = |p: &Path| p.is_file();
    if let Some(sibling) = cur_exe.parent().map(|d| d.join(name)) {
        if is_file(&sibling) {
            return Ok(sibling);
        }
    }
    if let Some(found) = path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|d| d.join(name))
        .find(|p| is_file(p))
    {
        return Ok(found);
    }
    Err(format!(
        "can't find the `{name}` program: it isn't next to {} or on PATH",
        cur_exe.display()
    ))
}

pub fn keeper_unit_name() -> String {
    format!("{}-offline.service", app_dir_name())
}

/// The pin unit template's file name (`<slug>-pin@.service`).
pub fn pin_template_name() -> String {
    format!("{}-pin@.service", app_dir_name())
}

pub fn pin_unit_name(id: &str) -> String {
    format!("{}-pin@{id}.service", app_dir_name())
}

pub fn dropin_path(service: &str) -> PathBuf {
    dropin_path_in(&unit_dir(), service)
}

/// The drop-in's path inside the unit directory `unit_dir`.
pub fn dropin_path_in(unit_dir: &Path, service: &str) -> PathBuf {
    unit_dir
        .join(format!("{service}.d"))
        .join(format!("50-{}.conf", app_dir_name()))
}

/// Quote an executable path for an `Exec*=` line if it needs it.
fn exec(exe: &str) -> String {
    if exe.contains([' ', '"', '\\']) {
        format!("\"{}\"", exe.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        exe.to_string()
    }
}

pub fn keeper_unit(exe: &str, service: &str) -> String {
    format!(
        "# Written by {app}; remove it from the app's settings.\n\
         [Unit]\n\
         Description=Keep rclone offline folders in sync ({app})\n\
         BindsTo={service}\n\
         After={service}\n\
         \n\
         [Service]\n\
         ExecStart={exe} --offline-keeper\n\
         Restart=on-failure\n\
         RestartSec=30\n\
         \n\
         [Install]\n\
         WantedBy={service}\n",
        app = app_dir_name(),
        exe = exec(exe),
    )
}

pub fn pin_unit(exe: &str, service: &str) -> String {
    format!(
        "# Written by {app}; remove it from the app's settings.\n\
         [Unit]\n\
         Description=rclone offline folder %i ({app})\n\
         BindsTo={service}\n\
         After={service}\n\
         \n\
         [Service]\n\
         Type=notify\n\
         ExecStart={exe} --pin-mount %i\n\
         ExecStopPost=-{exe} --pin-unmount %i\n\
         Restart=on-failure\n\
         RestartSec=10\n",
        app = app_dir_name(),
        exe = exec(exe),
    )
}

pub fn dropin() -> String {
    format!(
        "# Written by {app} so offline folders can be mounted inside this mount.\n\
         # allow_other lets the mount helper in; default permissions with a 077 umask keep\n\
         # every other user out. Turn offline folders off in the app's settings to remove it.\n\
         [Service]\n\
         Environment=RCLONE_ALLOW_OTHER=true\n\
         Environment=RCLONE_DEFAULT_PERMISSIONS=true\n\
         Environment=RCLONE_UMASK=077\n",
        app = app_dir_name()
    )
}

/// Write `content` to `path` unless it's already there. Returns whether anything changed.
pub fn write_if_changed(path: &Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|c| c == content) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

fn remove_if_present(path: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn systemctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("systemctl {}: {err}", args.join(" ")))
    }
}

pub fn is_active(unit: &str) -> bool {
    crate::rc::service_active(unit)
}

/// Are the unit files and the drop-in in place with today's contents?
pub fn installed(cfg: &Config) -> bool {
    let Ok(exe) = shared_exe() else {
        return false;
    };
    let exe = exe.to_string_lossy();
    let same = |p: PathBuf, want: String| std::fs::read_to_string(p).is_ok_and(|c| c == want);
    same(
        unit_dir().join(keeper_unit_name()),
        keeper_unit(&exe, &cfg.service),
    ) && same(
        unit_dir().join(pin_template_name()),
        pin_unit(&exe, &cfg.service),
    ) && same(dropin_path(&cfg.service), dropin())
}

/// Write the units and the drop-in, and enable and start the keeper.
pub fn install(cfg: &Config) -> Result<(), String> {
    let exe = shared_exe()?;
    let exe = exe.to_string_lossy();
    let io = |e: std::io::Error| e.to_string();
    let mut changed = write_if_changed(
        &unit_dir().join(keeper_unit_name()),
        &keeper_unit(&exe, &cfg.service),
    )
    .map_err(io)?;
    changed |= write_if_changed(
        &unit_dir().join(pin_template_name()),
        &pin_unit(&exe, &cfg.service),
    )
    .map_err(io)?;
    changed |= write_if_changed(&dropin_path(&cfg.service), &dropin()).map_err(io)?;
    if changed {
        systemctl(&["daemon-reload"])?;
    }
    systemctl(&["enable", "--now", &keeper_unit_name()])
}

/// Stop and remove everything [`install`] set up. The caller makes sure no folders are pinned.
pub fn remove(cfg: &Config) -> Result<(), String> {
    let pins = format!("{}-pin@*.service", app_dir_name());
    systemctl(&["stop", &pins])?;
    // Disabling fails harmlessly if it was never installed.
    let _ = systemctl(&["disable", "--now", &keeper_unit_name()]);
    let io = |e: std::io::Error| e.to_string();
    remove_if_present(&unit_dir().join(keeper_unit_name())).map_err(io)?;
    remove_if_present(&unit_dir().join(pin_template_name())).map_err(io)?;
    remove_if_present(&dropin_path(&cfg.service)).map_err(io)?;
    // Leave an empty `<service>.d` alone if the user has other drop-ins; remove it otherwise.
    if let Some(dir) = dropin_path(&cfg.service).parent() {
        let _ = std::fs::remove_dir(dir);
    }
    systemctl(&["daemon-reload"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_bound_to_the_mount_service() {
        for text in [
            keeper_unit("/x/rcs", "rclone-gdrive.service"),
            pin_unit("/x/rcs", "rclone-gdrive.service"),
        ] {
            assert!(text.contains("BindsTo=rclone-gdrive.service\n"), "{text}");
            assert!(text.contains("After=rclone-gdrive.service\n"), "{text}");
        }
        assert!(keeper_unit("/x/rcs", "m.service").contains("WantedBy=m.service\n"));
    }

    #[test]
    fn units_run_this_binary() {
        assert!(keeper_unit("/x/rcs", "m").contains("ExecStart=/x/rcs --offline-keeper\n"));
        let pin = pin_unit("/x/rcs", "m");
        assert!(pin.contains("Type=notify\n"));
        assert!(pin.contains("ExecStart=/x/rcs --pin-mount %i\n"));
        assert!(pin.contains("ExecStopPost=-/x/rcs --pin-unmount %i\n"));
        assert!(
            keeper_unit("/my bin/rcs", "m").contains("ExecStart=\"/my bin/rcs\" --offline-keeper")
        );
    }

    #[test]
    fn dropin_lets_mounts_nest_but_keeps_other_users_out() {
        let d = dropin();
        assert!(d.contains("[Service]\n"));
        for want in [
            "RCLONE_ALLOW_OTHER=true",
            "RCLONE_DEFAULT_PERMISSIONS=true",
            "RCLONE_UMASK=077",
        ] {
            assert!(d.contains(&format!("Environment={want}\n")), "{d}");
        }
    }

    #[test]
    fn names_follow_the_build() {
        assert!(keeper_unit_name().ends_with("-offline.service"));
        assert!(pin_unit_name("abc").ends_with("-pin@abc.service"));
        assert!(
            dropin_path("m.service").ends_with(format!("m.service.d/50-{}.conf", app_dir_name()))
        );
    }

    #[test]
    fn shared_exe_prefers_the_sibling_then_path() {
        let dir = std::env::temp_dir().join(format!("rcs-exe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (bin, other) = (dir.join("bin"), dir.join("other"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let cur = bin.join("front-end");
        let path = std::env::join_paths([&other]).unwrap();
        // Nothing anywhere: an error naming the program.
        let err = resolve_shared_exe(&cur, "rse", Some(&path)).unwrap_err();
        assert!(err.contains("`rse`"), "{err}");
        // On PATH only.
        std::fs::write(other.join("rse"), "").unwrap();
        assert_eq!(
            resolve_shared_exe(&cur, "rse", Some(&path)).unwrap(),
            other.join("rse")
        );
        // A sibling wins over PATH.
        std::fs::write(bin.join("rse"), "").unwrap();
        assert_eq!(
            resolve_shared_exe(&cur, "rse", Some(&path)).unwrap(),
            bin.join("rse")
        );
        assert!(resolve_shared_exe(&cur, "nope", None).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn write_if_changed_only_writes_differences() {
        let dir = std::env::temp_dir().join(format!("rcs-units-{}", std::process::id()));
        let f = dir.join("sub/a.service");
        assert!(write_if_changed(&f, "x").unwrap());
        assert!(!write_if_changed(&f, "x").unwrap());
        assert!(write_if_changed(&f, "y").unwrap());
        assert!(remove_if_present(&f).unwrap());
        assert!(!remove_if_present(&f).unwrap());
        std::fs::remove_dir_all(dir).ok();
    }
}
