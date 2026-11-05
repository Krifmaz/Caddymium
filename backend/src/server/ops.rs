use axum::extract::{Extension, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use super::{
    ApiError, ApiResult, AppState, Client, Id, JsonBody, OptJson, Principal, json, ok, ok_true,
};
use crate::apply;
use crate::auth;
use crate::diff;
use crate::generator;
use crate::notify;
use crate::registry::{self, DNS_PROVIDERS};
use crate::store::{ApiKey, Channel, User};

// ---------------------------------------------------------------- meta & status

pub async fn meta(State(st): State<Arc<AppState>>) -> ApiResult {
    let caps = st.caps();
    #[derive(Serialize)]
    struct Provider {
        #[serde(flatten)]
        p: &'static registry::DnsProvider,
        available: bool,
    }
    let provs: Vec<Provider> = DNS_PROVIDERS
        .iter()
        .map(|p| Provider {
            p,
            available: caps.has_module(&p.module()),
        })
        .collect();
    let cas: Vec<&str> = generator::CA_PRESETS.iter().map(|(k, _)| *k).collect();
    let resolvers: BTreeMap<&str, &[&str]> = generator::RESOLVER_PRESETS.iter().cloned().collect();
    ok(&serde_json::json!({
        "dns_providers": provs,
        "features": caps.all_feature_status(),
        "ca_presets": cas,
        "resolver_presets": resolvers,
        "notification_types": notify::types_map(),
        "notification_events": notify::ALL_EVENTS,
        "site_kinds": crate::model::ALL_KINDS,
    }))
}

pub async fn status(State(st): State<Arc<AppState>>) -> ApiResult {
    let caps = st.caps();
    let mut res = serde_json::Map::new();
    res.insert("panel_version".into(), st.version.clone().into());
    res.insert("caddy_version".into(), caps.version_s.clone().into());
    res.insert("caddy_detected".into(), caps.detected.into());
    res.insert("caddy_binary".into(), st.runner.available().into());
    res.insert("schema".into(), st.store.schema_version().into());
    let ping = tokio::time::timeout(Duration::from_secs(3), st.admin.ping()).await;
    match ping {
        Ok(Ok(())) => {
            res.insert("caddy_reachable".into(), true.into());
        }
        Ok(Err(e)) => {
            res.insert("caddy_reachable".into(), false.into());
            res.insert("caddy_error".into(), e.to_string().into());
        }
        Err(_) => {
            res.insert("caddy_reachable".into(), false.into());
            res.insert("caddy_error".into(), "timeout".into());
        }
    }
    if let Ok(mut v) = st.store.last_version() {
        v.caddyfile = String::new();
        res.insert(
            "last_apply".into(),
            serde_json::to_value(v).unwrap_or_default(),
        );
    }
    // Pending = the model differs from what was last applied successfully.
    if let Ok((out, _)) = st.pipeline.generate() {
        let pending = match st.store.last_good() {
            Ok(lg) => lg.caddyfile != out.caddyfile,
            Err(_) => true,
        };
        res.insert("pending_changes".into(), pending.into());
    }
    let sites = st.store.sites().unwrap_or_default();
    res.insert("sites_total".into(), sites.len().into());
    res.insert(
        "sites_enabled".into(),
        sites.iter().filter(|s| s.enabled).count().into(),
    );
    if let Some(j) = st.plugins.current_job() {
        if j.status == "running" {
            res.insert(
                "plugin_job".into(),
                serde_json::json!({"kind": j.kind, "started": j.started}),
            );
        }
    }
    if let Some(w) = crate::watchdog::Watchdog::last(&st.store) {
        res.insert(
            "watchdog".into(),
            serde_json::to_value(w).unwrap_or_default(),
        );
    }
    // From the background checker: no network call here.
    let u = st.updates.state();
    res.insert("update_available".into(), u.update_available.into());
    res.insert("latest_caddy".into(), u.latest.into());
    ok(&res)
}

pub async fn upstreams(State(st): State<Arc<AppState>>) -> ApiResult {
    match tokio::time::timeout(Duration::from_secs(5), st.admin.upstreams()).await {
        Ok(Ok(u)) => ok(&u),
        Ok(Err(e)) => Err(ApiError(
            StatusCode::BAD_GATEWAY,
            format!("can't reach Caddy: {e}"),
        )),
        Err(_) => Err(ApiError(
            StatusCode::BAD_GATEWAY,
            "can't reach Caddy: timeout".into(),
        )),
    }
}

// ---------------------------------------------------------------- config

pub async fn preview(State(st): State<Arc<AppState>>) -> ApiResult {
    let (out, _) = st
        .pipeline
        .generate()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    ok(&out)
}

pub async fn plan(State(st): State<Arc<AppState>>) -> ApiResult {
    let plan = st
        .pipeline
        .plan()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    ok(&plan)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ApplyReq {
    dry_run: bool,
    message: String,
}

pub async fn apply(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    OptJson(req): OptJson<ApplyReq>,
) -> ApiResult {
    let dry = req.dry_run || st.store.settings().dry_run;
    let res = st
        .pipeline
        .apply(apply::Options {
            dry_run: dry,
            user: p.user.username.clone(),
            message: req.message,
        })
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !dry {
        st.audit(Some(&p), &c, "config.apply", &res.status, &res.message);
    }
    ok(&res)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ValidateReq {
    text: String,
    /// "site" (inside a site block) or "top"
    context: String,
}

/// Checks an arbitrary snippet of Caddyfile text (raw site snippets, raw
/// blocks) by wrapping it the way it will be used.
pub async fn validate_text(
    State(st): State<Arc<AppState>>,
    JsonBody(req): JsonBody<ValidateReq>,
) -> ApiResult {
    if !st.runner.available() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "caddy binary not available for validation".into(),
        ));
    }
    let text = if req.context == "site" {
        format!(
            "{{\n\tadmin off\n}}\n\nhttp://validate.invalid {{\n{}\n}}\n",
            req.text
        )
    } else {
        format!("{{\n\tadmin off\n}}\n\n{}\n", req.text)
    };
    match st.runner.adapt(&text).await {
        Ok(_) => ok(&serde_json::json!({"valid": true})),
        Err(e) => ok(&serde_json::json!({"valid": false, "error": e.to_string()})),
    }
}

