// SPDX-License-Identifier: GPL-3.0-only
//! `GetConfig` / `SetConfig` JSON handling.

use rclone_ui_core::config::{store, Config};
use serde_json::Value;

pub fn get_json(cfg: &Config) -> String {
    serde_json::to_string(cfg).unwrap_or_else(|_| "{}".into())
}

/// `cfg` with the fields of the JSON object `json` replaced. Unknown fields and wrongly typed
/// values are errors.
pub fn merge(cfg: &Config, json: &str) -> Result<Config, String> {
    let Value::Object(patch) = serde_json::from_str::<Value>(json).map_err(|e| e.to_string())?
    else {
        return Err("expected a JSON object".into());
    };
    let Value::Object(mut base) = serde_json::to_value(cfg).map_err(|e| e.to_string())? else {
        return Err("config is not an object".into());
    };
    for (k, v) in patch {
        if !base.contains_key(&k) {
            return Err(format!("unknown field: {k}"));
        }
        let (old, new) = (&base[&k], &v);
        let same_kind = matches!(
            (old, new),
            (Value::String(_), Value::String(_))
                | (Value::Bool(_), Value::Bool(_))
                | (Value::Array(_), Value::Array(_))
        ) || (old.is_number() && new.is_u64());
        if !same_kind {
            return Err(format!("wrong type for field: {k}"));
        }
        base.insert(k, v);
    }
    serde_json::from_value(Value::Object(base)).map_err(|e| e.to_string())
}

/// Apply `json` to the saved config. Returns `""` on success or an error message.
pub fn set(json: &str) -> String {
    if let Err(e) = merge(&store().load(), json) {
        return e;
    }
    match store().save(&mut |c| {
        if let Ok(n) = merge(c, json) {
            *c = n;
        }
    }) {
        Ok(_) => String::new(),
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_known_fields() {
        let c = merge(
            &Config::default(),
            r#"{"rc_addr":"h:1","pins":["a","b"],"offline_enabled":true,"sync_interval_secs":9}"#,
        )
        .unwrap();
        assert_eq!(c.rc_addr, "h:1");
        assert_eq!(c.pins, ["a", "b"]);
        assert!(c.offline_enabled);
        assert_eq!(c.sync_interval_secs, 9);
        assert_eq!(c.service, Config::default().service);
        assert_eq!(merge(&Config::default(), "{}").unwrap(), Config::default());
    }

    #[test]
    fn rejects_bad_input() {
        let d = Config::default();
        assert!(merge(&d, r#"{"nope":1}"#)
            .unwrap_err()
            .contains("unknown field: nope"));
        assert!(merge(&d, r#"{"rc_addr":5}"#)
            .unwrap_err()
            .contains("rc_addr"));
        assert!(merge(&d, r#"{"context_menu":"yes"}"#).is_err());
        assert!(merge(&d, r#"{"sync_interval_secs":-1}"#).is_err());
        assert!(merge(&d, r#"{"sync_interval_secs":1.5}"#).is_err());
        assert!(merge(&d, r#"{"pins":[1]}"#).is_err());
        assert!(merge(&d, "[1]").is_err());
        assert!(merge(&d, "not json").is_err());
    }
}
