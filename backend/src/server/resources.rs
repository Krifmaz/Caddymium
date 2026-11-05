use axum::extract::{Extension, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{ApiError, ApiResult, AppState, Client, Id, JsonBody, Principal, json, ok, ok_true};
use crate::auth;
use crate::generator;
use crate::importer::valid_host;
use crate::model::*;
use crate::registry::dns_provider_by_id;
use crate::updates;

// ---------------------------------------------------------------- sites

fn non_empty(v: &[String]) -> Vec<String> {
    v.iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Validates a site and hashes any plaintext basic-auth passwords (the UI
/// sends new passwords in the hash field).
pub fn normalize_site(st: &mut Site) -> Result<(), String> {
    st.name = st.name.trim().to_string();
    crate::shields::validate_overrides(&mut st.config.access.shields)?;
    let mut hosts: Vec<String> = Vec::new();
    for h in &st.hosts {
        let mut h = h.trim().to_ascii_lowercase();
        if h.is_empty() || hosts.contains(&h) {
            continue;
        }
        h = h
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();
        if !valid_host(h.trim_start_matches("*.")) {
            if h.contains(':') {
                return Err(format!("{h:?}: enter just the hostname, without a port"));
            }
            return Err(format!("{h:?} is not a valid hostname"));
        }
        hosts.push(h);
    }
    if hosts.is_empty() {
        return Err("add at least one hostname".into());
    }
    st.hosts = hosts;
    if st.name.is_empty() {
        st.name = st.hosts[0].clone();
    }
    if !ALL_KINDS.contains(&st.kind.as_str()) {
        return Err(format!("unknown site type {:?}", st.kind));
    }
    let c = &mut st.config;
    match st.kind.as_str() {
        KIND_PROXY => {
            let ups = c
                .proxy
                .as_ref()
                .map(|p| non_empty(&p.upstreams))
                .unwrap_or_default();
            if ups.is_empty() {
                return Err(
                    "a reverse proxy needs at least one upstream (e.g. localhost:8080)".into(),
                );
            }
            if let Some(p) = c.proxy.as_mut() {
                p.upstreams = ups;
            }
        }
        KIND_STATIC => {
            if c.r#static
                .as_ref()
                .map(|s| s.root.trim().is_empty())
                .unwrap_or(true)
            {
                return Err("a file server needs a root folder".into());
            }
        }
        KIND_REDIRECT => {
            if c.redirect
                .as_ref()
                .map(|r| r.to.trim().is_empty())
                .unwrap_or(true)
            {
                return Err("a redirect needs a target URL".into());
            }
        }
        KIND_RESPOND => {
            if c.respond.is_none() {
                c.respond = Some(RespondConfig::default());
            }
        }
        KIND_FORWARD_PROXY => {
            let fp = c.forward_proxy.get_or_insert_with(Default::default);
            fp.basic_auth
                .retain(|u| !u.username.trim().is_empty() || !u.password.is_empty());
            for u in &mut fp.basic_auth {
                u.username = u.username.trim().to_string();
                if u.username.is_empty() || u.password.is_empty() {
                    return Err("forward proxy: every login needs a username and a password".into());
                }
            }
            fp.acl = fp
                .acl
                .iter()
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .collect();
            for r in &fp.acl {
                let mut it = r.split_whitespace();
                let verb = it.next().unwrap_or("");
                if !matches!(verb, "allow" | "deny" | "allow_file" | "deny_file")
                    || it.next().is_none()
                {
                    return Err(format!(
                        "forward proxy: access rule {r:?} must be \"allow <target>\" or \"deny <target>\""
                    ));
                }
            }
            fp.ports.retain(|p| (1..=65535).contains(p));
        }
        _ => {}
    }
    for (i, rt) in c.routes.iter_mut().enumerate() {
        rt.path = rt.path.trim().to_string();
        if rt.path.is_empty() || !rt.path.starts_with('/') {
            return Err(format!(
                "route {}: path must start with / (e.g. /api/*)",
                i + 1
            ));
        }
        if rt.kind == KIND_PROXY
            && rt
                .proxy
                .as_ref()
                .map(|p| non_empty(&p.upstreams).is_empty())
                .unwrap_or(true)
        {
            return Err(format!("route {}: needs an upstream", rt.path));
        }
        if let Some(p) = rt.proxy.as_mut() {
            p.upstreams = non_empty(&p.upstreams);
        }
    }
    for u in &mut c.access.basic_auth {
        u.username = u.username.trim().to_string();
        if u.username.is_empty() {
            return Err("basic auth: username is required".into());
        }
        if !u.hash.starts_with("$2") {
            if u.hash.is_empty() {
                return Err(format!("basic auth: set a password for {}", u.username));
            }
            u.hash = auth::bcrypt_hash(&u.hash).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Rejects a hostname that another enabled site already serves: Caddy
/// refuses duplicate site addresses.
fn host_conflict(st: &AppState, site: &Site) -> Result<(), ApiError> {
    if !site.enabled {
        return Ok(());
    }
    let Ok(all) = st.store.sites() else {
        return Ok(());
    };
    for o in all {
        if o.id == site.id || !o.enabled {
            continue;
        }
        if let Some(h) = o.hosts.iter().find(|h| site.hosts.contains(h)) {
            return Err(ApiError::conflict(format!(
                "{h} is already used by site {:?}",
                o.name
            )));
        }
    }
    Ok(())
}

pub async fn list_sites(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.store.sites()?)
}

pub async fn get_site(State(st): State<Arc<AppState>>, Id(id): Id) -> ApiResult {
    ok(&st.store.site(id)?)
}

pub async fn create_site(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(mut site): JsonBody<Site>,
) -> ApiResult {
    site.id = 0;
    let site = tokio::task::spawn_blocking(move || normalize_site(&mut site).map(|_| site))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(ApiError::bad)?;
    host_conflict(&st, &site)?;
    let site = st.store.create_site(site)?;
    st.audit(
        Some(&p),
        &c,
        "site.create",
        &site.name,
        &site.hosts.join(", "),
    );
    Ok(json(StatusCode::CREATED, &site))
}

pub async fn update_site(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
    JsonBody(mut site): JsonBody<Site>,
) -> ApiResult {
    let cur = st.store.site(id)?;
    site.id = id;
    site.created_at = cur.created_at;
    if site.position == 0 {
        site.position = cur.position;
    }
    let site = tokio::task::spawn_blocking(move || normalize_site(&mut site).map(|_| site))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(ApiError::bad)?;
    host_conflict(&st, &site)?;
    st.store.update_site(&site)?;
    st.audit(Some(&p), &c, "site.update", &site.name, "");
    ok(&st.store.site(id)?)
}

pub async fn delete_site(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    let site = st.store.site(id)?;
    st.store.delete_site(id)?;
    st.audit(Some(&p), &c, "site.delete", &site.name, "");
    ok_true()
}

/// Copies a site, disabled, so it doesn't clash with the original.
pub async fn clone_site(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    let mut site = st.store.site(id)?;
    site.id = 0;
    site.position = 0;
    site.enabled = false;
    site.name.push_str(" (copy)");
    let site = st.store.create_site(site)?;
    st.audit(Some(&p), &c, "site.clone", &site.name, "");
    Ok(json(StatusCode::CREATED, &site))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct BulkReq {
    ids: Vec<i64>,
    /// enable, disable, delete, reorder
    action: String,
}

pub async fn bulk_sites(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(req): JsonBody<BulkReq>,
) -> ApiResult {
    match req.action.as_str() {
        "enable" | "disable" => {
            if req.action == "enable" {
                for id in &req.ids {
                    if let Ok(mut s) = st.store.site(*id) {
                        s.enabled = true;
                        host_conflict(&st, &s)?;
                    }
                }
            }
            st.store
                .set_sites_enabled(&req.ids, req.action == "enable")?;
        }
        "delete" => {
            let mut first_err = None;
            for id in &req.ids {
                if let Err(e) = st.store.delete_site(*id) {
                    first_err.get_or_insert(e);
                }
            }
            if let Some(e) = first_err {
                return Err(e.into());
            }
        }
        "reorder" => st.store.reorder_sites(&req.ids)?,
        _ => return Err(ApiError::bad("unknown action")),
    }
    st.audit(
        Some(&p),
        &c,
        &format!("site.{}", req.action),
        &format!("{} sites", req.ids.len()),
        "",
    );
    ok_true()
}

// ---------------------------------------------------------------- domains

pub async fn list_domains(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.store.domains()?)
}

async fn save_domain_inner(
    st: &AppState,
    p: &Principal,
    c: &Client,
    id: i64,
    mut d: Domain,
) -> ApiResult {
    d.id = id;
    d.name = d.name.trim().trim_start_matches("*.").to_ascii_lowercase();
    if d.name.is_empty() || !valid_host(&d.name) {
        return Err(ApiError::bad("enter a domain like example.com"));
    }
    match d.challenge.as_str() {
        "" => d.challenge = CHALLENGE_HTTP.into(),
        CHALLENGE_HTTP | CHALLENGE_TLS_ALPN => {}
        CHALLENGE_DNS => {
            let Some(cid) = d.credential_id else {
                return Err(ApiError::bad("DNS-01 needs a DNS provider account"));
            };
            if st.store.credential(cid).is_err() {
                return Err(ApiError::bad("that DNS provider account doesn't exist"));
            }
        }
        _ => return Err(ApiError::bad("unknown challenge type")),
    }
    if d.challenge != CHALLENGE_DNS {
        d.credential_id = None;
        d.wildcard = false;
    }
    let d = st.store.save_domain(d)?;
    st.audit(Some(p), c, "domain.save", &d.name, &d.challenge);
    ok(&d)
}

pub async fn save_domain(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(d): JsonBody<Domain>,
) -> ApiResult {
    save_domain_inner(&st, &p, &c, 0, d).await
}

pub async fn save_domain_id(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
    JsonBody(d): JsonBody<Domain>,
) -> ApiResult {
    save_domain_inner(&st, &p, &c, id, d).await
}

pub async fn delete_domain(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    st.store.delete_domain(id)?;
    st.audit(Some(&p), &c, "domain.delete", &id.to_string(), "");
    ok_true()
}

// ---------------------------------------------------------------- DNS credentials

/// A credential as the UI sees it: secret values are never returned, only
/// whether they are set (or the env:/file: reference).
#[derive(Serialize)]
pub struct CredView {
    id: i64,
    name: String,
    provider: String,
    fields: BTreeMap<String, String>,
    set: BTreeMap<String, bool>,
}

pub fn view_cred(c: &DnsCredential) -> CredView {
    let secret: BTreeMap<&str, bool> = dns_provider_by_id(&c.provider)
        .map(|p| p.fields.iter().map(|f| (f.key, f.secret)).collect())
        .unwrap_or_default();
    let mut v = CredView {
        id: c.id,
        name: c.name.clone(),
        provider: c.provider.clone(),
        fields: BTreeMap::new(),
        set: BTreeMap::new(),
    };
    for (k, val) in &c.fields {
        v.set.insert(k.clone(), !val.is_empty());
        if !secret.get(k.as_str()).copied().unwrap_or(false)
            || val.starts_with("env:")
            || val.starts_with("file:")
        {
            v.fields.insert(k.clone(), val.clone());
        }
    }
    v
}

pub async fn list_credentials(State(st): State<Arc<AppState>>) -> ApiResult {
    let cs = st.store.credentials()?;
    ok(&cs.iter().map(view_cred).collect::<Vec<_>>())
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CredReq {
    name: String,
    provider: String,
    fields: BTreeMap<String, String>,
}

async fn save_credential_inner(
    st: &AppState,
    p: &Principal,
    c: &Client,
    id: i64,
    req: CredReq,
) -> ApiResult {
    let prov =
        dns_provider_by_id(&req.provider).ok_or_else(|| ApiError::bad("unknown DNS provider"))?;
    let mut cred = DnsCredential {
        name: req.name.trim().to_string(),
        provider: req.provider.clone(),
        ..Default::default()
    };
    if id != 0 {
        let cur = st.store.credential(id)?;
        cred.id = id;
        if cur.provider == req.provider {
            cred.fields = cur.fields; // blank secret inputs keep the stored value
        }
    }
    if cred.name.is_empty() {
        cred.name = prov.name.to_string();
    }
    for f in prov.fields {
        let Some(v) = req.fields.get(f.key) else {
            continue;
        };
        let v = v.trim();
        if v.is_empty() && f.secret {
            continue;
        }
        if v.is_empty() {
            cred.fields.remove(f.key);
            continue;
        }
        cred.fields.insert(f.key.to_string(), v.to_string());
    }
    for f in prov.fields {
        if f.required && cred.fields.get(f.key).map(|v| v.is_empty()).unwrap_or(true) {
            return Err(ApiError::bad(format!("{} is required", f.label)));
        }
    }
    let cred = st.store.save_credential(cred)?;
    st.audit(
        Some(p),
        c,
        "dns_credential.save",
        &cred.name,
        &cred.provider,
    );
    ok(&view_cred(&cred))
}

pub async fn save_credential(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(req): JsonBody<CredReq>,
) -> ApiResult {
    save_credential_inner(&st, &p, &c, 0, req).await
}

pub async fn save_credential_id(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
    JsonBody(req): JsonBody<CredReq>,
) -> ApiResult {
    save_credential_inner(&st, &p, &c, id, req).await
}

pub async fn delete_credential(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    if let Ok(ds) = st.store.domains() {
        if let Some(d) = ds.iter().find(|d| d.credential_id == Some(id)) {
            return Err(ApiError::conflict(format!(
                "domain {} still uses this account; change it first",
                d.name
            )));
        }
    }
    if st.store.settings().ddns.credential_id == Some(id) {
        return Err(ApiError::conflict(
            "Dynamic DNS still uses this account; change it first",
        ));
    }
    st.store.delete_credential(id)?;
    st.audit(Some(&p), &c, "dns_credential.delete", &id.to_string(), "");
    ok_true()
}

// ---------------------------------------------------------------- snippets & raw blocks

pub async fn list_snippets(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.store.snippets()?)
}

async fn save_snippet_inner(
    st: &AppState,
    p: &Principal,
    c: &Client,
    id: i64,
    mut sn: Snippet,
) -> ApiResult {
    sn.id = id;
    sn.name = sn.name.trim().to_string();
    if sn.name.is_empty() || sn.name.contains([' ', '\t', '(', ')', '{', '}']) {
        return Err(ApiError::bad(
            "snippet name is required and can't contain spaces or brackets",
        ));
    }
    let sn = st.store.save_snippet(sn)?;
    st.audit(Some(p), c, "snippet.save", &sn.name, "");
    ok(&sn)
}

pub async fn save_snippet(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(sn): JsonBody<Snippet>,
) -> ApiResult {
    save_snippet_inner(&st, &p, &c, 0, sn).await
}

pub async fn save_snippet_id(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
    JsonBody(sn): JsonBody<Snippet>,
) -> ApiResult {
    save_snippet_inner(&st, &p, &c, id, sn).await
}

pub async fn delete_snippet(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    st.store.delete_snippet(id)?;
    st.audit(Some(&p), &c, "snippet.delete", &id.to_string(), "");
    ok_true()
}

pub async fn list_raw_blocks(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.store.raw_blocks()?)
}

async fn save_raw_block_inner(
    st: &AppState,
    p: &Principal,
    c: &Client,
    id: i64,
    mut b: RawBlock,
) -> ApiResult {
    b.id = id;
    if b.body.trim().is_empty() {
        return Err(ApiError::bad("block is empty"));
    }
    if b.source.is_empty() {
        b.source = "user".into();
    }
    let b = st.store.save_raw_block(b)?;
    st.audit(Some(p), c, "raw_block.save", &b.name, "");
    ok(&b)
}

pub async fn save_raw_block(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(b): JsonBody<RawBlock>,
) -> ApiResult {
    save_raw_block_inner(&st, &p, &c, 0, b).await
}

pub async fn save_raw_block_id(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
    JsonBody(b): JsonBody<RawBlock>,
) -> ApiResult {
    save_raw_block_inner(&st, &p, &c, id, b).await
}

pub async fn delete_raw_block(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    st.store.delete_raw_block(id)?;
    st.audit(Some(&p), &c, "raw_block.delete", &id.to_string(), "");
    ok_true()
}

// ---------------------------------------------------------------- settings

#[derive(Serialize)]
struct SettingsView {
    #[serde(flatten)]
    settings: Settings,
    acme_eab_mac_set: bool,
    crowdsec_api_key_set: bool,
}

fn settings_view(st: &AppState) -> SettingsView {
    SettingsView {
        settings: st.store.settings(),
        acme_eab_mac_set: st.store.secret("acme_eab_mac").is_some(),
        crowdsec_api_key_set: st.store.secret("crowdsec_api_key").is_some(),
    }
}

pub async fn get_settings(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&settings_view(&st))
}

/// Checks and tidies settings before they are saved (`PUT /settings` and the Settings pages): the reason when
/// they can't be saved.
pub fn check_settings(st: &AppState, s: &mut Settings) -> Result<(), String> {
    if !s.acme_ca.is_empty()
        && s.acme_ca != "custom"
        && s.acme_ca != "caddy_internal"
        && generator::ca_preset(&s.acme_ca).is_none()
    {
        return Err(String::from("unknown certificate authority"));
    }
    if !s.resolver_preset.is_empty() && s.resolver_preset != "custom" {
        let Some(r) = generator::resolver_preset(&s.resolver_preset) else {
            return Err(String::from("unknown resolver preset"));
        };
        s.resolvers = r.iter().map(|x| x.to_string()).collect();
    }
    if s.session_timeout_minutes <= 0 {
        s.session_timeout_minutes = 60;
    }
    s.cert_alert_days.sort();
    crate::shields::validate_security(&mut s.security).map_err(|e| format!("Security: {e}"))?;
    if s.ddns.enabled {
        if let Some(cid) = s.ddns.credential_id {
            if st.store.credential(cid).is_err() {
                return Err(String::from(
                    "Dynamic DNS: that DNS provider account doesn't exist",
                ));
            }
        }
        for z in &mut s.ddns.domains {
            z.zone = z.zone.trim().trim_end_matches('.').to_ascii_lowercase();
            if !z.zone.is_empty() && !valid_host(&z.zone) {
                return Err(format!("Dynamic DNS: {:?} is not a valid zone", z.zone));
            }
            z.names = z
                .names
                .iter()
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect();
        }
        s.ddns.domains.retain(|z| !z.zone.is_empty());
        for src in &s.ddns.ip_sources {
            if !matches!(
                src.kind.as_str(),
                "upnp" | "simple_http" | "interface" | "static"
            ) {
                return Err(format!("Dynamic DNS: unknown IP source {:?}", src.kind));
            }
        }
    }
    {
        let u = &mut s.updates;
        if u.check_every_hours <= 0 {
            u.check_every_hours = 6;
        }
        u.check_every_hours = u.interval_hours();
        u.window_start = u.window_start.trim().to_string();
        u.window_end = u.window_end.trim().to_string();
        if u.window_start.is_empty() != u.window_end.is_empty() {
            return Err(String::from(
                "Updates: set both the start and the end of the maintenance window, or neither",
            ));
        }
        if !u.window_start.is_empty()
            && (updates::parse_hhmm(&u.window_start).is_none()
                || updates::parse_hhmm(&u.window_end).is_none())
        {
            return Err(String::from(
                "Updates: window times must be HH:MM (24-hour clock)",
            ));
        }
        if u.auto {
            u.check = true; // installing needs checking
        }
    }
    Ok(())
}

pub async fn put_settings(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(body): JsonBody<serde_json::Value>,
) -> ApiResult {
    let eab = body
        .get("acme_eab_mac")
        .and_then(|v| v.as_str())
        .map(String::from);
    let cs_key = body
        .get("crowdsec_api_key")
        .and_then(|v| v.as_str())
        .map(String::from);
    let mut s: Settings = serde_json::from_value(body)
        .map_err(|e| ApiError::bad(format!("invalid settings: {e}")))?;
    check_settings(&st, &mut s).map_err(ApiError::bad)?;
    st.store.update_settings(|cur| {
        let setup = cur.setup_complete;
        *cur = s;
        cur.setup_complete = setup || cur.setup_complete;
    })?;
    if let Some(v) = eab {
        st.store.set_secret("acme_eab_mac", &v)?;
    }
    if let Some(v) = cs_key {
        st.store.set_secret("crowdsec_api_key", &v)?;
    }
    st.audit(Some(&p), &c, "settings.update", "", "");
    ok(&settings_view(&st))
}
