//! The Security page: protections for all sites and per site, the sign-in
//! wall, the crawler list and the built-in CrowdSec.

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use super::{ApiError, ApiResult, AppState, Client, Id, JsonBody, OptJson, Principal, ok};
use crate::model::{BasicAuthUser, SecuritySettings, ShieldOverrides};
use crate::shields;

fn bad(e: impl std::fmt::Display) -> ApiError {
    ApiError::bad(e.to_string())
}

/// Overrides as the page shows them: the legacy per-site CrowdSec switch
/// counts as an explicit "on".
fn overrides_view(a: &crate::model::AccessConfig) -> ShieldOverrides {
    let mut o = a.shields.clone();
    if a.crowdsec && o.crowdsec.is_none() {
        o.crowdsec = Some(true);
    }
    o
}

/// Who visited recently, from the tail of each site's access log: whether
/// real visitor addresses reach Caddy (needed for home-only access and
/// per-visitor rate limits).
fn visitors(log_dir: &str) -> Value {
    let mut public = 0u64;
    let mut home = 0u64;
    let mut docker_like = 0u64;
    let Ok(rd) = std::fs::read_dir(log_dir) else {
        return json!({"available": false});
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !(name.starts_with("access-") && name.ends_with(".log")) {
            continue;
        }
        let Ok(mut f) = std::fs::File::open(e.path()) else {
            continue;
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = f.seek(SeekFrom::Start(len.saturating_sub(128 * 1024)));
        let mut buf = String::new();
        let _ = f.take(128 * 1024).read_to_string(&mut buf);
        for line in buf.lines().rev().take(500) {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let ip = v["request"]["client_ip"]
                .as_str()
                .or_else(|| v["request"]["remote_ip"].as_str())
                .unwrap_or("");
            let Ok(addr) = ip.parse::<std::net::IpAddr>() else {
                continue;
            };
            let private = match addr {
                std::net::IpAddr::V4(a) => a.is_private() || a.is_loopback() || a.is_link_local(),
                std::net::IpAddr::V6(a) => a.is_loopback() || (a.segments()[0] & 0xfe00) == 0xfc00,
            };
            if private {
                home += 1;
                if let std::net::IpAddr::V4(a) = addr {
                    let o = a.octets();
                    if o[0] == 172 && (16..=31).contains(&o[1]) && o[3] == 1 {
                        docker_like += 1;
                    }
                }
            } else {
                public += 1;
            }
        }
    }
    json!({"available": true, "public": public, "home": home, "docker_gateway": docker_like})
}

/// Everything the Security page shows.
pub async fn get(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&page_data(&st).await?)
}

