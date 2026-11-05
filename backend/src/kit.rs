//! The Krifmaz kit's server crates, wired to the panel.
//!
//! | Route | From |
//! | --- | --- |
//! | `GET /api/health` | kit-server-health |
//! | `/api/changelog`, `/api/changelog/new`, `/api/changelog/seen` (What's new), `/api/auth/prefs` (its switch) | kit-server-changelog |
//! | `/api/notices`, `/api/notices/check`, `/api/notices/{id}`, `/api/notices/{id}/check` | kit-server-notices |
//! | `/api/bug-reports/*`, `/api/crash-reports` | kit-server-bugreports |
//! | `/api/diagnostics`, `/api/diagnostics/{id}/test` | kit-server-diagnostics |
//!
//! The kit keeps its own tables (`kit_*`) in `kit.db` next to the panel's
//! database, so the panel's backups and migrations never touch them. Who is
//! signed in comes from the panel's own sessions and API keys.

use axum::Router;
use axum::http::request::Parts;
use kit_server_changelog::{Changelog, ChangelogHost};
use kit_server_core::{Account, Host, PrefsStore, SqliteDb, SqlitePrefs, err};
use kit_server_diagnostics::{Check, Kind, Registry};
use kit_server_notices::{Notice, NoticeCheck, Notices, Severity};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::auth;
use crate::notify;
use crate::server::AppState;

/// The release notes built in from docs/releases by build.rs: `NOTES`.
mod changelog_gen {
    include!(concat!(env!("OUT_DIR"), "/changelog_gen.rs"));
}

/// The name people see.
pub const APP_NAME: &str = "Caddymium";

/// Notice: Caddy is not serving.
pub const NOTICE_CADDY: &str = "caddy";
/// Notice: the last automatic Caddy update failed.
pub const NOTICE_UPDATE: &str = "update";
/// Notice: the data folder can't be written.
pub const NOTICE_DATA: &str = "data";

/// The kit's state, made once at start.
pub struct Kit {
    pub prefs: SqlitePrefs,
    pub reports: kit_server_bugreports::SqliteReports,
    pub changelog: Arc<Changelog>,
    pub notices: Notices<KitState>,
    pub data_dir: PathBuf,
    limits: Mutex<std::collections::HashMap<String, Vec<Instant>>>,
}

impl Kit {
    /// Opens (or makes) `<data_dir>/kit.db` with the kit's tables.
    pub fn open(data_dir: &Path, version: &str) -> anyhow::Result<Kit> {
        std::fs::create_dir_all(data_dir)?;
        Self::with_db(
            SqliteDb::open(&data_dir.join("kit.db"))
                .map_err(|e| anyhow::anyhow!("The kit's database can't be opened: {}", e.detail))?,
            data_dir,
            version,
        )
    }

    fn with_db(db: SqliteDb, data_dir: &Path, version: &str) -> anyhow::Result<Kit> {
        let kit = |e: kit_server_core::Error| {
            anyhow::anyhow!("The kit's tables can't be made: {}", e.detail)
        };
        let prefs = SqlitePrefs::new(db.clone()).map_err(kit)?;
        let reports = kit_server_bugreports::SqliteReports::new(db.clone()).map_err(kit)?;
        let kept = kit_server_notices::SqliteNotices::new(db.clone()).map_err(kit)?;
        let notices = notices().keep_in(Arc::new(kept));
        let version = version.trim_start_matches('v');
        let changelog = Arc::new(Changelog::new(APP_NAME, version, changelog_gen::NOTES));
        Ok(Kit {
            prefs,
            reports,
            changelog,
            notices,
            data_dir: data_dir.to_path_buf(),
            limits: Mutex::new(Default::default()),
        })
    }

    /// An in-memory kit (tests).
    #[cfg(test)]
    pub fn memory(data_dir: &Path) -> Kit {
        Self::with_db(SqliteDb::memory(), data_dir, "0.0.0").unwrap()
    }
}

/// What the kit's routes are handed: the panel's state.
#[derive(Clone)]
pub struct KitState(pub Arc<AppState>);

