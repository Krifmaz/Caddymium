//! Caddymium's HTTP layer: a JSON REST API under /api and the embedded web
//! UI on every other path.
//!
//! Authentication uses an opaque session cookie (only its SHA-256 is stored)
//! plus a per-session CSRF token that the UI sends in X-CSRF-Token on every
//! state-changing request. API keys (Authorization: Bearer) skip CSRF.

mod auth_handlers;
mod imports;
mod netscan;
mod ops;
mod plugin_handlers;
pub(crate) mod resources;
mod security_handlers;
mod sso;

#[cfg(test)]
mod tests;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequest, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::Utc;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::apply::Pipeline;
use crate::auth;
use crate::caddy::{Admin, Runner};
use crate::notify::Notifier;
use crate::plugins;
use crate::registry::Capabilities;
use crate::store::{self, Session, Store, User};
use crate::updates;

pub const COOKIE_NAME: &str = "caddymium_session";
const SESSION_ABSOLUTE: chrono::Duration = chrono::Duration::days(7);
const MAX_BODY: usize = 4 << 20;

/// Files the panel can import from without pasting.
#[derive(Debug, Clone, Default)]
pub struct ImportPaths {
    /// autosave.json saved before Caddymium first started Caddy
    pub autosave_backup: PathBuf,
    /// Caddy's live autosave.json
    pub autosave: PathBuf,
    /// an old Caddyfile mounted into the panel
    pub caddyfile: PathBuf,
}

/// Everything the handlers share.
pub struct AppState {
    pub store: Arc<Store>,
    pub pipeline: Arc<Pipeline>,
    pub runner: Arc<Runner>,
    pub admin: Arc<Admin>,
    pub notifier: Arc<Notifier>,
    pub caps: Arc<RwLock<Capabilities>>,
    pub plugins: Arc<plugins::Manager>,
    pub updates: Arc<updates::Checker>,
    pub crowdsec: Arc<crate::crowdsec::Manager>,
    pub log_dir: String,
    pub version: String,
    /// optional token required by first-run setup
    pub setup_token: String,
    /// trust X-Forwarded-For (panel behind Caddy)
    pub trust_xff: bool,
    /// set Secure on cookies
    pub secure_cookie: bool,
    pub dev_no_auth: bool,
    pub imports: ImportPaths,
    /// The kit's server crates (What's new, notices, bug reports, diagnostics).
    pub kit: crate::kit::Kit,
    limiter: Mutex<Limiter>,
    scan_lock: tokio::sync::Mutex<()>,
}

pub struct AppParts {
    pub store: Arc<Store>,
    pub crowdsec: Arc<crate::crowdsec::Manager>,
    /// Caddy's log directory (visitor check on the Security page).
    pub log_dir: String,
    pub pipeline: Arc<Pipeline>,
    pub runner: Arc<Runner>,
    pub admin: Arc<Admin>,
    pub notifier: Arc<Notifier>,
    pub caps: Arc<RwLock<Capabilities>>,
    pub plugins: Arc<plugins::Manager>,
    pub version: String,
    pub setup_token: String,
    pub trust_xff: bool,
    pub secure_cookie: bool,
    pub dev_no_auth: bool,
    pub imports: ImportPaths,
    pub kit: crate::kit::Kit,
}

