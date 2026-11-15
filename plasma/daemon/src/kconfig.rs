// SPDX-License-Identifier: GPL-3.0-only
//! A `ConfigStore` backed by a KConfig-style INI file (`~/.config/rclone-plasmarc`), group
//! `[General]`, one key per `Config` field. Hand-written: the format is tiny.

use rclone_ui_core::config::{Config, ConfigStore};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const GROUP: &str = "General";
const KEYS: [&str; 9] = [
    "rc_addr",
    "env_file",
    "service",
    "mount_path",
    "cache_dir",
    "context_menu",
    "offline_enabled",
    "pins",
    "sync_interval_secs",
];

pub struct KConfigStore {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl KConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_lock: Mutex::new(()),
        }
    }

    /// `$XDG_CONFIG_HOME/rclone-plasmarc`, defaulting to `~/.config`.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("rclone-plasmarc")
    }

    fn read(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_default()
    }
}

impl ConfigStore for KConfigStore {
    fn load(&self) -> Config {
        parse(&self.read())
    }

    fn save(&self, f: &mut dyn FnMut(&mut Config)) -> Result<Config, String> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let old = self.read();
        let mut cfg = parse(&old);
        f(&mut cfg);
        let text = render(&old, &cfg);
        write_atomic(&self.path, &text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(cfg)
    }
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// `KConfig` value escaping. `list` also escapes the item separator.
fn escape(s: &str, list: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let end = s.chars().count().saturating_sub(1);
    for (i, c) in s.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ',' if list => out.push_str("\\,"),
            ' ' if i == 0 || i == end => out.push_str("\\s"),
            c => out.push(c),
        }
    }
    out
}

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

pub fn parse(text: &str) -> Config {
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

/// The file text for `cfg`, keeping other groups and unknown `[General]` keys from `old`.
fn render(old: &str, cfg: &Config) -> String {
    let pins: Vec<String> = cfg.pins.iter().map(|p| escape(p, true)).collect();
    let values = [
        escape(&cfg.rc_addr, false),
        escape(&cfg.env_file, false),
        escape(&cfg.service, false),
        escape(&cfg.mount_path, false),
        escape(&cfg.cache_dir, false),
        cfg.context_menu.to_string(),
        cfg.offline_enabled.to_string(),
        pins.join(","),
        cfg.sync_interval_secs.to_string(),
    ];
    let mut general = format!("[{GROUP}]\n");
    for (k, v) in KEYS.iter().zip(values) {
        let _ = writeln!(general, "{k}={v}");
    }
    let mut rest = String::new();
    let mut in_general = false;
    for line in old.lines() {
        let t = line.trim_start();
        if t.starts_with('[') {
            in_general = t.trim_end() == format!("[{GROUP}]");
            if in_general {
                continue;
            }
        } else if in_general {
            let key = t.split_once('=').map(|(k, _)| k.trim());
            if t.is_empty() {
                continue;
            }
            if key.is_some_and(|k| KEYS.contains(&k)) {
                continue;
            }
            general.push_str(line);
            general.push('\n');
            continue;
        }
        rest.push_str(line);
        rest.push('\n');
    }
    if rest.trim().is_empty() {
        general
    } else {
        format!("{general}\n{rest}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_file() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rclone-plasma-kconfig-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("sub").join("rclone-plasmarc")
    }

    #[test]
    fn defaults_when_missing_or_invalid() {
        let s = KConfigStore::new(temp_file());
        assert_eq!(s.load(), Config::default());
        let c = parse("[General]\ncontext_menu=maybe\nsync_interval_secs=-4\nrc_addr=x:1\n");
        assert_eq!(c.rc_addr, "x:1");
        assert!(c.context_menu);
        assert_eq!(c.sync_interval_secs, 300);
        // Keys in other groups are ignored.
        assert_eq!(parse("[Other]\nrc_addr=bad\n"), Config::default());
    }

    #[test]
    fn round_trip() {
        let path = temp_file();
        let s = KConfigStore::new(path.clone());
        let saved = s
            .save(&mut |c| {
                c.rc_addr = "10.0.0.1:9".into();
                c.offline_enabled = true;
                c.context_menu = false;
                c.sync_interval_secs = 42;
                c.pins = vec!["Projects".into(), "A,b\\c".into(), "x/y z".into()];
            })
            .unwrap();
        assert_eq!(s.load(), saved);
        assert_eq!(KConfigStore::new(path.clone()).load(), saved);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[General]\n"));
        assert!(text.contains("pins=Projects,A\\,b\\\\c,x/y z\n"), "{text}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn escaping() {
        assert_eq!(escape("a,b\\c", true), "a\\,b\\\\c");
        assert_eq!(escape("a,b", false), "a,b");
        assert_eq!(escape(" x ", false), "\\sx\\s");
        assert_eq!(unescape(&escape(" a\tb\nc\\,d ", false)), " a\tb\nc\\,d ");
        assert_eq!(split_list("a\\,b,c\\\\,d"), vec!["a\\,b", "c\\\\", "d"]);
        assert_eq!(parse("[General]\npins=\n").pins, Vec::<String>::new());
    }

    #[test]
    fn save_keeps_foreign_content() {
        let path = temp_file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[General]\nextra=1\nrc_addr=old\n\n[Other]\nk=v\n").unwrap();
        let s = KConfigStore::new(path.clone());
        s.save(&mut |c| c.rc_addr = "new".into()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("extra=1\n") && text.contains("[Other]\nk=v\n"));
        assert!(text.contains("rc_addr=new\n") && !text.contains("old"));
        assert_eq!(s.load().rc_addr, "new");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