impl KitState {
    fn kit(&self) -> &Kit {
        &self.0.kit
    }
}

impl Host for KitState {
    async fn signed_in(&self, parts: &mut Parts) -> kit_server_core::Result<Account> {
        let p = self
            .0
            .authenticate(&parts.headers)
            .map_err(|e| err(401, format!("Not signed in: {e}. Sign in again.")))?;
        if p.session.as_ref().is_some_and(|s| s.pending_2fa) {
            return Err(err(401, "Two-factor code required. Sign in again."));
        }
        if let Some(s) = &p.session {
            let safe = matches!(parts.method.as_str(), "GET" | "HEAD" | "OPTIONS");
            let token = parts
                .headers
                .get("x-csrf-token")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !safe && token != s.csrf {
                return Err(err(
                    403,
                    "Missing or invalid CSRF token. Reload the page and try again.",
                ));
            }
        }
        let admin = p.role == auth::ROLE_ADMIN;
        Ok(Account {
            id: p.user.id,
            username: p.user.username.clone(),
            admin,
            owner: admin,
        })
    }
}

impl ChangelogHost for KitState {
    fn changelog(&self) -> &Changelog {
        &self.kit().changelog
    }
    fn prefs(&self) -> &dyn PrefsStore {
        &self.kit().prefs
    }
}

impl kit_server_bugreports::BugReportsHost for KitState {
    fn reports(&self) -> &dyn kit_server_bugreports::BugReportStore {
        &self.kit().reports
    }
    fn data_dir(&self) -> PathBuf {
        self.kit().data_dir.clone()
    }
    fn app_name(&self) -> String {
        APP_NAME.into()
    }
    fn app_version(&self) -> String {
        self.0.version.clone()
    }
    fn limited(&self, key: &str, limit: usize, window_secs: f64) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs_f64(window_secs.max(0.0));
        let mut m = self.kit().limits.lock().unwrap_or_else(|e| e.into_inner());
        let hits = m.entry(key.to_string()).or_default();
        hits.retain(|t| now.duration_since(*t) <= window);
        if hits.len() >= limit {
            return true;
        }
        hits.push(now);
        false
    }
    fn notify(
        &self,
        title: String,
        message: String,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        self.0
            .notifier
            .send(notify::EVENT_BUG_REPORT, &title, &message);
        async {}
    }
}

impl kit_server_bugreports::crash::CrashHost for KitState {
    fn crashes(&self) -> &dyn kit_server_bugreports::CrashStore {
        &self.kit().reports
    }
}

impl kit_server_notices::NoticesHost for KitState {
    fn notices(&self) -> Notices<KitState> {
        self.kit().notices.clone()
    }
}

impl kit_server_diagnostics::Diagnostics for KitState {
    fn diagnostics(&self) -> Registry<KitState> {
        diagnostics()
    }

    /// The checks plus the panel's version, its user, the time and the
    /// environment variables it was started with (secrets hidden).
    fn answer(
        &self,
        checks: Vec<serde_json::Value>,
    ) -> impl std::future::Future<Output = kit_server_core::Result<serde_json::Value>> + Send {
        let version = self.0.version.clone();
        async move {
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            Ok(serde_json::json!({
                "checks": checks,
                "version": version,
                "uid": process_uid(),
                "time": time,
                "settings": env_settings(std::env::vars()),
            }))
        }
    }
}

/// The user the panel runs as, None where that can't be told.
fn process_uid() -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata("/proc/self").ok().map(|m| m.uid())
    }
    #[cfg(not(unix))]
    None
}

