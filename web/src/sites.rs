//! What the Sites list and the site editor share: kinds, the "target" line, base domains for the hostname
//! picker, app templates, and the editor's defaults and save body.

use crate::app::{a, b, i, s};
use serde_json::{json, Map, Value};

/// A site kind as people know it.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "proxy" => "Reverse proxy",
        "static" => "File server",
        "redirect" => "Redirect",
        "respond" => "Static response",
        "forward_proxy" => "Forward proxy",
        _ => "Custom (raw)",
    }
}

/// The kinds the editor offers: (id, icon, label, sentence).
pub const KINDS: &[(&str, &str, &str, &str)] = &[
    ("proxy", "server", "Reverse proxy", "Forward to an app"),
    ("static", "folder", "File server", "Serve a folder"),
    ("redirect", "corner-up-right", "Redirect", "Send elsewhere"),
    ("respond", "message", "Response", "Fixed text & status"),
    ("forward_proxy", "arrow-left-right", "Forward proxy", "HTTPS proxy for devices"),
    ("none", "code", "Custom", "Raw Caddyfile"),
];

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// What a site points at, in one line.
pub fn target(site: &Value) -> String {
    let c = &site["config"];
    match s(site, "kind").as_str() {
        "proxy" => strings(&c["proxy"]["upstreams"]).join(", "),
        "static" => s(&c["static"], "root"),
        "redirect" => s(&c["redirect"], "to"),
        "respond" => {
            let st = i(&c["respond"], "status");
            let body: String = s(&c["respond"], "body").chars().take(40).collect();
            format!("{} · {body}", if st == 0 { 200 } else { st })
        }
        "forward_proxy" => match a(&c["forward_proxy"], "basic_auth").len() {
            0 => "open proxy (no login)".into(),
            n => format!("{n} login(s)"),
        },
        _ => "custom Caddyfile".into(),
    }
}

/// The address a site opens at, if it has one to open.
pub fn url(site: &Value) -> Option<String> {
    let h = strings(&site["hosts"]).into_iter().next()?;
    if h.starts_with("*.") || h.is_empty() {
        return None;
    }
    let scheme = if s(&site["config"]["tls"], "mode") == "off" { "http" } else { "https" };
    Some(format!("{scheme}://{h}"))
}

/// Whether a site has a login, SSO or an IP allow list.
pub fn protected(site: &Value) -> bool {
    let ac = &site["config"]["access"];
    !a(ac, "basic_auth").is_empty() || ac["forward_auth"].is_object() || !a(ac, "allow_cidrs").is_empty()
}

/// Whether the search `q` (lower case) finds `site`.
pub fn found(site: &Value, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let mut hay = vec![s(site, "name")];
    hay.extend(strings(&site["hosts"]));
    hay.extend(strings(&site["tags"]));
    hay.extend(strings(&site["config"]["proxy"]["upstreams"]));
    hay.push(s(&site["config"]["redirect"], "to"));
    hay.join(" ").to_lowercase().contains(q)
}

// ---------------------------------------------------------------- base domains

fn is_ip(h: &str) -> bool {
    h.contains(':') || h.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// The domain a hostname belongs to ("app.example.com" → "example.com"), "" for IPs and local names.
pub fn base_domain(h: &str) -> String {
    let h = h.to_lowercase();
    let h = h.trim_start_matches("*.");
    if !h.contains('.') || is_ip(h) || h.ends_with(".localhost") || h == "localhost" {
        return String::new();
    }
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() <= 2 {
        h.to_string()
    } else {
        parts[1..].join(".")
    }
}

/// The domains to offer in the hostname picker: the default first, then Domains & DNS and the sites' own.
pub fn known_domains(domains: &[Value], sites: &[Value], default: &str) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    let def = default.to_lowercase();
    if !def.is_empty() {
        set.insert(def.clone());
    }
    for d in domains {
        let n = s(d, "name").to_lowercase();
        if !n.is_empty() {
            set.insert(n);
        }
    }
    for site in sites {
        for h in strings(&site["hosts"]) {
            let bd = base_domain(&h);
            if !bd.is_empty() {
                set.insert(bd);
            }
        }
    }
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort_by_key(|d| d != &def);
    out
}