async fn page_data(st: &AppState) -> Result<Value, ApiError> {
    let s = st.store.settings();
    let sec = s.security.clone();
    let caps = st.caps();
    let sites = st.store.sites()?;
    let rows: Vec<Value> = sites
        .iter()
        .map(|x| {
            let a = &x.config.access;
            json!({
                "id": x.id,
                "name": x.name,
                "hosts": x.hosts,
                "enabled": x.enabled,
                "kind": x.kind,
                "overrides": overrides_view(a),
                "effective": shields::effective(&sec, a),
                "own": {
                    "basic_auth": a.basic_auth.len(),
                    "forward_auth": a.forward_auth.as_ref().is_some_and(|f| !f.upstream.is_empty()),
                    "allow_cidrs": a.allow_cidrs.len(),
                    "deny_cidrs": a.deny_cidrs.len(),
                    "rate_limit": a.rate_limit.as_ref().is_some_and(|r| r.events > 0),
                },
            })
        })
        .collect();
    let (ai, fetched) = shields::ai_bots(&st.store);
    let users: Vec<Value> = st
        .store
        .users()?
        .iter()
        .map(|u| json!({"username": u.username, "role": u.role, "totp_enabled": u.totp_enabled, "disabled": u.disabled}))
        .collect();
    let mut sec_view = serde_json::to_value(&sec).unwrap_or_default();
    // Hashes stay on the server; the page only needs the names.
    sec_view["basic_auth_users"] = json!(
        sec.basic_auth_users
            .iter()
            .map(|u| json!({"username": u.username}))
            .collect::<Vec<_>>()
    );
    Ok(json!({
        "security": sec_view,
        "sites": rows,
        "app_presets": shields::APP_PRESETS,
        "bots": {"ai_count": ai.len(), "fetched_at": fetched, "other_count": shields::OTHER_CRAWLERS.len()},
        "home_networks": shields::home_networks(&sec),
        "sso": {
            "cookie_domain": sec.sso.effective_cookie_domain(),
            "sessions": st.store.sso_sessions().map(|v| v.len()).unwrap_or(0),
        },
        "plugins": {"crowdsec": caps.has("crowdsec"), "rate_limit": caps.has("rate_limit")},
        "crowdsec": st.crowdsec.status().await,
        "crowdsec_external": {
            "enabled": s.crowdsec.enabled,
            "api_url": s.crowdsec.api_url,
            "key_set": st.store.secret("crowdsec_api_key").is_some(),
        },
        "users": users,
        "visitors": visitors(&st.log_dir),
        "pending_changes": st.pipeline.pending(),
        "suggestions": crate::suggestions::list(&st.store),
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SuggestionIn {
    /// Apply to this site only; none = to every site.
    site_id: Option<i64>,
}

/// Applies a suggested protection to every site or to one site.
pub async fn apply_suggestion(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Path(id): Path<String>,
    OptJson(body): OptJson<SuggestionIn>,
) -> ApiResult {
    crate::suggestions::apply(&st.store, &id, body.site_id).map_err(bad)?;
    let target = body
        .site_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| "all sites".into());
    st.audit(Some(&p), &c, "security.suggestion", &id, &target);
    ok(&page_data(&st).await?)
}

/// Hides a suggestion.
pub async fn dismiss_suggestion(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Path(id): Path<String>,
) -> ApiResult {
    crate::suggestions::dismiss(&st.store, &id).map_err(bad)?;
    st.audit(Some(&p), &c, "security.suggestion_dismissed", &id, "");
    ok(&page_data(&st).await?)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UserIn {
    username: String,
    password: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ExternalIn {
    enabled: bool,
    api_url: String,
    api_key: String,
}

/// Saves the Security page settings. Login accounts come with a password
/// when it is new or changed; otherwise the stored hash is kept.
pub async fn put(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(mut body): JsonBody<Value>,
) -> ApiResult {
    let users_in: Vec<UserIn> = serde_json::from_value(
        body.get_mut("security")
            .and_then(|s| s.as_object_mut())
            .and_then(|s| s.remove("basic_auth_users"))
            .filter(|v| !v.is_null())
            .unwrap_or(json!([])),
    )
    .map_err(|e| bad(format!("invalid login accounts: {e}")))?;
    let external: Option<ExternalIn> = body
        .get("crowdsec_external")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());
    let mut sec: SecuritySettings =
        serde_json::from_value(body.get("security").cloned().unwrap_or(json!({})))
            .map_err(|e| bad(format!("invalid security settings: {e}")))?;
    let old = st.store.settings().security.basic_auth_users;
    let mut users = Vec::new();
    for u in users_in {
        let name = u.username.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let hash = if !u.password.is_empty() {
            if u.password.len() < 8 {
                return Err(bad(format!(
                    "the password for {name} must have at least 8 characters"
                )));
            }
            let pw = u.password.clone();
            tokio::task::spawn_blocking(move || crate::auth::bcrypt_hash(&pw))
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?
                .map_err(|e| ApiError::internal(e.to_string()))?
        } else {
            old.iter()
                .find(|o| o.username.eq_ignore_ascii_case(&name))
                .map(|o| o.hash.clone())
                .ok_or_else(|| bad(format!("set a password for {name}")))?
        };
        users.push(BasicAuthUser {
            username: name,
            hash,
        });
    }
    sec.basic_auth_users = users;
    shields::validate_security(&mut sec).map_err(bad)?;
    if let Some(x) = &external {
        if x.enabled && !x.api_url.trim().starts_with("http") {
            return Err(bad("the CrowdSec URL must start with http:// or https://"));
        }
    }
    st.store.update_settings(|cur| {
        cur.security = sec;
        if let Some(x) = &external {
            cur.crowdsec.enabled = x.enabled;
            if !x.api_url.trim().is_empty() {
                cur.crowdsec.api_url = x.api_url.trim().to_string();
            }
        }
    })?;
    if let Some(x) = &external {
        if !x.api_key.is_empty() {
            st.store.set_secret("crowdsec_api_key", &x.api_key)?;
        }
    }
    st.audit(Some(&p), &c, "security.update", "", "");
    ok(&page_data(&st).await?)
}

/// Sets one site's overrides.
pub async fn put_site(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Id(id): Id,
    JsonBody(mut o): JsonBody<ShieldOverrides>,
) -> ApiResult {
    shields::validate_overrides(&mut o).map_err(bad)?;
    let mut site = st.store.site(id)?;
    site.config.access.shields = o;
    site.config.access.crowdsec = false; // superseded by the override
    site.updated_at = chrono::Utc::now();
    st.store.update_site(&site)?;
    st.audit(Some(&p), &c, "security.site", &site.name, "");
    ok(&page_data(&st).await?)
}

pub async fn refresh_bots(State(st): State<Arc<AppState>>) -> ApiResult {
    let changed = shields::refresh_ai_list(&st.store).await.map_err(|e| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            format!("could not fetch the list: {e:#}"),
        )
    })?;
    let (names, fetched) = shields::ai_bots(&st.store);
    ok(
        &json!({"changed": changed, "ai_count": names.len(), "fetched_at": fetched, "pending_changes": st.pipeline.pending()}),
    )
}

pub async fn sessions(State(st): State<Arc<AppState>>) -> ApiResult {
    let v: Vec<Value> = st
        .store
        .sso_sessions()?
        .into_iter()
        .map(|(s, user)| {
            json!({
                "id": s.id.chars().take(16).collect::<String>(),
                "username": user,
                "created_at": s.created_at,
                "last_seen": s.last_seen,
                "expires_at": s.expires_at,
                "ip": s.ip,
                "user_agent": s.user_agent,
                "mfa": s.mfa,
            })
        })
        .collect();
    ok(&v)
}

/// Ends one sign-in (by the id prefix the list shows) or, with "all", every
/// sign-in.
pub async fn end_session(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    Path(id): Path<String>,
) -> ApiResult {
    if id == "all" {
        let n = st.store.delete_all_sso_sessions();
        st.audit(Some(&p), &c, "sso.sessions_ended", "all", &n.to_string());
        return ok(&json!({"ended": n}));
    }
    let found = st
        .store
        .sso_sessions()?
        .into_iter()
        .find(|(s, _)| s.id.starts_with(&id) && id.len() >= 8);
    let Some((s, user)) = found else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such sign-in".into()));
    };
    st.store.delete_sso_session(&s.id);
    st.audit(Some(&p), &c, "sso.session_ended", &user, "");
    ok(&json!({"ended": 1}))
}

// ---------------------------------------------------------------- CrowdSec

pub async fn crowdsec_status(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.crowdsec.status().await)
}