/// The panel's environment variables (`CADDYMIUM_*`, and the older
/// `CADDYHUB_*`) for the Diagnostics page, secrets hidden.
pub fn env_settings(vars: impl Iterator<Item = (String, String)>) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = vars
        .filter(|(k, _)| k.starts_with("CADDYMIUM_") || k.starts_with("CADDYHUB_"))
        .map(|(k, v)| {
            let secret = ["PASSWORD", "TOKEN", "SECRET", "KEY"]
                .iter()
                .any(|w| k.contains(w));
            let shown = if secret && !v.is_empty() {
                "•••• (set)".to_string()
            } else {
                v
            };
            let key = k
                .trim_start_matches("CADDYMIUM_")
                .trim_start_matches("CADDYHUB_")
                .to_ascii_lowercase();
            serde_json::json!({"key": key, "source": "env", "env": k, "value": shown})
        })
        .collect();
    out.sort_by(|a, b| a["env"].as_str().cmp(&b["env"].as_str()));
    out
}

/// The kit's routes, with their state.
pub fn router<S: Clone + Send + Sync + 'static>(st: Arc<AppState>) -> Router<S> {
    let ks = KitState(st);
    let notices = ks.kit().notices.clone();
    tokio::spawn(async move {
        if let Err(e) = notices.restore().await {
            tracing::warn!("the saved notices can't be read: {}", e.detail);
        }
    });
    kit_server_notices::spawn_rechecks(ks.clone());
    let health = kit_server_health::Health::new(
        ks.0.version.trim_start_matches('v'),
        crate::web::next_build(),
    );
    Router::new()
        .merge(kit_server_health::router(health))
        .merge(kit_server_changelog::router())
        .merge(kit_server_changelog::prefs_router("/api/auth/prefs"))
        .merge(kit_server_notices::router())
        .merge(kit_server_bugreports::router())
        .merge(kit_server_bugreports::crash::router())
        .merge(kit_server_diagnostics::router())
        .with_state(ks)
}

// ---------------------------------------------------------------- notices