/// One hostname row: a name under one of the known domains, or a full hostname (`custom`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HostRow {
    pub custom: bool,
    pub sub: String,
    pub domain: String,
    pub full: String,
}

/// Splits `host` into a name and the longest known domain it ends in.
pub fn split_host(host: &str, list: &[String]) -> HostRow {
    let host = host.trim().to_lowercase();
    let best = list.iter().filter(|d| host == **d || host.ends_with(&format!(".{d}"))).max_by_key(|d| d.len()).cloned();
    match best {
        None => HostRow { custom: true, full: host, ..Default::default() },
        Some(d) => {
            let sub = if host == d { String::new() } else { host[..host.len() - d.len() - 1].to_string() };
            HostRow { custom: false, sub, domain: d, full: host }
        }
    }
}

/// The hostname a row stands for.
pub fn join_host(r: &HostRow) -> String {
    if r.custom {
        return r.full.trim().to_lowercase();
    }
    let sub = r.sub.trim().to_lowercase();
    let sub = sub.trim_end_matches('.');
    match (sub.is_empty(), r.domain.is_empty()) {
        (_, true) => sub.to_string(),
        (true, false) => r.domain.clone(),
        (false, false) => format!("{sub}.{}", r.domain),
    }
}

// ---------------------------------------------------------------- templates

/// Starting points for common self-hosted apps: (id, name, site). Only defaults: the user still sets the
/// hostname and the upstream.
pub fn templates() -> Vec<(&'static str, &'static str, Value)> {
    let proxy = |up: &str, extra: Value, apps: &str, tag: &str| {
        let mut p = json!({ "upstreams": [up] });
        if let (Some(o), Some(x)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(x.clone());
        }
        let mut config = json!({ "proxy": p });
        if !apps.is_empty() {
            config["access"] = json!({ "shields": { "apps": apps } });
        }
        json!({ "kind": "proxy", "tags": [tag], "config": config })
    };
    vec![
        ("vaultwarden", "Vaultwarden", proxy("vaultwarden:80", json!({ "websocket": true }), "vaultwarden", "security")),
        ("jellyfin", "Jellyfin", proxy("jellyfin:8096", json!({ "websocket": true, "flush_interval": "-1" }), "jellyfin", "media")),
        ("plex", "Plex", proxy("plex:32400", json!({ "websocket": true }), "plex", "media")),
        (
            "immich",
            "Immich",
            json!({ "kind": "proxy", "tags": ["media"], "config": { "proxy": { "upstreams": ["immich-server:2283"], "websocket": true }, "max_body_size": "50GB", "access": { "shields": { "apps": "immich" } } } }),
        ),
        (
            "nextcloud",
            "Nextcloud",
            json!({ "kind": "proxy", "tags": ["files"], "config": { "proxy": { "upstreams": ["nextcloud:80"] }, "max_body_size": "10GB",
                "headers_set": { "Strict-Transport-Security": "max-age=31536000;" },
                "raw": "redir /.well-known/carddav /remote.php/dav 301\nredir /.well-known/caldav /remote.php/dav 301" } }),
        ),
        ("homeassistant", "Home Assistant", proxy("homeassistant:8123", json!({ "websocket": true }), "", "home")),
        ("proxmox", "Proxmox VE", proxy("https://192.168.1.10:8006", json!({ "tls_skip_verify": true, "websocket": true }), "", "infra")),
        (
            "filebrowser",
            "File Browser",
            json!({ "kind": "proxy", "tags": ["files"], "config": { "proxy": { "upstreams": ["filebrowser:80"] }, "max_body_size": "10GB" } }),
        ),
        ("homepage", "Homepage", proxy("homepage:3000", json!({}), "", "dashboard")),
        ("grafana", "Grafana", proxy("grafana:3000", json!({ "websocket": true }), "", "monitoring")),
        (
            "static",
            "Static website",
            json!({ "kind": "static", "config": { "static": { "root": "/srv/www" }, "compression": ["zstd", "gzip"] } }),
        ),
        (
            "proxy",
            "Forward proxy (HTTPS proxy for your devices)",
            json!({ "kind": "forward_proxy", "tags": ["network"], "config": { "forward_proxy": { "basic_auth": [{ "username": "", "password": "" }], "hide_ip": true, "hide_via": true, "ports": [80, 443] } } }),
        ),
    ]
}