pub async fn export(
    State(st): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let text = if q.get("source").map(|s| s.as_str()) == Some("applied") {
        st.store
            .last_good()
            .map(|v| v.caddyfile)
            .map_err(|_| ApiError(StatusCode::NOT_FOUND, "nothing has been applied yet".into()))?
    } else {
        st.pipeline
            .generate()
            .map_err(|e| ApiError::internal(e.to_string()))?
            .0
            .caddyfile
    };
    let mut r = (StatusCode::OK, text).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    r.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"Caddyfile\""),
    );
    Ok(r)
}

fn qint(q: &HashMap<String, String>, k: &str) -> i64 {
    q.get(k).and_then(|v| v.parse().ok()).unwrap_or(0)
}

pub async fn list_versions(
    State(st): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let mut limit = qint(&q, "limit");
    if limit <= 0 || limit > 200 {
        limit = 50;
    }
    let mut vs = st.store.versions(limit)?;
    for v in &mut vs {
        v.caddyfile = String::new();
    }
    ok(&vs)
}

pub async fn get_version(State(st): State<Arc<AppState>>, Id(id): Id) -> ApiResult {
    ok(&st.store.version(id)?)
}

/// Compares two versions; b=0 (or missing) means the pending config.
pub async fn diff_versions(
    State(st): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let text = |id: i64| -> Result<(String, String), ApiError> {
        if id == 0 {
            let (out, _) = st
                .pipeline
                .generate()
                .map_err(|e| ApiError::internal(e.to_string()))?;
            return Ok((out.caddyfile, "pending".into()));
        }
        let v = st.store.version(id)?;
        Ok((v.caddyfile, format!("version {id}")))
    };
    let (ta, na) = text(qint(&q, "a"))?;
    let (tb, nb) = text(qint(&q, "b"))?;
    let (ins, del) = diff::stats(&diff::lines(&ta, &tb));
    ok(
        &serde_json::json!({"diff": diff::unified(&ta, &tb, &na, &nb, 3), "added": ins, "removed": del}),
    )
}

pub async fn rollback_version(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    let res = st
        .pipeline
        .apply_snapshot(
            id,
            apply::Options {
                user: p.user.username.clone(),
                ..Default::default()
            },
        )
        .await?;
    st.audit(
        Some(&p),
        &c,
        "config.rollback",
        &id.to_string(),
        &format!("{}: {}", res.status, res.message),
    );
    ok(&res)
}

// ---------------------------------------------------------------- users & keys

pub async fn list_users(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.store.users()?)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct UserReq {
    username: String,
    password: String,
    role: String,
    disabled: bool,
    reset_2fa: bool,
}

