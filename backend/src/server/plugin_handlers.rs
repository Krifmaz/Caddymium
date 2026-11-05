use axum::extract::{Extension, State};
use axum::http::StatusCode;
use serde::Deserialize;
use std::sync::Arc;

use super::{ApiError, ApiResult, AppState, Client, OptJson, Principal, ok};
use crate::ddns;

pub async fn status(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.plugins.status().await)
}

pub async fn refresh_catalog(State(st): State<Arc<AppState>>) -> ApiResult {
    let c = st.plugins.catalog(true).await;
    ok(&c)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct InstallReq {
    add: Vec<String>,
    remove: Vec<String>,
}

/// Starts a background install. With no changes, Caddy is rebuilt with the
/// current plugins against the latest release (an upgrade).
pub async fn install(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    OptJson(req): OptJson<InstallReq>,
) -> ApiResult {
    let add: Vec<String> = req
        .add
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let remove: Vec<String> = req
        .remove
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let job = st
        .plugins
        .start_install(add.clone(), remove.clone(), p.user.username.clone())
        .map_err(|e| {
            let msg = format!("{e:#}");
            if msg.contains("still running") {
                ApiError::conflict(msg)
            } else {
                ApiError::bad(msg)
            }
        })?;
    st.audit(
        Some(&p),
        &c,
        "plugins.job",
        &job.kind,
        &format!("add {} remove {}", add.join(","), remove.join(",")),
    );
    Ok(super::json(StatusCode::ACCEPTED, &job))
}

pub async fn job(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.plugins.current_job())
}

pub async fn reset(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
) -> ApiResult {
    let job = st
        .plugins
        .start_reset(p.user.username.clone())
        .map_err(|e| ApiError::bad(format!("{e:#}")))?;
    Ok(super::json(StatusCode::ACCEPTED, &job))
}

pub async fn rollback(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
) -> ApiResult {
    let job = st
        .plugins
        .start_rollback(p.user.username.clone())
        .map_err(|e| ApiError::bad(format!("{e:#}")))?;
    Ok(super::json(StatusCode::ACCEPTED, &job))
}

pub async fn restart(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
) -> ApiResult {
    let job = st
        .plugins
        .start_restart(p.user.username.clone())
        .map_err(|e| ApiError::bad(format!("{e:#}")))?;
    Ok(super::json(StatusCode::ACCEPTED, &job))
}

pub async fn ddns_status(State(st): State<Arc<AppState>>) -> ApiResult {
    let d = st.store.settings().ddns;
    ok(&ddns::status(&d).await)
}

/// Update checker state and schedule (shown on the Plugins page).
pub async fn updates(State(st): State<Arc<AppState>>) -> ApiResult {
    ok(&st.updates.report())
}

/// Checks for a new Caddy release right now.
pub async fn updates_check(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
) -> ApiResult {
    let r = st.updates.check(true).await;
    st.audit(Some(&p), &c, "updates.check", &r.latest, "");
    ok(&st.updates.report())
}
