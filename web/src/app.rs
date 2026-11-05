//! What every page shares: who is signed in (`/api/me`), Caddy's status (`/api/status`), the CSRF token, the
//! theme and the address.

use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::Value;
use std::cell::RefCell;

thread_local! {
    static ME: ArcRwSignal<Value> = ArcRwSignal::new(Value::Null);
    static STATUS: ArcRwSignal<Value> = ArcRwSignal::new(Value::Null);
    static STATUS_STARTED: RefCell<bool> = const { RefCell::new(false) };
}

/// `/api/me`: `{user, csrf, needs_setup, pending_2fa, setup_complete, setup_token_required, version}`.
pub fn me() -> ArcRwSignal<Value> {
    ME.with(Clone::clone)
}

/// Keeps a `/api/me` answer (or a login answer with a new CSRF token).
pub fn set_me(v: Value) {
    me().set(v);
}

/// The CSRF token every write sends.
pub fn csrf() -> String {
    s(&me().get_untracked(), "csrf")
}

/// The signed-in account's role (`admin`, `editor`, `viewer`).
pub fn role() -> String {
    me().get_untracked().get("user").map(|u| s(u, "role")).unwrap_or_default()
}

/// Whether the account may change sites and snippets.
pub fn can_edit() -> bool {
    matches!(role().as_str(), "admin" | "editor")
}

/// Whether the account is an admin.
pub fn is_admin() -> bool {
    role() == "admin"
}

/// `/api/status`, refreshed every 20 seconds while the page shows.
pub fn status() -> ArcRwSignal<Value> {
    STATUS.with(Clone::clone)
}

/// Reads `/api/status` now.
pub fn refresh_status() {
    rt::spawn(async {
        if let Ok(v) = api::get("/status").await {
            status().set(v);
        }
    });
}

/// Starts refreshing the status (once).
pub fn start_status() {
    if STATUS_STARTED.with(|s| s.replace(true)) {
        return;
    }
    refresh_status();
    rt::every(20_000, || {
        let hidden = rt::document().map(|d| d.hidden()).unwrap_or(false);
        if !hidden {
            refresh_status();
        }
    });
}

/// A string field of a JSON object, "" when missing.
pub fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A bool field, false when missing.
pub fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// A number field, 0 when missing.
pub fn i(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

/// An array field, empty when missing.
pub fn a(v: &Value, k: &str) -> Vec<Value> {
    v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

// ---------------------------------------------------------------- theme

/// Where the theme choice is kept in this browser.
const THEME_KEY: &str = "caddymium.theme";

/// The themes people pick from: the kit's four, and following the system.
pub const THEMES: &[(&str, &str)] = &[("encore", "Encore"), ("dark", "Dark"), ("oled", "Black"), ("light", "Light"), ("system", "System")];

/// The theme chosen in this browser (`encore` by default).
pub fn theme() -> String {
    rt::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(THEME_KEY).ok().flatten())
        .unwrap_or_else(|| "encore".into())
}

/// Keeps `t` as this browser's theme and shows it.
pub fn set_theme(t: &str) {
    if let Some(s) = rt::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(THEME_KEY, t);
    }
    apply_theme();
}

/// Shows the chosen theme: `<html data-theme>`; System follows the device's light or dark mode.
pub fn apply_theme() {
    let mut t = theme();
    if t == "system" {
        let dark = rt::window().and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten()).is_some_and(|m| m.matches());
        t = if dark { "encore".into() } else { "light".into() };
    }
    if let Some(el) = rt::document().and_then(|d| d.document_element()) {
        let _ = el.set_attribute("data-theme", &t);
    }
}

// ---------------------------------------------------------------- address

/// The page the address names: `#/sites/3?tab=tls` → path `/sites/3`, query `tab=tls`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Route {
    /// the part after `#`, before `?`
    pub path: String,
    /// the part after `?`
    pub query: String,
}

impl Route {
    /// The route of a `location.hash`.
    pub fn parse(hash: &str) -> Route {
        let h = hash.trim_start_matches('#');
        let (path, query) = h.split_once('?').unwrap_or((h, ""));
        Route { path: if path.is_empty() { "/".into() } else { path.to_string() }, query: query.to_string() }
    }

    /// The address's route now.
    pub fn current() -> Route {
        Route::parse(&rt::window().and_then(|w| w.location().hash().ok()).unwrap_or_default())
    }

    /// The path's parts: `/sites/3` → `["sites", "3"]`.
    pub fn parts(&self) -> Vec<&str> {
        self.path.split('/').filter(|p| !p.is_empty()).collect()
    }

    /// The first part (`sites`), `sites` for the start page.
    pub fn page(&self) -> &str {
        self.path.split('/').find(|p| !p.is_empty()).unwrap_or("sites")
    }

    /// The value of `key` in the query, "" when it is not there.
    pub fn param(&self, key: &str) -> String {
        self.query.split('&').filter_map(|p| p.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v.to_string()).unwrap_or_default()
    }
}

/// Opens `path` of the app (`/sites/3`).
pub fn navigate(path: &str) {
    if let Some(w) = rt::window() {
        let _ = w.location().set_hash(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn routes_come_from_the_hash() {
        assert_eq!(Route::parse("").page(), "sites");
        let r = Route::parse("#/sites/3?tab=tls&x=1");
        assert_eq!(r.parts(), vec!["sites", "3"]);
        assert_eq!(r.page(), "sites");
        assert_eq!(r.param("tab"), "tls");
        assert_eq!(r.param("missing"), "");
    }

    #[test]
    fn fields_read_plainly() {
        let v = json!({"a": "x", "n": 3, "t": true, "l": [1]});
        assert_eq!((s(&v, "a"), s(&v, "n"), s(&v, "zz")), ("x".into(), "3".into(), String::new()));
        assert!(b(&v, "t") && !b(&v, "a"));
        assert_eq!((i(&v, "n"), a(&v, "l").len()), (3, 1));
    }
}