async fn save_user_inner(
    st: &AppState,
    p: &Principal,
    c: &Client,
    id: i64,
    req: UserReq,
) -> ApiResult {
    if !auth::valid_role(&req.role) {
        return Err(ApiError::bad("role must be admin, editor or viewer"));
    }
    let mut u = User::default();
    if id != 0 {
        u = st.store.user_by_id(id)?;
        let demote = u.role == auth::ROLE_ADMIN && (req.role != auth::ROLE_ADMIN || req.disabled);
        if demote && !u.disabled && st.store.admin_count() <= 1 {
            return Err(ApiError::conflict("you can't remove the last admin"));
        }
    } else if req.password.is_empty() {
        return Err(ApiError::bad("set a password for the new user"));
    }
    u.username = req.username.trim().to_string();
    if u.username.is_empty() {
        return Err(ApiError::bad("username is required"));
    }
    u.role = req.role.clone();
    u.disabled = req.disabled;
    if req.reset_2fa {
        u.totp_enabled = false;
        u.totp_secret = String::new();
    }
    if !req.password.is_empty() {
        auth::password_policy(&req.password).map_err(ApiError::bad)?;
        u.password_hash =
            auth::hash_password(&req.password).map_err(|e| ApiError::internal(e.to_string()))?;
    }
    let u = st.store.save_user(u)?;
    if u.disabled || !req.password.is_empty() {
        st.store.delete_user_sessions(u.id, "");
    }
    st.audit(Some(p), c, "user.save", &u.username, &u.role);
    ok(&u)
}

pub async fn save_user(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(req): JsonBody<UserReq>,
) -> ApiResult {
    save_user_inner(&st, &p, &c, 0, req).await
}

pub async fn save_user_id(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
    JsonBody(req): JsonBody<UserReq>,
) -> ApiResult {
    save_user_inner(&st, &p, &c, id, req).await
}

pub async fn delete_user(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    let u = st.store.user_by_id(id)?;
    if id == p.user.id {
        return Err(ApiError::conflict("you can't delete yourself"));
    }
    if u.role == auth::ROLE_ADMIN && !u.disabled && st.store.admin_count() <= 1 {
        return Err(ApiError::conflict("you can't delete the last admin"));
    }
    st.store.delete_user_sessions(id, "");
    st.store.delete_user(id)?;
    st.audit(Some(&p), &c, "user.delete", &u.username, "");
    ok_true()
}

pub async fn list_keys(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.store.api_keys()?)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct KeyReq {
    name: String,
    role: String,
    valid_days: i64,
}

pub async fn create_key(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(req): JsonBody<KeyReq>,
) -> ApiResult {
    if !auth::valid_role(&req.role) {
        return Err(ApiError::bad("role must be admin, editor or viewer"));
    }
    let tok = format!("chk_{}", auth::random_token(32));
    let mut k = ApiKey {
        name: req.name.trim().to_string(),
        prefix: tok[..12].to_string(),
        hash: auth::token_hash(&tok),
        role: req.role.clone(),
        user_id: p.user.id,
        ..Default::default()
    };
    if req.valid_days > 0 {
        k.expires_at = Some(Utc::now() + chrono::Duration::days(req.valid_days));
    }
    let k = st.store.create_api_key(k)?;
    st.audit(Some(&p), &c, "api_key.create", &k.name, &k.role);
    Ok(json(
        StatusCode::CREATED,
        &serde_json::json!({"key": k, "token": tok}),
    ))
}

pub async fn delete_key(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
) -> ApiResult {
    st.store.delete_api_key(id)?;
    st.audit(Some(&p), &c, "api_key.delete", &id.to_string(), "");
    ok_true()
}

// ---------------------------------------------------------------- notifications

const MASKED: &str = "••••••";

fn mask_channel(mut c: Channel) -> Channel {
    for (k, v) in c.config.iter_mut() {
        if notify::is_secret_field(k) && !v.is_empty() {
            *v = MASKED.into();
        }
    }
    c
}

pub async fn list_channels(State(st): State<Arc<AppState>>) -> ApiResult {
    let cs = st.store.channels()?;
    ok(&cs.into_iter().map(mask_channel).collect::<Vec<_>>())
}

