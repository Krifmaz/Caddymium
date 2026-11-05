use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::{
    ApiError, ApiResult, AppState, COOKIE_NAME, Client, JsonBody, Principal, SESSION_ABSOLUTE,
    cookie, json, ok_true, session_cookie,
};
use crate::auth;
use crate::store::{Session, User};

#[derive(Serialize)]
struct MeResponse {
    user: Option<User>,
    #[serde(skip_serializing_if = "String::is_empty")]
    csrf: String,
    pending_2fa: bool,
    needs_setup: bool,
    setup_token_required: bool,
    setup_complete: bool,
    version: String,
}

/// Reports who is logged in. It never fails with 401 so the UI can decide
/// between the setup wizard, the login form and the app.
pub async fn me(State(st): State<Arc<AppState>>, h: HeaderMap) -> ApiResult {
    let mut res = MeResponse {
        user: None,
        csrf: String::new(),
        pending_2fa: false,
        needs_setup: st.store.user_count() == 0,
        setup_token_required: !st.setup_token.is_empty(),
        setup_complete: st.store.settings().setup_complete,
        version: st.version.clone(),
    };
    if let Ok(p) = st.authenticate(&h) {
        res.user = Some(p.user);
        if let Some(s) = p.session {
            res.csrf = s.csrf;
            res.pending_2fa = s.pending_2fa;
        }
    }
    Ok(json(StatusCode::OK, &res))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SetupReq {
    username: String,
    password: String,
    token: String,
}

/// Creates the first admin account. It only works while no user exists, and
/// requires the setup token when one is configured.
pub async fn setup(
    State(st): State<Arc<AppState>>,
    c: Client,
    JsonBody(req): JsonBody<SetupReq>,
) -> ApiResult {
    let key = format!("setup:{}", c.ip);
    if st.limiter().blocked(&key) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again later".into(),
        ));
    }
    if st.store.user_count() > 0 {
        return Err(ApiError::conflict("setup has already been completed"));
    }
    if !st.setup_token.is_empty() && req.token != st.setup_token {
        st.limiter().fail(&key);
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "wrong setup token (see CADDYMIUM_SETUP_TOKEN)".into(),
        ));
    }
    let username = req.username.trim().to_string();
    if username.is_empty() {
        return Err(ApiError::bad("username is required"));
    }
    auth::password_policy(&req.password).map_err(ApiError::bad)?;
    let hash = auth::hash_password(&req.password).map_err(|e| ApiError::internal(e.to_string()))?;
    let u = st.store.save_user(User {
        username,
        password_hash: hash,
        role: auth::ROLE_ADMIN.into(),
        ..Default::default()
    })?;
    st.store
        .audit(&u.username, "setup.admin_created", &u.username, "", &c.ip);
    start_session(&st, &c, u, false).await
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LoginReq {
    username: String,
    password: String,
}

pub async fn login(
    State(st): State<Arc<AppState>>,
    c: Client,
    JsonBody(req): JsonBody<LoginReq>,
) -> ApiResult {
    let key = format!("login:{}", c.ip);
    if st.limiter().blocked(&key) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed logins; try again in a few minutes".into(),
        ));
    }
    let u = match st.store.user_by_name(req.username.trim()) {
        Ok(u) if !u.password_hash.is_empty() => u,
        _ => {
            let pw = req.password.clone();
            tokio::task::spawn_blocking(move || auth::verify_dummy(&pw))
                .await
                .ok(); // equalize timing for unknown users
            st.limiter().fail(&key);
            return Err(ApiError(
                StatusCode::UNAUTHORIZED,
                "wrong username or password".into(),
            ));
        }
    };
    let (hash, pw) = (u.password_hash.clone(), req.password.clone());
    let okay = tokio::task::spawn_blocking(move || auth::verify_password(&hash, &pw))
        .await
        .unwrap_or(false);
    if !okay {
        st.limiter().fail(&key);
        st.store
            .audit(&u.username, "login.failed", &u.username, "", &c.ip);
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "wrong username or password".into(),
        ));
    }
    if u.disabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "this account is disabled".into(),
        ));
    }
    if u.role == auth::ROLE_MEMBER {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "this account can sign in to your protected sites, not to the panel".into(),
        ));
    }
    st.limiter().reset(&key);
    let pending = u.totp_enabled;
    start_session(&st, &c, u, pending).await
}