/// Whether a file can be made in `dir`: `Ok`, or the reason in words.
pub fn writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("missing".into()),
        Err(e) if e.raw_os_error() == Some(30) => Err("read-only".into()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err("not writable by the panel's user".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Why Caddy isn't serving, or None when it is (or isn't expected to run).
async fn caddy_problem(st: &AppState) -> Option<String> {
    if !st.caps().detected {
        return None;
    }
    if st
        .plugins
        .current_job()
        .is_some_and(|j| j.status == "running")
    {
        return None;
    }
    st.plugins
        .restarter
        .serving()
        .await
        .err()
        .map(|e| format!("{e:#}"))
}

/// The banner text for a Caddy that isn't serving.
pub fn caddy_notice_text(why: &str) -> String {
    format!(
        "Caddy isn't serving your sites ({why}). The watchdog restarts it after a minute; if this stays, press Restart Caddy on the Plugins page."
    )
}

/// The banner text for a failed automatic update, or None.
fn update_notice_text(st: &AppState) -> Option<String> {
    let u = st.updates.state_settled();
    let a = u.last_auto.as_ref()?;
    if a.ok {
        return None;
    }
    Some(format!(
        "The automatic Caddy update to {} failed: {}. Caddy still runs the previous version; see the Plugins page.",
        a.to, a.error
    ))
}

fn notices() -> Notices<KitState> {
    Notices::new()
        .register(
            NoticeCheck::kept(NOTICE_CADDY, Severity::Err, |st: KitState| async move {
                match caddy_problem(&st.0).await {
                    None => Notice::fine(),
                    Some(why) => Notice::failing(caddy_notice_text(&why)),
                }
            })
            .every(Duration::from_secs(30)),
        )
        .register(
            NoticeCheck::kept(NOTICE_UPDATE, Severity::Warn, |st: KitState| async move {
                let s = st.0.clone();
                match tokio::task::spawn_blocking(move || update_notice_text(&s)).await {
                    Ok(None) => Notice::fine(),
                    Ok(Some(t)) => Notice::failing(t),
                    Err(e) => Notice::failing(e.to_string()),
                }
            })
            .every(Duration::from_secs(300)),
        )
        .register(
            NoticeCheck::kept(NOTICE_DATA, Severity::Warn, |st: KitState| async move {
                let dir = st.kit().data_dir.clone();
                match tokio::task::spawn_blocking(move || writable(&dir).map_err(|w| (dir, w)))
                    .await
                {
                    Ok(Ok(())) => Notice::fine(),
                    Ok(Err((dir, why))) => Notice::failing(format!(
                        "The data folder {} is {why}: settings and certificates can't be saved. Check the disk or the volume mount.",
                        dir.display()
                    )),
                    Err(e) => Notice::failing(e.to_string()),
                }
            })
            .every(Duration::from_secs(60)),
        )
}

// ---------------------------------------------------------------- diagnostics

fn diagnostics() -> Registry<KitState> {
    Registry::new()
        .check("caddy_admin", |st: KitState| async move {
            let probe = st.0.plugins.restarter.timing.probe;
            let r = tokio::time::timeout(probe, st.0.admin.ping()).await;
            match r {
                Ok(Ok(())) => {
                    let v = st.0.caps().version_s;
                    let v = if v.is_empty() { "answers".into() } else { format!("answers · Caddy {v}") };
                    Check::new("Caddy admin API", Kind::Service, true, v)
                }
                Ok(Err(e)) => Check::new("Caddy admin API", Kind::Service, false, format!("{e:#}"))
                    .fix("Check that Caddy runs (docker ps) and that CADDYMIUM_CADDY_ADMIN points at its admin address; press Restart Caddy on the Plugins page"),
                Err(_) => Check::new("Caddy admin API", Kind::Service, false, "timed out")
                    .fix("Caddy is running but stuck; press Restart Caddy on the Plugins page"),
            }
        })
        .check("caddy_serving", |st: KitState| async move {
            match st.0.plugins.restarter.serving().await {
                Ok(()) => Check::new("Sites served", Kind::Service, true, "every listener answers"),
                Err(e) => Check::new("Sites served", Kind::Service, false, format!("{e:#}"))
                    .fix("Press Restart Caddy on the Plugins page; if a port is taken, stop what else listens on it"),
            }
        })
        .check("data", |st: KitState| async move {
            let dir = st.kit().data_dir.clone();
            let shown = dir.display().to_string();
            match tokio::task::spawn_blocking(move || writable(&dir))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            {
                Ok(()) => Check::new("Data folder", Kind::Path, true, format!("{shown} · writable"))
                    .request(shown),
                Err(why) => Check::new("Data folder", Kind::Path, false, format!("{shown} · {why}"))
                    .request(shown)
                    .fix("Check the disk and the /data volume mount, and that the panel's user may write to it"),
            }
        })
        .check("database", |st: KitState| async move {
            let s = st.0.clone();
            match tokio::task::spawn_blocking(move || s.store.sites().map(|v| v.len())).await {
                Ok(Ok(n)) => Check::new("Database", Kind::Path, true, format!("OK · {n} sites")),
                Ok(Err(e)) => Check::new("Database", Kind::Path, false, format!("{e:#}"))
                    .fix("Restore the last backup (Settings → Backup) or check the disk"),
                Err(e) => Check::new("Database", Kind::Path, false, e.to_string()),
            }
        })
        .check("caddy_logs", |st: KitState| async move {
            let dir = st.0.log_dir.clone();
            if dir.is_empty() {
                return Check::new("Caddy logs", Kind::Path, true, "not used");
            }
            let p = PathBuf::from(&dir);
            match tokio::task::spawn_blocking(move || writable(&p))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            {
                Ok(()) => Check::new("Caddy logs", Kind::Path, true, format!("{dir} · writable")).request(dir),
                Err(why) => Check::new("Caddy logs", Kind::Path, false, format!("{dir} · {why}"))
                    .request(dir)
                    .fix("Check the log folder's mount and permissions"),
            }
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostics_hide_secrets() {
        let vars = [
            ("CADDYMIUM_SETUP_TOKEN", "abc"),
            ("CADDYMIUM_LISTEN", ":8080"),
            ("PATH", "/bin"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()));
        let s = super::env_settings(vars);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0]["env"], "CADDYMIUM_LISTEN");
        assert_eq!(s[0]["value"], ":8080");
        assert_eq!(s[1]["value"], "•••• (set)");
    }
}