pub async fn crowdsec_enable(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
) -> ApiResult {
    let was_pending = st.pipeline.pending();
    st.crowdsec.start_enable().map_err(bad)?;
    st.audit(Some(&p), &c, "crowdsec.enable", "", "");
    // Once it is set up, put Caddy's bouncer in place (if nothing else is
    // waiting for review).
    let s2 = st.clone();
    tokio::spawn(async move {
        for _ in 0..600 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            match s2.crowdsec.current_job() {
                Some(j) if j.status == "running" => continue,
                Some(j) if j.status == "done" => {
                    s2.pipeline
                        .apply_if_clean(was_pending, "CrowdSec turned on")
                        .await;
                    return;
                }
                _ => return,
            }
        }
    });
    Ok(super::json(
        StatusCode::ACCEPTED,
        &st.crowdsec.status().await,
    ))
}

pub async fn crowdsec_disable(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
) -> ApiResult {
    let was_pending = st.pipeline.pending();
    st.crowdsec.disable().map_err(bad)?;
    st.audit(Some(&p), &c, "crowdsec.disable", "", "");
    st.pipeline
        .apply_if_clean(was_pending, "CrowdSec turned off")
        .await;
    ok(&st.crowdsec.status().await)
}

pub async fn crowdsec_update(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
) -> ApiResult {
    st.crowdsec
        .start_update(p.user.username.clone())
        .map_err(bad)?;
    Ok(super::json(
        StatusCode::ACCEPTED,
        &st.crowdsec.status().await,
    ))
}

pub async fn crowdsec_rules(State(st): State<Arc<AppState>>) -> ApiResult {
    st.crowdsec.start_hub_update().map_err(bad)?;
    Ok(super::json(
        StatusCode::ACCEPTED,
        &st.crowdsec.status().await,
    ))
}

pub async fn crowdsec_decisions(State(st): State<Arc<AppState>>) -> ApiResult {
    let d = st
        .crowdsec
        .decisions()
        .await
        .map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")))?;
    ok(&d)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct BanReq {
    ip: String,
    duration: String,
}

pub async fn crowdsec_ban(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    OptJson(req): OptJson<BanReq>,
) -> ApiResult {
    st.crowdsec
        .ban(req.ip.trim(), &req.duration, &p.user.username)
        .await
        .map_err(bad)?;
    crowdsec_decisions(State(st)).await
}

pub async fn crowdsec_unban(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    Id(id): Id,
) -> ApiResult {
    st.crowdsec.unban(id, &p.user.username).await.map_err(bad)?;
    crowdsec_decisions(State(st)).await
}