// ---------------------------------------------------------------- editor model

/// A new site.
pub fn blank() -> Value {
    json!({
        "name": "", "hosts": [""], "kind": "proxy", "enabled": true, "tags": [],
        "config": { "proxy": { "upstreams": [""] }, "tls": { "mode": "auto" }, "access": {}, "routes": [] },
    })
}

fn obj<'a>(v: &'a mut Value, k: &str) -> &'a mut Value {
    if !v[k].is_object() {
        v[k] = json!({});
    }
    &mut v[k]
}

fn list<'a>(v: &'a mut Value, k: &str) -> &'a mut Value {
    if !v[k].is_array() {
        v[k] = json!([]);
    }
    &mut v[k]
}

fn text_or(v: &mut Value, k: &str, d: &str) {
    if s(v, k).is_empty() {
        v[k] = json!(d);
    }
}

/// Fills in what the editor's fields need, plus the editor-only fields (`_tags`, `_ports`, `_access_log`).
pub fn prepare(mut site: Value) -> Value {
    let tags = strings(&site["tags"]).join(", ");
    {
        let c = obj(&mut site, "config");
        text_or(obj(c, "tls"), "mode", "auto");
        list(c, "routes");
        let p = obj(c, "proxy");
        if a(p, "upstreams").is_empty() {
            p["upstreams"] = json!([""]);
        }
        if p["lb_policy"].is_null() {
            p["lb_policy"] = json!("");
        }
        text_or(obj(c, "static"), "root", "");
        let r = obj(c, "redirect");
        if i(r, "code") == 0 {
            r["code"] = json!(302);
        }
        let rs = obj(c, "respond");
        if i(rs, "status") == 0 {
            rs["status"] = json!(200);
        }
        let fp = obj(c, "forward_proxy");
        for k in ["basic_auth", "ports", "acl"] {
            list(fp, k);
        }
        let ports: Vec<String> = a(fp, "ports").iter().map(|p| p.to_string()).collect();
        let ac = obj(c, "access");
        for k in ["allow_cidrs", "deny_cidrs", "basic_auth"] {
            list(ac, k);
        }
        let crowdsec = b(ac, "crowdsec");
        let sh = obj(ac, "shields");
        for k in ["app_paths", "sso_users"] {
            list(sh, k);
        }
        if crowdsec && sh["crowdsec"].is_null() {
            sh["crowdsec"] = json!(true);
        }
        for k in ["headers_remove", "compression", "snippets"] {
            list(c, k);
        }
        text_or(c, "www_redirect", "");
        let log = match c["access_log"].as_bool() {
            None => "default",
            Some(true) => "on",
            Some(false) => "off",
        };
        c["_access_log"] = json!(log);
        c["_ports"] = json!(ports.join(", "));
    }
    if a(&site, "hosts").is_empty() {
        site["hosts"] = json!([""]);
    }
    site["_tags"] = json!(tags);
    site
}

fn trimmed(v: &Value) -> Value {
    Value::Array(strings(v).iter().map(|x| x.trim()).filter(|x| !x.is_empty()).map(|x| json!(x)).collect())
}