/// Merges masked secret fields with stored values.
fn channel_from_request(st: &AppState, id: i64, mut c: Channel) -> Result<Channel, ApiError> {
    if !notify::valid_type(&c.r#type) {
        return Err(ApiError::bad("unknown notification type"));
    }
    if id != 0 {
        c.id = id;
    }
    if c.id != 0 {
        if let Ok(cs) = st.store.channels() {
            if let Some(old) = cs.into_iter().find(|o| o.id == c.id) {
                for (k, v) in c.config.iter_mut() {
                    if v == MASKED {
                        *v = old.config.get(k).cloned().unwrap_or_default();
                    }
                }
            }
        }
    }
    Ok(c)
}

async fn save_channel_inner(
    st: &AppState,
    p: &Principal,
    cl: &Client,
    id: i64,
    c: Channel,
) -> ApiResult {
    let c = channel_from_request(st, id, c)?;
    let c = st.store.save_channel(c)?;
    st.audit(Some(p), cl, "notification.save", &c.name, &c.r#type);
    ok(&mask_channel(c))
}

pub async fn save_channel(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    cl: Client,
    JsonBody(c): JsonBody<Channel>,
) -> ApiResult {
    save_channel_inner(&st, &p, &cl, 0, c).await
}

pub async fn save_channel_id(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    cl: Client,
    Id(id): Id,
    JsonBody(c): JsonBody<Channel>,
) -> ApiResult {
    save_channel_inner(&st, &p, &cl, id, c).await
}

pub async fn delete_channel(State(st): State<Arc<AppState>>, Id(id): Id) -> ApiResult {
    st.store.delete_channel(id)?;
    ok_true()
}

pub async fn test_channel(
    State(st): State<Arc<AppState>>,
    JsonBody(c): JsonBody<Channel>,
) -> ApiResult {
    let c = channel_from_request(&st, 0, c)?;
    match tokio::time::timeout(Duration::from_secs(20), st.notifier.test(&c)).await {
        Ok(Ok(())) => ok_true(),
        Ok(Err(e)) => Err(ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}"))),
        Err(_) => Err(ApiError(StatusCode::BAD_GATEWAY, "timed out".into())),
    }
}

pub async fn list_audit(
    State(st): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let mut limit = qint(&q, "limit");
    if limit <= 0 || limit > 500 {
        limit = 100;
    }
    let offset = qint(&q, "offset");
    ok(&st
        .store
        .audit_log(q.get("q").map(|s| s.as_str()).unwrap_or(""), limit, offset)?)
}

// ---------------------------------------------------------------- discard / backup

/// Puts the editable configuration back to what was last applied, from the
/// snapshot recorded with that version.
pub async fn discard(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
) -> ApiResult {
    let lg = st.store.last_good().map_err(|_| {
        ApiError(
            StatusCode::CONFLICT,
            "nothing has been applied yet, so there is nothing to go back to".into(),
        )
    })?;
    if lg.model.is_empty() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            format!(
                "version {} was applied before snapshots were recorded; after the next apply, Discard works from that version on",
                lg.id
            ),
        ));
    }
    let sum = crate::backup::restore_json(&st.store, &lg.model)
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    st.audit(
        Some(&p),
        &c,
        "config.discard",
        &lg.id.to_string(),
        "unapplied changes discarded",
    );
    ok(&serde_json::json!({"version_id": lg.id, "restored": sum}))
}

/// A backup archive of the editable configuration, sealed with the secret key.
pub async fn backup_download(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
) -> ApiResult {
    let caddy = st.caps().version_s;
    let env = crate::backup::seal(&st.store, &st.version, &caddy)
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    st.audit(Some(&p), &c, "backup.download", "", "");
    let name = format!(
        "caddymium-backup-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let mut r = super::json(StatusCode::OK, &env);
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(r)
}

/// Restores a backup archive: replaces sites, domains, DNS accounts,
/// snippets, raw blocks, templates, notification channels, named secrets and
/// settings. Nothing is applied to Caddy; review and apply afterwards.
pub async fn backup_restore(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(env): JsonBody<crate::backup::Envelope>,
) -> ApiResult {
    let snap = crate::backup::open(&st.store, &env).map_err(|e| ApiError::bad(format!("{e:#}")))?;
    let sum =
        crate::backup::restore(&st.store, &snap).map_err(|e| ApiError::bad(format!("{e:#}")))?;
    st.audit(
        Some(&p),
        &c,
        "backup.restore",
        &env.created_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
        &format!("{} tables", sum.tables.len()),
    );
    ok(&serde_json::json!({
        "restored": sum,
        "created_at": env.created_at,
        "panel_version": env.panel_version,
    }))
}