impl AppState {
    pub fn new(p: AppParts) -> Arc<AppState> {
        let updates = Arc::new(updates::Checker {
            store: p.store.clone(),
            plugins: p.plugins.clone(),
            notifier: p.notifier.clone(),
            caps: p.caps.clone(),
        });
        Arc::new(AppState {
            store: p.store,
            pipeline: p.pipeline,
            runner: p.runner,
            admin: p.admin,
            notifier: p.notifier,
            caps: p.caps,
            plugins: p.plugins,
            updates,
            crowdsec: p.crowdsec,
            log_dir: p.log_dir,
            version: p.version,
            setup_token: p.setup_token,
            trust_xff: p.trust_xff,
            secure_cookie: p.secure_cookie,
            dev_no_auth: p.dev_no_auth,
            imports: p.imports,
            kit: p.kit,
            limiter: Mutex::new(Limiter::new(10, Duration::from_secs(15 * 60))),
            scan_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn caps(&self) -> Capabilities {
        self.caps.read().map(|c| c.clone()).unwrap_or_default()
    }

    fn limiter(&self) -> std::sync::MutexGuard<'_, Limiter> {
        self.limiter.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ---------------------------------------------------------------- errors / json

/// An API error: status code plus a message rendered as {"error": ...}.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn bad(msg: impl Into<String>) -> ApiError {
        ApiError(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn conflict(msg: impl Into<String>) -> ApiError {
        ApiError(StatusCode::CONFLICT, msg.into())
    }
    pub fn internal(msg: impl Into<String>) -> ApiError {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg.into())
    }
}

/// What to do after an unexpected server error.
const SERVER_ERROR_HINT: &str = "Try again; if it keeps failing, copy this message and check the panel's log (docker logs caddymium).";

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let msg = if self.0.is_server_error() {
            tracing::error!(status = self.0.as_u16(), "{}", self.1);
            let m = self.1.trim_end_matches('.');
            format!("Something went wrong: {m}. {SERVER_ERROR_HINT}")
        } else {
            self.1
        };
        json(self.0, &serde_json::json!({"error": msg}))
    }
}

/// Maps store errors to HTTP responses.
impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> ApiError {
        if store::is_not_found(&err) {
            return ApiError(
                StatusCode::NOT_FOUND,
                "Not found: it may have been deleted meanwhile. Reload the page.".into(),
            );
        }
        let msg = format!("{err:#}");
        if msg.contains("UNIQUE constraint") {
            return ApiError(
                StatusCode::CONFLICT,
                "an entry with that name already exists".into(),
            );
        }
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

pub type ApiResult = Result<Response, ApiError>;

/// A JSON response with no-store caching.
pub fn json<T: Serialize>(code: StatusCode, v: &T) -> Response {
    let body = serde_json::to_vec(v).unwrap_or_else(|_| b"null".to_vec());
    let mut r = (code, body).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

pub fn ok<T: Serialize>(v: &T) -> ApiResult {
    Ok(json(StatusCode::OK, v))
}

pub fn ok_true() -> ApiResult {
    ok(&serde_json::json!({"ok": true}))
}

/// A JSON request body; an unparsable body is a 400 with a readable message.
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|e| ApiError::bad(format!("invalid body: {e}")))?;
        let v = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::bad(format!("invalid JSON: {e}")))?;
        Ok(JsonBody(v))
    }
}

/// Like JsonBody, but an empty body yields the default.
pub struct OptJson<T>(pub T);

impl<S, T> FromRequest<S> for OptJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Default,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|e| ApiError::bad(format!("invalid body: {e}")))?;
        if bytes.is_empty() {
            return Ok(OptJson(T::default()));
        }
        let v = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::bad(format!("invalid JSON: {e}")))?;
        Ok(OptJson(v))
    }
}

/// A positive numeric path id.
pub struct Id(pub i64);

impl<S: Send + Sync> FromRequestParts<S> for Id {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Path(raw): axum::extract::Path<String> =
            axum::extract::Path::from_request_parts(parts, state)
                .await
                .map_err(|_| ApiError::bad("invalid id"))?;
        match raw.parse::<i64>() {
            Ok(id) if id > 0 => Ok(Id(id)),
            _ => Err(ApiError::bad("invalid id")),
        }
    }
}

// ---------------------------------------------------------------- identity

/// The authenticated caller.
#[derive(Debug, Clone)]
pub struct Principal {
    pub user: User,
    /// None for API keys
    pub session: Option<Session>,
    #[allow(dead_code)]
    pub via_key: bool,
    pub role: String,
}

/// Request details every handler needs for auditing.
#[derive(Debug, Clone)]
pub struct Client {
    pub ip: String,
    pub user_agent: String,
    pub host: String,
}

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<Client>()
            .cloned()
            .unwrap_or_else(|| Client {
                ip: String::new(),
                user_agent: String::new(),
                host: String::new(),
            }))
    }
}