/// The body to save: editor-only fields and the unused kinds' settings left out, lists trimmed, numbers typed.
pub fn payload(site: &Value) -> Value {
    let mut out = site.clone();
    let kind = s(site, "kind");
    out["tags"] = Value::Array(s(site, "_tags").split(',').map(str::trim).filter(|t| !t.is_empty()).map(|t| json!(t)).collect());
    out["hosts"] = trimmed(&site["hosts"]);
    if let Some(o) = out.as_object_mut() {
        o.remove("_tags");
    }
    let c = &mut out["config"];
    let log = s(c, "_access_log");
    c["access_log"] = match log.as_str() {
        "on" => json!(true),
        "off" => json!(false),
        _ => Value::Null,
    };
    let ports: Vec<Value> =
        s(c, "_ports").split(',').filter_map(|p| p.trim().parse::<i64>().ok()).filter(|p| *p > 0).map(Value::from).collect();
    if let Some(o) = c.as_object_mut() {
        o.remove("_access_log");
        o.remove("_ports");
    }
    // shields: "default" is no value at all
    if let Some(sh) = c["access"]["shields"].as_object_mut() {
        sh.retain(|_, v| !v.is_null());
        sh.insert("app_paths".into(), trimmed(&sh.get("app_paths").cloned().unwrap_or_default()));
        sh.insert("sso_users".into(), trimmed(&sh.get("sso_users").cloned().unwrap_or_default()));
    }
    c["access"]["crowdsec"] = json!(false);
    if c["forward_proxy"].is_object() {
        let fp = &mut c["forward_proxy"];
        fp["ports"] = Value::Array(ports);
        fp["acl"] = trimmed(&fp["acl"]);
        let logins: Vec<Value> =
            a(fp, "basic_auth").into_iter().filter(|u| !s(u, "username").trim().is_empty() || !s(u, "password").is_empty()).collect();
        fp["basic_auth"] = Value::Array(logins);
    }
    for k in ["proxy", "static", "redirect", "respond", "forward_proxy"] {
        if kind != k {
            if let Some(o) = c.as_object_mut() {
                o.remove(k);
            }
        }
    }
    if c["proxy"].is_object() {
        c["proxy"]["upstreams"] = trimmed(&c["proxy"]["upstreams"]);
    }
    if c["redirect"].is_object() {
        let code = i(&c["redirect"], "code");
        c["redirect"]["code"] = json!(if code == 0 { 302 } else { code });
    }
    if c["respond"].is_object() {
        let st = i(&c["respond"], "status");
        c["respond"]["status"] = json!(if st == 0 { 200 } else { st });
    }
    for k in ["allow_cidrs", "deny_cidrs"] {
        c["access"][k] = trimmed(&c["access"][k]);
    }
    c["headers_remove"] = trimmed(&c["headers_remove"]);
    let ac = &mut c["access"];
    if ac["rate_limit"].is_object() {
        let ev = i(&ac["rate_limit"], "events");
        if ev == 0 {
            ac.as_object_mut().map(|o| o.remove("rate_limit"));
        } else {
            ac["rate_limit"]["events"] = json!(ev);
        }
    }
    if ac["forward_auth"].is_object() && s(&ac["forward_auth"], "upstream").is_empty() {
        ac.as_object_mut().map(|o| o.remove("forward_auth"));
    }
    if let Some(routes) = c["routes"].as_array_mut() {
        for r in routes {
            let rk = s(r, "kind");
            if let Some(o) = r.as_object_mut() {
                o.retain(|k, _| !matches!(k.as_str(), "proxy" | "static" | "redirect" | "respond") || *k == rk);
            }
        }
    }
    for k in ["headers_set"] {
        if c[k].as_object().is_some_and(Map::is_empty) {
            c.as_object_mut().map(|o| o.remove(k));
        }
    }
    out
}

