//! The Settings pages from a schema (kit-server-settings): settings.toml describes Caddy's global options as
//! one flat table, and this module maps each key onto the panel's settings document ([`Settings`]), so the
//! Caddyfile generator, backups, DDNS, updates and Security keep reading the same values.
//!
//! Routes (admins): `GET/PUT /api/settings/form`, `GET /api/settings/form/schema`,
//! `GET /api/settings/form/secret/{key}`. The panel's own `GET/PUT /api/settings` (the whole document) stays.

use crate::model::Settings;
use crate::server::AppState;
use kit_server_core::Result;
use kit_server_settings::{SettingsPaths, Table, Values};
use serde_json::{Map, Value, json};

/// What build.rs generated from settings.toml (`Keys` names each setting; the table is what's used here).
#[allow(dead_code)]
mod generated {
    pub use kit_server_settings::{DefaultValue, Field, Group, KeyDef, Section};
    include!(concat!(env!("OUT_DIR"), "/settings_gen.rs"));
}
use generated::{SCHEMA, TABLE};

/// The table settings.toml describes.
pub static SETTINGS: Table = Table {
    keys: TABLE,
    sections: SCHEMA,
};

/// Where the routes are (the panel's `/api/settings` is the whole document).
pub fn paths() -> SettingsPaths {
    SettingsPaths {
        values: "/api/settings/form".into(),
        schema: "/api/settings/form/schema".into(),
        secret: "/api/settings/form/secret/{key}".into(),
    }
}

/// The secret kept outside the document, in the encrypted secrets table.
const EAB_MAC: &str = "acme_eab_mac";

fn lines(v: &[String]) -> String {
    v.join("\n")
}

fn split_list(t: &str) -> Vec<String> {
    t.split(['\n', ','])
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "on" | "1"
        ),
        _ => false,
    }
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Value::String(s) => s.trim().parse().unwrap_or(0),
        Value::Bool(b) => *b as i64,
        _ => 0,
    }
}