fn header_str(h: &HeaderMap, name: &str) -> String {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Reads a cookie value from the request headers.
pub fn cookie(h: &HeaderMap, name: &str) -> Option<String> {
    for v in h.get_all(header::COOKIE) {
        let Ok(s) = v.to_str() else { continue };
        for part in s.split(';') {
            let part = part.trim();
            if let Some(val) = part.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

impl AppState {
    /// Resolves the caller from a cookie or API key.
    pub fn authenticate(&self, h: &HeaderMap) -> Result<Principal, String> {
        if self.dev_no_auth {
            let u = User {
                id: 0,
                username: "dev".into(),
                role: auth::ROLE_ADMIN.into(),
                ..Default::default()
            };
            return Ok(Principal {
                user: u,
                session: None,
                via_key: false,
                role: auth::ROLE_ADMIN.into(),
            });
        }
        let authz = header_str(h, "authorization");
        if let Some(tok) = authz.strip_prefix("Bearer ") {
            let k = self
                .store
                .api_key_by_hash(&auth::token_hash(tok.trim()))
                .map_err(|_| "invalid API key".to_string())?;
            if k.expires_at.is_some_and(|t| Utc::now() > t) {
                return Err("API key expired".into());
            }
            let u = self
                .store
                .user_by_id(k.user_id)
                .map_err(|_| "API key owner disabled".to_string())?;
            if u.disabled {
                return Err("API key owner disabled".into());
            }
            self.store.touch_api_key(k.id);
            // a key never exceeds its owner
            let role = if auth::allows(&u.role, &k.role) {
                k.role.clone()
            } else {
                u.role.clone()
            };
            return Ok(Principal {
                user: u,
                session: None,
                via_key: true,
                role,
            });
        }
        let c = cookie(h, COOKIE_NAME)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "not logged in".to_string())?;
        let se = self
            .store
            .get_session(&auth::token_hash(&c))
            .map_err(|_| "session expired".to_string())?;
        let now = Utc::now();
        let idle = chrono::Duration::minutes(self.store.settings().session_timeout_minutes.max(0));
        if now > se.expires_at || (!idle.is_zero() && now - se.last_seen > idle) {
            self.store.delete_session(&se.id);
            return Err("session expired".into());
        }
        let u = self
            .store
            .user_by_id(se.user_id)
            .map_err(|_| "account disabled".to_string())?;
        if u.disabled {
            return Err("account disabled".into());
        }
        if now - se.last_seen > chrono::Duration::seconds(30) {
            self.store.touch_session(&se.id, now);
        }
        let role = u.role.clone();
        Ok(Principal {
            user: u,
            session: Some(se),
            via_key: false,
            role,
        })
    }

    pub fn client_ip(&self, h: &HeaderMap, remote: Option<SocketAddr>) -> String {
        if self.trust_xff {
            let xff = header_str(h, "x-forwarded-for");
            if !xff.is_empty() {
                return xff.split(',').next().unwrap_or("").trim().to_string();
            }
        }
        remote.map(|a| a.ip().to_string()).unwrap_or_default()
    }

    pub fn audit(
        &self,
        p: Option<&Principal>,
        c: &Client,
        action: &str,
        target: &str,
        details: &str,
    ) {
        let user = p.map(|p| p.user.username.as_str()).unwrap_or("");
        self.store.audit(user, action, target, details, &c.ip);
    }
}

fn safe_method(m: &Method) -> bool {
    matches!(*m, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Authentication, a role check and CSRF for every protected route.
async fn guard(st: Arc<AppState>, mut req: Request, next: Next, role: &'static str) -> Response {
    let p = match st.authenticate(req.headers()) {
        Ok(p) => p,
        Err(e) => return ApiError(StatusCode::UNAUTHORIZED, e).into_response(),
    };
    if p.session.as_ref().is_some_and(|s| s.pending_2fa) {
        return ApiError(StatusCode::UNAUTHORIZED, "two-factor code required".into())
            .into_response();
    }
    if !auth::allows(&p.role, role) {
        return ApiError(
            StatusCode::FORBIDDEN,
            format!("your role ({}) can't do this", p.role),
        )
        .into_response();
    }
    if let Some(s) = &p.session {
        if !safe_method(req.method()) && header_str(req.headers(), "x-csrf-token") != s.csrf {
            return ApiError(
                StatusCode::FORBIDDEN,
                "missing or invalid CSRF token; reload the page".into(),
            )
            .into_response();
        }
    }
    req.extensions_mut().insert(p);
    next.run(req).await
}

/// Records the client address and sets security headers on every response.
async fn common(State(st): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let remote = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    let c = Client {
        ip: st.client_ip(req.headers(), remote),
        user_agent: header_str(req.headers(), "user-agent")
            .chars()
            .take(200)
            .collect(),
        host: header_str(req.headers(), "host"),
    };
    req.extensions_mut().insert(c);
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    // every answer says it is the panel's: the kit web app tells a proxy's
    // error page (a 502 while the panel restarts) from the panel's own errors
    h.insert("x-caddymium", HeaderValue::from_static("1"));
    resp
}

// ---------------------------------------------------------------- rate limit

/// Counts failed attempts per key in a sliding window.
pub struct Limiter {
    max: usize,
    window: Duration,
    hits: HashMap<String, Vec<Instant>>,
}

impl Limiter {
    fn new(max: usize, window: Duration) -> Limiter {
        Limiter {
            max,
            window,
            hits: HashMap::new(),
        }
    }

    fn prune(&mut self, key: &str, now: Instant) -> usize {
        let window = self.window;
        let n = match self.hits.get_mut(key) {
            Some(h) => {
                h.retain(|t| now.duration_since(*t) <= window);
                h.len()
            }
            None => 0,
        };
        if n == 0 {
            self.hits.remove(key);
        }
        n
    }

    pub fn blocked(&mut self, key: &str) -> bool {
        self.prune(key, Instant::now()) >= self.max
    }

    pub fn fail(&mut self, key: &str) {
        let now = Instant::now();
        self.prune(key, now);
        self.hits.entry(key.to_string()).or_default().push(now);
    }

    pub fn reset(&mut self, key: &str) {
        self.hits.remove(key);
    }
}

// ---------------------------------------------------------------- cookies

pub fn session_cookie(
    st: &AppState,
    value: &str,
    expires: Option<chrono::DateTime<Utc>>,
) -> HeaderValue {
    let mut c = format!("{COOKIE_NAME}={value}; Path=/; HttpOnly; SameSite=Lax");
    if st.secure_cookie {
        c.push_str("; Secure");
    }
    match expires {
        Some(t) => c.push_str(&format!(
            "; Expires={}",
            t.format("%a, %d %b %Y %H:%M:%S GMT")
        )),
        None => c.push_str("; Max-Age=0"),
    }
    HeaderValue::from_str(&c).unwrap_or_else(|_| HeaderValue::from_static(""))
}

// ---------------------------------------------------------------- routes

/// Builds the router.
pub fn router(st: Arc<AppState>) -> Router {
    use auth::{ROLE_ADMIN as A, ROLE_EDITOR as E, ROLE_VIEWER as V};
    let layer = |role: &'static str| {
        middleware::from_fn_with_state(
            st.clone(),
            move |State(s): State<Arc<AppState>>, req: Request, next: Next| {
                guard(s, req, next, role)
            },
        )
    };

    let public = Router::new()
        .route("/me", get(auth_handlers::me))
        .route("/setup", post(auth_handlers::setup))
        .route("/login", post(auth_handlers::login))
        .route("/login/totp", post(auth_handlers::login_totp))
        .route("/logout", post(auth_handlers::logout))
        .route("/sso/verify", get(sso::verify));

    let viewer = Router::new()
        .route("/me/password", post(auth_handlers::change_password))
        .route("/me/totp/begin", post(auth_handlers::totp_begin))
        .route("/me/totp/enable", post(auth_handlers::totp_enable))
        .route("/me/totp/disable", post(auth_handlers::totp_disable))
        .route("/meta", get(ops::meta))
        .route("/status", get(ops::status))
        .route("/upstreams", get(ops::upstreams))
        .route("/upstreams/reach", get(netscan::reach))
        .route("/sites", get(resources::list_sites))
        .route("/sites/{id}", get(resources::get_site))
        .route("/domains", get(resources::list_domains))
        .route("/snippets", get(resources::list_snippets))
        .route("/raw-blocks", get(resources::list_raw_blocks))
        .route("/settings", get(resources::get_settings))
        .route("/config/preview", get(ops::preview))
        .route("/config/plan", post(ops::plan))
        .route("/config/export", get(ops::export))
        .route("/versions", get(ops::list_versions))
        .route("/versions/diff", get(ops::diff_versions))
        .route("/versions/{id}", get(ops::get_version))
        .route("/ddns/status", get(plugin_handlers::ddns_status))
        .route("/updates", get(plugin_handlers::updates))
        .route_layer(layer(V));

    let editor = Router::new()
        .route("/scan", post(netscan::scan))
        .route("/sites", post(resources::create_site))
        .route("/sites/bulk", post(resources::bulk_sites))
        .route(
            "/sites/{id}",
            axum::routing::put(resources::update_site).delete(resources::delete_site),
        )
        .route("/sites/{id}/clone", post(resources::clone_site))
        .route("/snippets", post(resources::save_snippet))
        .route(
            "/snippets/{id}",
            axum::routing::put(resources::save_snippet_id).delete(resources::delete_snippet),
        )
        .route("/config/apply", post(ops::apply))
        .route("/config/discard", post(ops::discard))
        .route("/config/validate", post(ops::validate_text))
        .route("/versions/{id}/rollback", post(ops::rollback_version))
        .route_layer(layer(E));

    let admin = Router::new()
        .route(
            "/security",
            get(security_handlers::get).put(security_handlers::put),
        )
        .route(
            "/security/sites/{id}",
            axum::routing::put(security_handlers::put_site),
        )
        .route(
            "/security/bots/refresh",
            post(security_handlers::refresh_bots),
        )
        .route(
            "/security/suggestions/{id}/apply",
            post(security_handlers::apply_suggestion),
        )
        .route(
            "/security/suggestions/{id}/dismiss",
            post(security_handlers::dismiss_suggestion),
        )
        .route("/security/sessions", get(security_handlers::sessions))
        .route(
            "/security/sessions/{id}",
            axum::routing::delete(security_handlers::end_session),
        )
        .route("/crowdsec", get(security_handlers::crowdsec_status))
        .route("/crowdsec/enable", post(security_handlers::crowdsec_enable))
        .route(
            "/crowdsec/disable",
            post(security_handlers::crowdsec_disable),
        )
        .route("/crowdsec/update", post(security_handlers::crowdsec_update))
        .route("/crowdsec/rules", post(security_handlers::crowdsec_rules))
        .route(
            "/crowdsec/decisions",
            get(security_handlers::crowdsec_decisions).post(security_handlers::crowdsec_ban),
        )
        .route(
            "/crowdsec/decisions/{id}",
            axum::routing::delete(security_handlers::crowdsec_unban),
        )
        .route("/domains", post(resources::save_domain))
        .route(
            "/domains/{id}",
            axum::routing::put(resources::save_domain_id).delete(resources::delete_domain),
        )
        .route(
            "/dns-credentials",
            get(resources::list_credentials).post(resources::save_credential),
        )
        .route(
            "/dns-credentials/{id}",
            axum::routing::put(resources::save_credential_id).delete(resources::delete_credential),
        )
        .route("/raw-blocks", post(resources::save_raw_block))
        .route(
            "/raw-blocks/{id}",
            axum::routing::put(resources::save_raw_block_id).delete(resources::delete_raw_block),
        )
        .route("/settings", axum::routing::put(resources::put_settings))
        .route("/import/sources", get(imports::sources))
        .route("/import/preview", post(imports::preview))
        .route("/import/commit", post(imports::commit))
        .route("/users", get(ops::list_users).post(ops::save_user))
        .route(
            "/users/{id}",
            axum::routing::put(ops::save_user_id).delete(ops::delete_user),
        )
        .route("/api-keys", get(ops::list_keys).post(ops::create_key))
        .route("/api-keys/{id}", axum::routing::delete(ops::delete_key))
        .route(
            "/notifications",
            get(ops::list_channels).post(ops::save_channel),
        )
        .route(
            "/notifications/{id}",
            axum::routing::put(ops::save_channel_id).delete(ops::delete_channel),
        )
        .route("/notifications/test", post(ops::test_channel))
        .route("/audit", get(ops::list_audit))
        .route("/plugins", get(plugin_handlers::status))
        .route(
            "/plugins/catalog/refresh",
            post(plugin_handlers::refresh_catalog),
        )
        .route("/plugins/install", post(plugin_handlers::install))
        .route("/plugins/job", get(plugin_handlers::job))
        .route("/plugins/reset", post(plugin_handlers::reset))
        .route("/plugins/rollback", post(plugin_handlers::rollback))
        .route("/plugins/restart", post(plugin_handlers::restart))
        .route("/updates/check", post(plugin_handlers::updates_check))
        .route("/backup", get(ops::backup_download))
        .route("/backup/restore", post(ops::backup_restore))
        .route_layer(layer(A));

    let api = Router::new()
        .merge(public)
        .merge(viewer)
        .merge(editor)
        .merge(admin)
        .fallback(|| async {
            ApiError(StatusCode::NOT_FOUND, "no such API endpoint".into()).into_response()
        });

    // The sign-in page for sites behind the login wall (Caddy proxies
    // /sso/* on the sign-in host to the panel).
    let sso_pages = Router::new()
        .route("/sso", get(sso::status_page))
        .route("/sso/", get(sso::status_page))
        .route("/sso/login", get(sso::login_page).post(sso::login_submit))
        .route("/sso/enroll", post(sso::enroll_submit))
        .route("/sso/logout", get(sso::logout));

    Router::new()
        .merge(crate::kit::router(st.clone()))
        .nest("/api", api)
        .merge(sso_pages)
        .route("/next", get(crate::web::old_next))
        .route("/next/", get(crate::web::old_next))
        .route("/next/{*path}", get(crate::web::old_next))
        .fallback(crate::web::spa)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn_with_state(st.clone(), common))
        .with_state(st)
}