/// A site made from template `t`: the template's settings over a blank site, keeping a hostname already typed.
pub fn from_template(t: &Value, id: &str, name: &str, current: &Value, typed_hosts: bool, default_domain: &str) -> Value {
    let mut site = blank();
    if let (Some(o), Some(x)) = (site.as_object_mut(), t.as_object()) {
        for (k, v) in x {
            if k != "config" {
                o.insert(k.clone(), v.clone());
            }
        }
    }
    if let (Some(o), Some(x)) = (site["config"].as_object_mut(), t["config"].as_object()) {
        o.extend(x.clone());
    }
    site["hosts"] = if typed_hosts {
        current["hosts"].clone()
    } else if default_domain.is_empty() {
        json!([""])
    } else {
        json!([format!("{id}.{default_domain}")])
    };
    let cur = s(current, "name");
    site["name"] = json!(if cur.is_empty() { name.to_string() } else { cur });
    prepare(site)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_split_and_join() {
        let list = vec!["example.com".to_string(), "home.example.com".to_string()];
        let r = split_host("app.home.example.com", &list);
        assert_eq!((r.sub.as_str(), r.domain.as_str(), r.custom), ("app", "home.example.com", false));
        assert_eq!(join_host(&r), "app.home.example.com");
        assert!(split_host("other.org", &list).custom);
        let apex = split_host("example.com", &list);
        assert_eq!((apex.sub.as_str(), join_host(&apex).as_str()), ("", "example.com"));
        assert_eq!(base_domain("a.b.example.com"), "b.example.com");
        assert_eq!(base_domain("10.0.0.1"), "");
        assert_eq!(base_domain("app.localhost"), "");
        let sites = vec![json!({ "hosts": ["x.foo.org"] })];
        assert_eq!(known_domains(&[json!({ "name": "bar.net" })], &sites, "zed.io"), vec!["zed.io", "bar.net", "foo.org"]);
    }

    #[test]
    fn editor_round_trip() {
        let mut site = prepare(blank());
        site["_tags"] = json!("media, home ,");
        site["hosts"] = json!([" app.example.com ", ""]);
        site["config"]["proxy"]["upstreams"] = json!(["jellyfin:8096", " "]);
        site["config"]["access"]["shields"]["bots"] = Value::Null;
        site["config"]["access"]["shields"]["crowdsec"] = json!(true);
        site["config"]["_access_log"] = json!("off");
        let p = payload(&site);
        assert_eq!(p["tags"], json!(["media", "home"]));
        assert_eq!(p["hosts"], json!(["app.example.com"]));
        assert_eq!(p["config"]["proxy"]["upstreams"], json!(["jellyfin:8096"]));
        assert_eq!(p["config"]["access_log"], json!(false));
        assert!(p["config"]["static"].is_null() && p["config"]["redirect"].is_null());
        assert!(p["config"]["access"]["shields"].get("bots").is_none());
        assert_eq!(p["config"]["access"]["shields"]["crowdsec"], json!(true));
        assert!(p.get("_tags").is_none() && p["config"].get("_ports").is_none());
    }

    #[test]
    fn forward_proxy_ports_and_templates() {
        let mut site =
            prepare(json!({ "kind": "forward_proxy", "hosts": ["p.example.com"], "config": { "forward_proxy": { "ports": [80, 443] } } }));
        assert_eq!(s(&site["config"], "_ports"), "80, 443");
        site["config"]["_ports"] = json!("443, x, 8443");
        assert_eq!(payload(&site)["config"]["forward_proxy"]["ports"], json!([443, 8443]));
        let (id, name, t) = templates().into_iter().find(|t| t.0 == "jellyfin").unwrap();
        let made = from_template(&t, id, name, &blank(), false, "example.com");
        assert_eq!(made["hosts"], json!(["jellyfin.example.com"]));
        assert_eq!(s(&made, "name"), "Jellyfin");
        assert_eq!(made["config"]["proxy"]["upstreams"], json!(["jellyfin:8096"]));
        assert_eq!(target(&made), "jellyfin:8096");
    }
}