/// Issues a session cookie. With pending_2fa the session can only be used
/// to submit a TOTP code.
async fn start_session(st: &AppState, c: &Client, u: User, pending_2fa: bool) -> ApiResult {
    let tok = auth::random_token(32);
    let now = Utc::now();
    let se = Session {
        id: auth::token_hash(&tok),
        user_id: u.id,
        csrf: auth::random_token(24),
        created_at: now,
        expires_at: now + SESSION_ABSOLUTE,
        last_seen: now,
        ip: c.ip.clone(),
        user_agent: c.user_agent.clone(),
        pending_2fa,
    };
    st.store.create_session(&se)?;
    if !pending_2fa {
        st.store.touch_login(u.id);
        st.store
            .audit(&u.username, "login", &u.username, "", &se.ip);
    }
    let res = MeResponse {
        csrf: se.csrf.clone(),
        pending_2fa,
        needs_setup: false,
        setup_token_required: !st.setup_token.is_empty(),
        setup_complete: st.store.settings().setup_complete,
        version: st.version.clone(),
        user: Some(u),
    };
    let mut r = json(StatusCode::OK, &res);
    r.headers_mut().append(
        header::SET_COOKIE,
        session_cookie(st, &tok, Some(se.expires_at)),
    );
    Ok(r)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CodeReq {
    code: String,
}

/// Completes a login that is waiting for a 2FA code.
pub async fn login_totp(
    State(st): State<Arc<AppState>>,
    c: Client,
    h: HeaderMap,
    JsonBody(req): JsonBody<CodeReq>,
) -> ApiResult {
    let cv = cookie(&h, COOKIE_NAME)
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "log in first".into()))?;
    let se = match st.store.get_session(&auth::token_hash(&cv)) {
        Ok(se) if se.pending_2fa && Utc::now() - se.created_at <= chrono::Duration::minutes(10) => {
            se
        }
        _ => {
            return Err(ApiError(
                StatusCode::UNAUTHORIZED,
                "login expired; start again".into(),
            ));
        }
    };
    let key = format!("totp:{}", se.user_id);
    if st.limiter().blocked(&key) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many wrong codes; try again later".into(),
        ));
    }
    let u = st
        .store
        .user_by_id(se.user_id)
        .map_err(|_| ApiError(StatusCode::UNAUTHORIZED, "account not found".into()))?;
    if !check_totp(&st, &u, &req.code) {
        st.limiter().fail(&key);
        return Err(ApiError(StatusCode::UNAUTHORIZED, "wrong code".into()));
    }
    st.limiter().reset(&key);
    st.store.complete_session_2fa(&se.id)?;
    st.store.touch_login(u.id);
    st.store
        .audit(&u.username, "login", &u.username, "with 2FA", &c.ip);
    let res = MeResponse {
        csrf: se.csrf.clone(),
        pending_2fa: false,
        needs_setup: false,
        setup_token_required: !st.setup_token.is_empty(),
        setup_complete: st.store.settings().setup_complete,
        version: st.version.clone(),
        user: Some(u),
    };
    Ok(json(StatusCode::OK, &res))
}

/// Verifies a code and rejects reuse of an already-used time step.
pub(crate) fn check_totp(st: &AppState, u: &User, code: &str) -> bool {
    if u.totp_secret.is_empty() {
        return false;
    }
    let Some(step) = auth::verify_totp(
        &u.totp_secret,
        &code.replace(' ', ""),
        Utc::now().timestamp(),
    ) else {
        return false;
    };
    let k = format!("totp_last_step:{}", u.id);
    let last: u64 = st.store.kv_get_json(&k).unwrap_or(0);
    if step <= last {
        return false;
    }
    let _ = st.store.kv_set_json(&k, &step);
    true
}

