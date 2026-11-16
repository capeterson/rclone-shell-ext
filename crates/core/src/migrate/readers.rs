// SPDX-License-Identifier: GPL-3.0-only
//! Readers for the two old config formats: the COSMIC applet's cosmic-config directory (one RON
//! value per file) and the Plasma front end's `KConfig` INI file.

use crate::config::Config;
use rclone_ui_i18n::fl;
use std::path::Path;
use std::time::SystemTime;

const GROUP: &str = "General";

/// The old COSMIC settings in `dir` (`.../com.ca-peterson.CosmicRcloneUi/v1`), and when they
/// were last written. A missing file means that field's default.
///
/// # Errors
/// A message naming the file if one holds something that isn't the expected RON value.
pub fn read_cosmic(dir: &Path) -> Result<Option<(Config, Option<SystemTime>)>, String> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut newest: Option<SystemTime> = None;
    let mut cfg = Config::default();
    macro_rules! field {
        ($name:ident) => {{
            let file = dir.join(stringify!($name));
            match std::fs::read_to_string(&file) {
                Ok(text) => {
                    cfg.$name = ron::from_str(&text).map_err(|e| {
                        fl!(
                            "migrate-error-setting",
                            path = file.display().to_string(),
                            error = e.to_string()
                        )
                    })?;
                    let mtime = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
                    newest = newest.max(mtime);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", file.display())),
            }
        }};
    }
    field!(rc_addr);
    field!(env_file);
    field!(service);
    field!(mount_path);
    field!(cache_dir);
    field!(context_menu);
    field!(offline_enabled);
    field!(pins);
    field!(sync_interval_secs);
    Ok(Some((cfg, newest)))
}

/// `KConfig` value unescaping.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('s') => out.push(' '),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

/// Split a list value on unescaped commas (items are left escaped).
fn split_list(s: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let (mut start, mut esc) = (0, false);
    for (i, c) in s.char_indices() {
        match c {
            _ if esc => esc = false,
            '\\' => esc = true,
            ',' => {
                items.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    items.push(&s[start..]);
    items
}

/// The `key=value` pairs of the `[General]` group, values still escaped.
fn general_entries(text: &str) -> Vec<(&str, &str)> {
    let mut in_general = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(g) = line.strip_prefix('[') {
            in_general = g.trim_end().strip_suffix(']') == Some(GROUP);
        } else if in_general && !line.starts_with('#') {
            if let Some((k, v)) = line.split_once('=') {
                out.push((k.trim(), v));
            }
        }
    }
    out
}

/// The old Plasma settings in `KConfig` INI `text` (group `[General]`, one key per field), with the
/// default for anything missing or unreadable.
pub fn parse_kconfig(text: &str) -> Config {
    let mut cfg = Config::default();
    for (k, raw) in general_entries(text) {
        match k {
            "rc_addr" => cfg.rc_addr = unescape(raw),
            "env_file" => cfg.env_file = unescape(raw),
            "service" => cfg.service = unescape(raw),
            "mount_path" => cfg.mount_path = unescape(raw),
            "cache_dir" => cfg.cache_dir = unescape(raw),
            "context_menu" => {
                if let Ok(b) = raw.trim().parse() {
                    cfg.context_menu = b;
                }
            }
            "offline_enabled" => {
                if let Ok(b) = raw.trim().parse() {
                    cfg.offline_enabled = b;
                }
            }
            "pins" => {
                cfg.pins = split_list(raw)
                    .into_iter()
                    .map(unescape)
                    .filter(|p| !p.is_empty())
                    .collect();
            }
            "sync_interval_secs" => {
                if let Ok(n) = raw.trim().parse() {
                    cfg.sync_interval_secs = n;
                }
            }
            _ => {}
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kconfig_defaults_when_invalid() {
        let c =
            parse_kconfig("[General]\ncontext_menu=maybe\nsync_interval_secs=-4\nrc_addr=x:1\n");
        assert_eq!(c.rc_addr, "x:1");
        assert!(c.context_menu);
        assert_eq!(c.sync_interval_secs, 300);
        // Keys in other groups are ignored.
        assert_eq!(parse_kconfig("[Other]\nrc_addr=bad\n"), Config::default());
    }

    #[test]
    fn kconfig_lists_and_escapes() {
        assert_eq!(unescape("\\sx\\s"), " x ");
        assert_eq!(split_list("a\\,b,c\\\\,d"), vec!["a\\,b", "c\\\\", "d"]);
        assert_eq!(
            parse_kconfig("[General]\npins=\n").pins,
            Vec::<String>::new()
        );
        let c = parse_kconfig("[General]\npins=Projects,A\\,b\\\\c,x/y z\noffline_enabled=true\n");
        assert_eq!(c.pins, ["Projects", "A,b\\c", "x/y z"]);
        assert!(c.offline_enabled);
    }

    #[test]
    fn cosmic_values_are_read_and_missing_ones_default() {
        let dir = std::env::temp_dir().join(format!("rcs-ronread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rc_addr"), "\"10.0.0.1:9\"").unwrap();
        std::fs::write(dir.join("offline_enabled"), "true").unwrap();
        std::fs::write(dir.join("pins"), "[\n    \"A/b\",\n    \"c\",\n]").unwrap();
        std::fs::write(dir.join("sync_interval_secs"), "60").unwrap();
        let (cfg, mtime) = read_cosmic(&dir).unwrap().unwrap();
        assert_eq!(cfg.rc_addr, "10.0.0.1:9");
        assert!(cfg.offline_enabled);
        assert_eq!(cfg.pins, ["A/b", "c"]);
        assert_eq!(cfg.sync_interval_secs, 60);
        assert_eq!(cfg.service, Config::default().service);
        assert!(mtime.is_some());
        std::fs::write(dir.join("context_menu"), "perhaps").unwrap();
        let err = read_cosmic(&dir).unwrap_err();
        assert!(err.contains("context_menu"), "{err}");
        assert_eq!(read_cosmic(&dir.join("nope")), Ok(None));
        let _ = std::fs::remove_dir_all(dir);
    }
}