/// The value of `key` in `s` (None for a key the document doesn't hold).
pub fn read(s: &Settings, key: &str) -> Option<Value> {
    Some(match key {
        "default_domain" => json!(s.default_domain),
        "acme_email" => json!(s.acme_email),
        "acme_ca" => json!(s.acme_ca),
        "acme_ca_url" => json!(s.acme_ca_url),
        "acme_ca_root" => json!(s.acme_ca_root),
        "acme_eab_key_id" => json!(s.acme_eab_key_id),
        "cert_alert_days" => json!(
            s.cert_alert_days
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "resolver_preset" => json!(s.resolver_preset),
        "resolvers" => json!(lines(&s.resolvers)),
        "propagation_delay" => json!(s.propagation_delay),
        "propagation_timeout" => json!(s.propagation_timeout),
        "scan_ranges" => json!(lines(&s.scan_ranges)),
        "trusted_proxies" => json!(s.trusted_proxies.enabled),
        "trusted_proxy_presets" => json!(s.trusted_proxies.presets.join(",")),
        "trusted_proxy_ranges" => json!(lines(&s.trusted_proxies.custom)),
        "default_access_log" => json!(s.default_access_log),
        "log_roll_size" => json!(s.log_roll_size),
        "log_roll_keep" => json!(s.log_roll_keep),
        "metrics" => json!(s.metrics),
        "dry_run" => json!(s.dry_run),
        "session_timeout_minutes" => json!(s.session_timeout_minutes),
        "panel_domain" => json!(s.panel_domain),
        "raw_global" => json!(s.raw_global),
        _ => return None,
    })
}

/// Sets `key` in `s` to `v` (taking the setting's type); false for a key the document doesn't hold.
pub fn write(s: &mut Settings, key: &str, v: &Value) -> bool {
    match key {
        "default_domain" => s.default_domain = text(v).trim().to_ascii_lowercase(),
        "acme_email" => s.acme_email = text(v).trim().to_string(),
        "acme_ca" => s.acme_ca = text(v),
        "acme_ca_url" => s.acme_ca_url = text(v).trim().to_string(),
        "acme_ca_root" => s.acme_ca_root = text(v).trim().to_string(),
        "acme_eab_key_id" => s.acme_eab_key_id = text(v).trim().to_string(),
        "cert_alert_days" => {
            s.cert_alert_days = split_list(&text(v))
                .iter()
                .filter_map(|d| d.parse::<i64>().ok())
                .filter(|d| *d > 0)
                .collect()
        }
        "resolver_preset" => s.resolver_preset = text(v),
        "resolvers" => s.resolvers = split_list(&text(v)),
        "propagation_delay" => s.propagation_delay = text(v).trim().to_string(),
        "propagation_timeout" => s.propagation_timeout = text(v).trim().to_string(),
        "scan_ranges" => s.scan_ranges = split_list(&text(v)),
        "trusted_proxies" => s.trusted_proxies.enabled = truthy(v),
        "trusted_proxy_presets" => s.trusted_proxies.presets = split_list(&text(v)),
        "trusted_proxy_ranges" => s.trusted_proxies.custom = split_list(&text(v)),
        "default_access_log" => s.default_access_log = truthy(v),
        "log_roll_size" => s.log_roll_size = text(v).trim().to_string(),
        "log_roll_keep" => s.log_roll_keep = int(v).max(0),
        "metrics" => s.metrics = truthy(v),
        "dry_run" => s.dry_run = truthy(v),
        "session_timeout_minutes" => s.session_timeout_minutes = int(v),
        "panel_domain" => s.panel_domain = text(v).trim().to_ascii_lowercase(),
        "raw_global" => s.raw_global = text(v),
        _ => return false,
    }
    true
}

#[kit_server_core::async_trait]
impl Values for AppState {
    fn table(&self) -> Table {
        SETTINGS
    }

    fn get(&self, key: &str) -> Value {
        if key == EAB_MAC {
            return json!(self.store.secret(EAB_MAC).unwrap_or_default());
        }
        read(&self.store.settings(), key).unwrap_or(Value::Null)
    }

    fn env_locked(&self, _key: &str) -> bool {
        false
    }

    async fn set_many(&self, values: &Value) -> Result<()> {
        let Some(values) = values.as_object() else {
            return Ok(());
        };
        let mut s = self.store.settings();
        for (k, v) in values {
            if k != EAB_MAC {
                write(&mut s, k, v);
            }
        }
        crate::server::resources::check_settings(self, &mut s)
            .map_err(|e| kit_server_core::Error::new(400, e))?;
        let fail = |e: anyhow::Error| {
            kit_server_core::Error::new(500, format!("The settings can't be saved: {e:#}"))
        };
        self.store
            .update_settings(|cur| {
                let setup = cur.setup_complete;
                *cur = s;
                cur.setup_complete = setup || cur.setup_complete;
            })
            .map_err(fail)?;
        if let Some(mac) = values.get(EAB_MAC) {
            self.store.set_secret(EAB_MAC, &text(mac)).map_err(fail)?;
        }
        Ok(())
    }

    fn dump(&self) -> Value {
        let s = self.store.settings();
        let mut out = Map::new();
        let mut meta = Map::new();
        for def in SETTINGS.keys {
            let v = if def.key == EAB_MAC {
                self.get(EAB_MAC)
            } else {
                read(&s, def.key).unwrap_or(Value::Null)
            };
            let source = if v == def.default.to_json() {
                "default"
            } else {
                "ui"
            };
            let shown = if def.secret {
                kit_server_settings::values::masked(&v)
            } else {
                v
            };
            out.insert(def.key.to_string(), shown);
            meta.insert(
                def.key.to_string(),
                json!({ "source": source, "env": def.env }),
            );
        }
        out.insert("_meta".into(), Value::Object(meta));
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_maps_onto_the_document() {
        let s = Settings::default();
        for def in SETTINGS.keys {
            if def.key == EAB_MAC {
                continue;
            }
            let v = read(&s, def.key).unwrap_or_else(|| panic!("{} is not read", def.key));
            let mut t = Settings::default();
            assert!(write(&mut t, def.key, &v), "{} is not written", def.key);
            assert_eq!(
                read(&t, def.key),
                Some(v),
                "{} does not round-trip",
                def.key
            );
        }
    }

    #[test]
    fn lists_and_types_from_the_form() {
        let mut s = Settings::default();
        write(&mut s, "cert_alert_days", &json!("30, 7,x, 0"));
        write(
            &mut s,
            "scan_ranges",
            &json!("10.0.2.0/24\n\n 10.0.3.0/24 "),
        );
        write(
            &mut s,
            "trusted_proxy_presets",
            &json!("cloudflare,private"),
        );
        write(&mut s, "log_roll_keep", &json!("9"));
        write(&mut s, "metrics", &json!("false"));
        assert_eq!(s.cert_alert_days, vec![30, 7]);
        assert_eq!(s.scan_ranges, vec!["10.0.2.0/24", "10.0.3.0/24"]);
        assert_eq!(s.trusted_proxies.presets, vec!["cloudflare", "private"]);
        assert_eq!((s.log_roll_keep, s.metrics), (9, false));
        assert!(!write(&mut s, "setup_complete", &json!(true)));
    }
}