pub async fn logout(State(st): State<Arc<AppState>>, h: HeaderMap) -> ApiResult {
    if let Some(c) = cookie(&h, COOKIE_NAME) {
        st.store.delete_session(&auth::token_hash(&c));
    }
    let mut r = json(StatusCode::OK, &serde_json::json!({"ok": true}));
    r.headers_mut()
        .append(header::SET_COOKIE, session_cookie(&st, "", None));
    Ok(r)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PasswordReq {
    current: String,
    new: String,
}

pub async fn change_password(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(req): JsonBody<PasswordReq>,
) -> ApiResult {
    let mut u = st.store.user_by_id(p.user.id)?;
    if !auth::verify_password(&u.password_hash, &req.current) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "current password is wrong".into(),
        ));
    }
    auth::password_policy(&req.new).map_err(ApiError::bad)?;
    u.password_hash =
        auth::hash_password(&req.new).map_err(|e| ApiError::internal(e.to_string()))?;
    let u = st.store.save_user(u)?;
    let except = p.session.as_ref().map(|s| s.id.clone()).unwrap_or_default();
    st.store.delete_user_sessions(u.id, &except);
    st.audit(Some(&p), &c, "user.password_changed", &u.username, "");
    ok_true()
}

/// Creates a pending secret for enrollment.
pub async fn totp_begin(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
) -> ApiResult {
    let secret = auth::new_totp_secret();
    let enc = st
        .store
        .secret_box()
        .seal(secret.as_bytes())
        .map_err(|e| ApiError::internal(e.to_string()))?;
    st.store
        .kv_set_json(&format!("totp_pending:{}", p.user.id), &enc)?;
    let uri = auth::totp_uri("Caddymium", &p.user.username, &secret);
    // the QR code as SVG, so a page needs no QR library
    let qr = qrcode::QrCode::new(uri.as_bytes())
        .map(|c| {
            c.render::<qrcode::render::svg::Color>()
                .min_dimensions(200, 200)
                .build()
        })
        .unwrap_or_default();
    Ok(json(
        StatusCode::OK,
        &serde_json::json!({"secret": secret, "uri": uri, "qr_svg": qr}),
    ))
}

pub async fn totp_enable(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(req): JsonBody<CodeReq>,
) -> ApiResult {
    let key = format!("totp_pending:{}", p.user.id);
    let enc: Vec<u8> = st
        .store
        .kv_get_json(&key)
        .ok_or_else(|| ApiError::bad("start 2FA setup first"))?;
    let plain = st
        .store
        .secret_box()
        .open(&enc)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut u = st.store.user_by_id(p.user.id)?;
    u.totp_secret = String::from_utf8_lossy(&plain).to_string();
    if !check_totp(&st, &u, &req.code) {
        return Err(ApiError::bad(
            "that code doesn't match; check your phone's clock",
        ));
    }
    u.totp_enabled = true;
    let u = st.store.save_user(u)?;
    let _ = st.store.kv_set(&key, "null");
    st.audit(Some(&p), &c, "user.2fa_enabled", &u.username, "");
    ok_true()
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PasswordOnly {
    password: String,
}

pub async fn totp_disable(
    State(st): State<Arc<AppState>>,
    Extension(p): Extension<Principal>,
    c: Client,
    JsonBody(req): JsonBody<PasswordOnly>,
) -> ApiResult {
    let mut u = st.store.user_by_id(p.user.id)?;
    if !auth::verify_password(&u.password_hash, &req.password) {
        return Err(ApiError(StatusCode::FORBIDDEN, "password is wrong".into()));
    }
    u.totp_enabled = false;
    u.totp_secret = String::new();
    let u = st.store.save_user(u)?;
    st.audit(Some(&p), &c, "user.2fa_disabled", &u.username, "");
    ok_true()
}

#[allow(dead_code)]
fn _unused(r: axum::response::Response) -> axum::response::Response {
    r.into_response()
}
