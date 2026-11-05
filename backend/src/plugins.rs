//! The plugin manager: installs Caddy plugins and updates Caddy itself.
//!
//! Caddy plugins are compiled in, so "installing" one means getting a new
//! binary. The official download service (caddyserver.com/api/download, the
//! same one `caddy add-package` uses) builds a binary with the requested
//! packages on demand. The panel downloads it into the volume it shares with
//! the Caddy container, checks it (version, modules, and that the current
//! config validates with it), then asks Caddy to exit; the container's
//! entrypoint restarts with the new binary. The previous build is kept for a
//! rollback.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::apply::Pipeline;
use crate::caddy::control::{Control, NoControl};
use crate::caddy::{Admin, Runner};
use crate::notify;
use crate::recovery::{Fallback, Outcome, Restarter};
use crate::registry::{self, Capabilities};
use crate::store::Store;

/// Default download/catalog service.
pub const DOWNLOAD_BASE: &str = "https://caddyserver.com";

const CATALOG_KEY: &str = "plugin_catalog";
const LATEST_KEY: &str = "caddy_latest";
const STATE_KEY: &str = "plugins_state";

/// Sends a notification: event, title, message.
pub type NotifyFn = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

/// Name of the custom build inside the shared bin directory.
pub const CUSTOM_NAME: &str = "caddy-custom";

/// A package from the download service, merged with the curated list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogEntry {
    pub path: String,
    pub repo: String,
    pub downloads: i64,
    pub modules: Vec<String>,
    pub description: String,
    pub name: String,
    pub category: String,
    pub featured: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Catalog {
    pub fetched_at: Option<DateTime<Utc>>,
    pub packages: Vec<CatalogEntry>,
}

/// A non-standard package compiled into the running Caddy.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Installed {
    pub path: String,
    pub version: String,
    pub modules: Vec<String>,
    pub name: String,
}

/// Progress of a running or finished install.
#[derive(Debug, Clone, Default, Serialize)]
pub struct JobState {
    pub id: u64,
    /// install, upgrade, reset, rollback
    pub kind: String,
    /// running, done, failed
    pub status: String,
    pub log: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    pub packages: Vec<String>,
    pub started: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished: Option<DateTime<Utc>>,
}

/// What the UI shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub supported: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    pub caddy_version: String,
    pub arch: String,
    pub os: String,
    pub custom_binary: bool,
    pub previous_binary: bool,
    pub installed: Vec<Installed>,
    pub catalog: Vec<CatalogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_fetched_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub latest_caddy: String,
    pub update_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<JobState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    packages: Vec<String>,
    caddy_version: String,
    installed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Latest {
    version: String,
    checked_at: Option<DateTime<Utc>>,
}

pub struct Manager {
    pub store: Arc<Store>,
    pub admin: Arc<Admin>,
    pub caps: Arc<RwLock<Capabilities>>,
    pub pipeline: Arc<Pipeline>,
    /// the running Caddy binary (copied into the shared volume by the
    /// caddy container's entrypoint)
    pub caddy_bin: PathBuf,
    pub arch_override: String,
    pub github_token: String,
    /// download service base URL (overridable for tests)
    pub base_url: String,
    /// ask Caddy to restart after swapping binaries (off in tests)
    pub restart: bool,
    /// Restarts Caddy and makes sure it serves, or rolls back.
    pub restarter: Arc<Restarter>,
    /// Sends a notification (event, title, message).
    pub notify: Option<NotifyFn>,
    client: reqwest::Client,
    job: Mutex<Option<JobState>>,
    next_id: Mutex<u64>,
}

/// Maps Rust's arch names to Go's.
fn go_arch(a: &str) -> &str {
    match a {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        "arm" => "arm",
        "riscv64" => "riscv64",
        "s390x" => "s390x",
        "powerpc64" => "ppc64le",
        other => other,
    }
}

fn go_os(o: &str) -> &str {
    match o {
        "macos" => "darwin",
        other => other,
    }
}

/// Checks a package path before it goes into a URL.
pub fn valid_package(p: &str) -> bool {
    !p.is_empty()
        && p.len() < 200
        && p.contains('/')
        && !p.contains("..")
        && p.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '/' | '-' | '_' | '@' | '+' | '~')
        })
}

impl Manager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<Store>,
        admin: Arc<Admin>,
        caps: Arc<RwLock<Capabilities>>,
        pipeline: Arc<Pipeline>,
        caddy_bin: PathBuf,
        arch_override: String,
        github_token: String,
    ) -> Manager {
        Manager {
            store,
            admin: admin.clone(),
            caps,
            pipeline,
            caddy_bin,
            arch_override,
            github_token,
            base_url: DOWNLOAD_BASE.into(),
            restart: true,
            restarter: Arc::new(Restarter::new(
                admin.clone(),
                Arc::new(NoControl),
                Vec::new(),
            )),
            notify: None,
            client: crate::httpc::builder()
                .timeout(Duration::from_secs(20 * 60))
                .build()
                .expect("http client"),
            job: Mutex::new(None),
            next_id: Mutex::new(1),
        }
    }

    /// Sets how Caddy is restarted: the process control of this layout, the
    /// addresses that must answer when Caddy serves, and where to notify.
    pub fn with_recovery(
        mut self,
        control: Arc<dyn Control>,
        probe_addrs: Vec<String>,
        notify: Option<NotifyFn>,
    ) -> Manager {
        self.restarter = Arc::new(Restarter::new(self.admin.clone(), control, probe_addrs));
        self.notify = notify;
        self
    }

    pub fn send(&self, event: &str, title: &str, msg: &str) {
        if let Some(n) = &self.notify {
            n(event, title, msg);
        }
    }

    fn bin_dir(&self) -> PathBuf {
        self.caddy_bin
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn custom_path(&self) -> PathBuf {
        self.bin_dir().join(CUSTOM_NAME)
    }

    fn prev_path(&self) -> PathBuf {
        self.bin_dir().join(format!("{CUSTOM_NAME}.prev"))
    }

    fn download_path(&self) -> PathBuf {
        self.bin_dir().join("caddy.download")
    }

    fn caps(&self) -> Capabilities {
        self.caps.read().map(|c| c.clone()).unwrap_or_default()
    }

    /// Whether the shared bin directory can be written.
    pub fn writable(&self) -> Result<()> {
        let dir = self.bin_dir();
        if !dir.is_dir() {
            return Err(anyhow!("{} is not a directory", dir.display()));
        }
        let probe = dir.join(".caddymium-write-test");
        std::fs::write(&probe, b"ok")
            .with_context(|| format!("{} is not writable by the panel", dir.display()))?;
        let _ = std::fs::remove_file(&probe);
        Ok(())
    }

    pub fn current_job(&self) -> Option<JobState> {
        self.job.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn log(&self, line: &str) {
        tracing::info!(target: "plugins", "{line}");
        if let Some(j) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            j.log
                .push(format!("{} {line}", Utc::now().format("%H:%M:%S")));
        }
    }

    fn finish(&self, result: Result<()>) {
        if let Some(j) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            j.finished = Some(Utc::now());
            match result {
                Ok(()) => j.status = "done".into(),
                Err(e) => {
                    j.status = "failed".into();
                    j.error = format!("{e:#}");
                    j.log
                        .push(format!("{} ERROR: {e:#}", Utc::now().format("%H:%M:%S")));
                }
            }
        }
    }

    fn start_job(&self, kind: &str, packages: Vec<String>) -> Result<JobState> {
        let mut job = self.job.lock().unwrap_or_else(|e| e.into_inner());
        if job.as_ref().is_some_and(|j| j.status == "running") {
            return Err(anyhow!("another plugin operation is still running"));
        }
        let mut id = self.next_id.lock().unwrap_or_else(|e| e.into_inner());
        let st = JobState {
            id: *id,
            kind: kind.into(),
            status: "running".into(),
            packages,
            started: Utc::now(),
            ..Default::default()
        };
        *id += 1;
        *job = Some(st.clone());
        Ok(st)
    }

    // ---------------------------------------------------------------- catalog

    /// The cached catalog, fetching it when missing or older than a day.
    pub async fn catalog(&self, force: bool) -> Catalog {
        let cached: Option<Catalog> = self.store.kv_get_json(CATALOG_KEY);
        let fresh = cached
            .as_ref()
            .and_then(|c| c.fetched_at)
            .is_some_and(|t| Utc::now() - t < chrono::Duration::hours(24));
        if !force && fresh {
            if let Some(c) = cached {
                return c;
            }
        }
        match self.fetch_catalog().await {
            Ok(c) => {
                let _ = self.store.kv_set_json(CATALOG_KEY, &c);
                c
            }
            Err(e) => {
                tracing::warn!(err = %e, "plugin catalog unavailable");
                cached.unwrap_or_else(|| Catalog {
                    fetched_at: None,
                    packages: curated_only(),
                })
            }
        }
    }

    async fn fetch_catalog(&self) -> Result<Catalog> {
        #[derive(Deserialize)]
        struct Resp {
            result: Vec<Pkg>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Pkg {
            path: String,
            repo: String,
            downloads: i64,
            listed: bool,
            available: bool,
            modules: Vec<Module>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Module {
            name: String,
            docs: String,
        }
        let url = format!("{}/api/packages", self.base_url);
        let r: Resp = crate::httpc::builder()
            .timeout(Duration::from_secs(60))
            .build()?
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let curated: BTreeMap<String, registry::Plugin> = registry::features::plugins()
            .into_iter()
            .map(|p| (p.path.clone(), p))
            .collect();
        let mut packages: Vec<CatalogEntry> = Vec::new();
        for p in r.result {
            if !p.listed || !p.available || p.path.is_empty() {
                continue;
            }
            let mut e = CatalogEntry {
                path: p.path.clone(),
                repo: p.repo,
                downloads: p.downloads,
                modules: p.modules.iter().map(|m| m.name.clone()).collect(),
                description: p
                    .modules
                    .first()
                    .map(|m| first_sentence(&m.docs))
                    .unwrap_or_default(),
                name: default_name(&p.path),
                category: default_category(
                    &p.path,
                    &p.modules
                        .iter()
                        .map(|m| m.name.as_str())
                        .collect::<Vec<_>>(),
                ),
                featured: false,
            };
            if let Some(c) = curated.get(&p.path) {
                e.name = c.name.clone();
                e.category = c.category.clone();
                e.description = c.description.clone();
                e.featured = true;
            }
            packages.push(e);
        }
        // Curated entries the service doesn't list (should not happen, but
        // keeps the UI usable).
        for (path, c) in curated {
            if !packages.iter().any(|e| e.path == path) {
                packages.push(CatalogEntry {
                    path,
                    repo: String::new(),
                    downloads: 0,
                    modules: vec![c.module.clone()],
                    description: c.description,
                    name: c.name,
                    category: c.category,
                    featured: true,
                });
            }
        }
        packages.sort_by(|a, b| {
            b.featured
                .cmp(&a.featured)
                .then(b.downloads.cmp(&a.downloads))
        });
        Ok(Catalog {
            fetched_at: Some(Utc::now()),
            packages,
        })
    }

    /// The newest Caddy release tag (cached for 6 hours unless `force`).
    pub async fn latest_caddy(&self, force: bool) -> String {
        let cached: Option<Latest> = self.store.kv_get_json(LATEST_KEY);
        if let (false, Some(c)) = (force, &cached) {
            if c.checked_at
                .is_some_and(|t| Utc::now() - t < chrono::Duration::hours(6))
            {
                return c.version.clone();
            }
        }
        #[derive(Deserialize)]
        struct Rel {
            tag_name: String,
        }
        let mut req = crate::httpc::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("http client")
            .get("https://api.github.com/repos/caddyserver/caddy/releases/latest");
        if !self.github_token.is_empty() {
            req = req.bearer_auth(&self.github_token);
        }
        match req.send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => {
                let v = r
                    .json::<Rel>()
                    .await
                    .map(|r| r.tag_name)
                    .unwrap_or_default();
                let _ = self.store.kv_set_json(
                    LATEST_KEY,
                    &Latest {
                        version: v.clone(),
                        checked_at: Some(Utc::now()),
                    },
                );
                v
            }
            Err(e) => {
                tracing::debug!(err = %e, "latest caddy version unavailable");
                cached.map(|c| c.version).unwrap_or_default()
            }
        }
    }

    pub async fn status(&self) -> Status {
        let caps = self.caps();
        let catalog = self.catalog(false).await;
        let latest = self.latest_caddy(false).await;
        let (supported, reason) = match self.writable() {
            Ok(()) if caps.detected => (true, String::new()),
            Ok(()) => (
                false,
                "Caddy has not been detected yet; wait a moment and reload".to_string(),
            ),
            Err(e) => (false, format!("{e:#}")),
        };
        let update_available = !latest.is_empty()
            && caps.version.valid
            && registry::parse_version(&latest).valid
            && registry::parse_version(&latest).compare(&caps.version)
                == std::cmp::Ordering::Greater;
        Status {
            supported,
            reason,
            caddy_version: caps.version_s.clone(),
            arch: self.target_arch().await,
            os: go_os(std::env::consts::OS).to_string(),
            custom_binary: self.custom_path().exists(),
            previous_binary: self.prev_path().exists(),
            installed: installed_from(&caps),
            catalog: catalog.packages,
            catalog_fetched_at: catalog.fetched_at,
            latest_caddy: latest,
            update_available,
            job: self.current_job(),
        }
    }

    async fn target_arch(&self) -> String {
        if !self.arch_override.is_empty() {
            return self.arch_override.clone();
        }
        if let Some(a) = Runner::new(&self.caddy_bin).arch().await {
            return a;
        }
        go_arch(std::env::consts::ARCH).to_string()
    }

    // ---------------------------------------------------------------- install

    /// Starts an install/upgrade in the background. `add` and `remove` are
    /// package paths; with both empty the current set is rebuilt against the
    /// latest Caddy release (an upgrade).
    pub fn start_install(
        self: &Arc<Self>,
        add: Vec<String>,
        remove: Vec<String>,
        user: String,
    ) -> Result<JobState> {
        self.writable()?;
        if !self.caps().detected {
            return Err(anyhow!("Caddy has not been detected yet"));
        }
        for p in add.iter().chain(remove.iter()) {
            if !valid_package(p) {
                return Err(anyhow!("{p:?} is not a valid package path"));
            }
        }
        let mut desired: BTreeSet<String> = self.caps().plugin_packages().keys().cloned().collect();
        for r in &remove {
            desired.remove(r);
        }
        for a in &add {
            desired.insert(a.clone());
        }
        let desired: Vec<String> = desired.into_iter().collect();
        let kind = if add.is_empty() && remove.is_empty() {
            "upgrade"
        } else {
            "install"
        };
        let job = self.start_job(kind, desired.clone())?;
        let m = self.clone();
        tokio::spawn(async move {
            let r = m.install(desired, &user).await;
            m.finish(r);
        });
        Ok(job)
    }

    async fn install(&self, desired: Vec<String>, user: &str) -> Result<()> {
        let arch = self.target_arch().await;
        let os = go_os(std::env::consts::OS).to_string();
        self.log(&format!(
            "Requesting Caddy for {os}/{arch} with {} plugin(s): {}",
            desired.len(),
            if desired.is_empty() {
                "(none)".to_string()
            } else {
                desired.join(", ")
            }
        ));
        let mut url = url::Url::parse(&format!("{}/api/download", self.base_url))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("os", &os).append_pair("arch", &arch);
            for p in &desired {
                q.append_pair("p", p);
            }
            q.append_pair("idempotency", &crate::auth::random_token(8));
        }
        let tmp = self.download_path();
        let _ = std::fs::remove_file(&tmp);
        self.log(
            "Downloading (the service builds the binary on demand; this can take a few minutes)…",
        );
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("download request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.pointer("/error/message")
                        .and_then(|m| m.as_str())
                        .map(String::from)
                })
                .unwrap_or_else(|| body.chars().take(300).collect());
            return Err(anyhow!(
                "download service answered HTTP {}: {}",
                status.as_u16(),
                msg.trim()
            ));
        }
        let total = resp.content_length();
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .with_context(|| format!("create {}", tmp.display()))?;
        let mut resp = resp;
        let mut n: u64 = 0;
        let mut next_log: u64 = 8 << 20;
        while let Some(chunk) = resp.chunk().await.context("download interrupted")? {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
            n += chunk.len() as u64;
            if n >= next_log {
                self.log(&format!(
                    "… {} MB{}",
                    n >> 20,
                    total
                        .map(|t| format!(" of {}", t >> 20))
                        .unwrap_or_default()
                ));
                next_log += 8 << 20;
            }
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        drop(file);
        self.log(&format!("Downloaded {} MB", n >> 20));
        if n < 1 << 20 {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow!("the download is too small to be a Caddy binary"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }

        let check = async {
            let r = Runner::new(&tmp);
            let version = r
                .version()
                .await
                .context("the downloaded binary does not run")?;
            let v = version.split_whitespace().next().unwrap_or("").to_string();
            self.log(&format!("New binary reports Caddy {v}"));
            let caps = r
                .capabilities()
                .await
                .context("list modules of the new binary")?;
            let pkgs = caps.plugin_packages();
            for p in &desired {
                if !pkgs.contains_key(p) {
                    return Err(anyhow!(
                        "plugin {p} is not present in the downloaded binary (it registered no module)"
                    ));
                }
            }
            self.log(&format!(
                "{} modules, {} plugin package(s) present",
                caps.modules.len(),
                pkgs.len()
            ));
            self.log("Validating the current configuration with the new binary…");
            let plan = self.pipeline.plan_with(&r).await?;
            if !plan.valid {
                let errs: Vec<String> = plan.errors.iter().map(|m| m.text.clone()).collect();
                return Err(anyhow!(
                    "the current config does not validate with the new build ({}): {}",
                    plan.stage,
                    errs.join("; ")
                ));
            }
            for w in &plan.warnings {
                self.log(&format!("warning: {w}"));
            }
            Ok((v, caps))
        };
        let (new_version, new_caps) = match check.await {
            Ok(x) => x,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };

        // Swap binaries: keep the previous custom build for a rollback.
        let custom = self.custom_path();
        let had_custom = custom.exists();
        let old_state: Option<State> = self.store.kv_get_json(STATE_KEY);
        let old_version = self.caps().version_s;
        if had_custom {
            let _ = std::fs::remove_file(self.prev_path());
            std::fs::rename(&custom, self.prev_path()).context("keep previous build")?;
        }
        std::fs::rename(&tmp, &custom).context("install new build")?;
        self.log(&format!("Installed as {}", custom.display()));
        let _ = self.store.kv_set_json(
            STATE_KEY,
            &State {
                packages: desired.clone(),
                caddy_version: new_version.clone(),
                installed_at: Some(Utc::now()),
            },
        );

        if !self.restart {
            self.log("Restart skipped");
            self.audit_install(user, &new_version, &desired);
            return Ok(());
        }
        // Undo the swap: the build that served before this install.
        let (dir, m_custom, m_prev) = (self.bin_dir(), custom.clone(), self.prev_path());
        let undo = Fallback {
            label: if old_version.is_empty() {
                "the previous build".to_string()
            } else {
                format!("the previous build ({old_version})")
            },
            apply: Box::new(move || {
                set_aside(&m_custom, &dir.join(format!("{CUSTOM_NAME}.failed")))?;
                if had_custom {
                    std::fs::rename(&m_prev, &m_custom).context("restore the previous build")?;
                }
                Ok(())
            }),
        };
        let mut fallbacks = vec![undo];
        if had_custom {
            fallbacks.push(self.image_fallback());
        }
        match self
            .restart_verified(Some((&new_version, &new_caps)), fallbacks)
            .await
        {
            Ok(()) => {
                self.audit_install(user, &new_version, &desired);
                Ok(())
            }
            Err(e) => {
                match old_state {
                    Some(st) => {
                        let _ = self.store.kv_set_json(STATE_KEY, &st);
                    }
                    None => {
                        let _ = self.store.kv_set_json(STATE_KEY, &State::default());
                    }
                }
                self.store.audit(
                    user,
                    "plugins.install_failed",
                    &new_version,
                    &format!("{e:#}"),
                    "",
                );
                Err(e)
            }
        }
    }

    fn audit_install(&self, user: &str, version: &str, desired: &[String]) {
        self.store
            .audit(user, "plugins.install", version, &desired.join(", "), "");
    }

    /// Falls back to the image's built-in Caddy by setting the custom build
    /// aside.
    fn image_fallback(&self) -> Fallback {
        let (custom, broken) = (
            self.custom_path(),
            self.bin_dir().join(format!("{CUSTOM_NAME}.broken")),
        );
        Fallback {
            label: "the image's built-in Caddy".into(),
            apply: Box::new(move || set_aside(&custom, &broken)),
        }
    }

    /// Restarts Caddy and makes sure it serves (and, with `expect`, that it
    /// is the expected build); otherwise rolls back through `fallbacks`. An
    /// error says what happened and what to do, and was also notified.
    async fn restart_verified(
        &self,
        expect: Option<(&str, &Capabilities)>,
        fallbacks: Vec<Fallback>,
    ) -> Result<()> {
        let out = self.restart_outcome(expect, fallbacks).await;
        self.outcome_result(&out).await
    }

    async fn restart_outcome(
        &self,
        expect: Option<(&str, &Capabilities)>,
        fallbacks: Vec<Fallback>,
    ) -> Outcome {
        let log = |l: &str| self.log(l);
        let check =
            || -> crate::apply::BoxFut<'_, Result<()>> { Box::pin(self.refresh_caps(expect)) };
        self.restarter.restart(&log, &check, &fallbacks).await
    }

    /// Notifies a rollback or an outage and turns it into an error that says
    /// what happened and what to do.
    async fn outcome_result(&self, out: &Outcome) -> Result<()> {
        match out {
            Outcome::Serving => Ok(()),
            Outcome::RolledBack { to, why } => {
                let _ = self.refresh_caps(None).await;
                let msg = format!(
                    "The restarted Caddy did not serve: {why}. Rolled back to {to}, which is serving your sites again. \
                     Nothing to do right now; see the job log on the Plugins page for details."
                );
                self.send(notify::EVENT_CADDY_RECOVERED, "Caddy rolled back", &msg);
                Err(anyhow!(msg))
            }
            Outcome::Down { why } => {
                let msg = format!(
                    "Caddy is not serving: {why}. Restarting it and rolling back did not help. \
                     Check the Caddy log (docker logs on the Caddy container), fix the cause, then use Restart Caddy on the Plugins page."
                );
                self.send(notify::EVENT_CADDY_DOWN, "Caddy is down", &msg);
                Err(anyhow!(msg))
            }
        }
    }

    /// The watchdog's restart: shown as a job on the Plugins page, falls back
    /// to the image's Caddy when the custom build won't serve. Fails without
    /// doing anything while another job runs.
    pub async fn recover(&self, why: &str) -> Result<Outcome> {
        self.start_job("watchdog", Vec::new())?;
        self.log(why);
        self.store.audit("watchdog", "caddy.restart", "", why, "");
        let out = self
            .restart_outcome(None, vec![self.image_fallback()])
            .await;
        let r = self.outcome_result(&out).await;
        self.finish(r);
        Ok(out)
    }

    /// Re-detects the running Caddy and, with `expect`, checks it is that
    /// build with those plugins.
    async fn refresh_caps(&self, expect: Option<(&str, &Capabilities)>) -> Result<()> {
        // The supervisor/entrypoint copies the binary it runs into place.
        let mut caps = Capabilities::default();
        for _ in 0..20 {
            if let Ok(c) = Runner::new(&self.caddy_bin).capabilities().await {
                caps = c;
                if expect.is_none_or(|(v, _)| caps.version_s == v) {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if caps.detected {
            if let Ok(mut w) = self.caps.write() {
                *w = caps.clone();
            }
        }
        let Some((expect_version, want)) = expect else {
            if caps.detected {
                self.log(&format!(
                    "Caddy {} is running with {} modules",
                    caps.version_s,
                    caps.modules.len()
                ));
            }
            return Ok(());
        };
        if caps.version_s != expect_version {
            return Err(anyhow!(
                "Caddy came back as {} instead of {expect_version}",
                if caps.version_s.is_empty() {
                    "an unknown version".to_string()
                } else {
                    caps.version_s.clone()
                }
            ));
        }
        let have = caps.plugin_packages();
        let missing: Vec<String> = want
            .plugin_packages()
            .into_keys()
            .filter(|p| !have.contains_key(p))
            .collect();
        if !missing.is_empty() {
            return Err(anyhow!("Caddy restarted without: {}", missing.join(", ")));
        }
        self.log(&format!(
            "Caddy {} is running with {} modules",
            caps.version_s,
            caps.modules.len()
        ));
        Ok(())
    }

    /// Removes the custom build so the container's built-in Caddy is used.
    pub fn start_reset(self: &Arc<Self>, user: String) -> Result<JobState> {
        if !self.custom_path().exists() {
            return Err(anyhow!("no custom build is installed"));
        }
        let job = self.start_job("reset", Vec::new())?;
        let m = self.clone();
        tokio::spawn(async move {
            let r = async {
                let _ = std::fs::remove_file(m.prev_path());
                std::fs::rename(m.custom_path(), m.prev_path()).context("remove custom build")?;
                m.log("Custom build removed; Caddy will start from the image's built-in binary");
                let old_state: Option<State> = m.store.kv_get_json(STATE_KEY);
                let _ = m.store.kv_set_json(STATE_KEY, &State::default());
                m.store.audit(&user, "plugins.reset", "", "", "");
                if m.restart {
                    let (custom, prev) = (m.custom_path(), m.prev_path());
                    let undo = Fallback {
                        label: "the custom build".into(),
                        apply: Box::new(move || {
                            std::fs::rename(&prev, &custom).context("restore the custom build")
                        }),
                    };
                    if let Err(e) = m.restart_verified(None, vec![undo]).await {
                        if let Some(st) = old_state {
                            let _ = m.store.kv_set_json(STATE_KEY, &st);
                        }
                        return Err(e);
                    }
                }
                Ok(())
            }
            .await;
            m.finish(r);
        });
        Ok(job)
    }

    /// Restarts Caddy (killing it if it does not answer) and makes sure it
    /// serves; falls back to the image's Caddy when the custom build won't.
    pub fn start_restart(self: &Arc<Self>, user: String) -> Result<JobState> {
        let job = self.start_job("restart", Vec::new())?;
        let m = self.clone();
        tokio::spawn(async move {
            m.store.audit(&user, "caddy.restart", "", "", "");
            let r = m.restart_verified(None, vec![m.image_fallback()]).await;
            m.finish(r);
        });
        Ok(job)
    }

    /// Swaps the previous custom build back in.
    pub fn start_rollback(self: &Arc<Self>, user: String) -> Result<JobState> {
        if !self.prev_path().exists() {
            return Err(anyhow!("no previous build to roll back to"));
        }
        let job = self.start_job("rollback", Vec::new())?;
        let m = self.clone();
        tokio::spawn(async move {
            let r = async {
                let (custom, prev) = (m.custom_path(), m.prev_path());
                let tmp = m.bin_dir().join(format!("{CUSTOM_NAME}.swap"));
                if custom.exists() {
                    std::fs::rename(&custom, &tmp)?;
                }
                std::fs::rename(&prev, &custom).context("restore previous build")?;
                if tmp.exists() {
                    std::fs::rename(&tmp, &prev)?;
                }
                let v = Runner::new(&custom).version().await.unwrap_or_default();
                m.log(&format!(
                    "Restored previous build ({})",
                    v.split_whitespace().next().unwrap_or("unknown version")
                ));
                m.store.audit(&user, "plugins.rollback", &v, "", "");
                if m.restart {
                    let (custom, prev, swap) = (m.custom_path(), m.prev_path(), tmp.clone());
                    let undo = Fallback {
                        label: "the build that ran before the rollback".into(),
                        apply: Box::new(move || {
                            std::fs::rename(&custom, &swap)?;
                            std::fs::rename(&prev, &custom)
                                .context("restore the build that ran before")?;
                            std::fs::rename(&swap, &prev)?;
                            Ok(())
                        }),
                    };
                    let image = m.image_fallback();
                    m.restart_verified(None, vec![undo, image]).await?;
                }
                Ok(())
            }
            .await;
            m.finish(r);
        });
        Ok(job)
    }
}

/// Moves a binary out of the way (replacing an older one there); a missing
/// binary is fine.
fn set_aside(bin: &Path, to: &Path) -> Result<()> {
    if !bin.exists() {
        return Ok(());
    }
    let _ = std::fs::remove_file(to);
    std::fs::rename(bin, to).with_context(|| format!("move {} aside", bin.display()))
}

fn installed_from(caps: &Capabilities) -> Vec<Installed> {
    let curated: BTreeMap<String, registry::Plugin> = registry::features::plugins()
        .into_iter()
        .map(|p| (p.path.clone(), p))
        .collect();
    caps.plugin_packages()
        .into_iter()
        .map(|(path, version)| {
            let mut modules: Vec<String> = caps
                .packages
                .iter()
                .filter(|(_, pv)| {
                    pv.rsplit_once('@')
                        .map(|(p, _)| registry::features::strip_major(p) == path)
                        .unwrap_or(false)
                })
                .map(|(m, _)| m.clone())
                .collect();
            modules.sort();
            Installed {
                name: curated
                    .get(&path)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| default_name(&path)),
                path,
                version,
                modules,
            }
        })
        .collect()
}

fn curated_only() -> Vec<CatalogEntry> {
    registry::features::plugins()
        .into_iter()
        .map(|c| CatalogEntry {
            path: c.path,
            repo: String::new(),
            downloads: 0,
            modules: vec![c.module],
            description: c.description,
            name: c.name,
            category: c.category,
            featured: true,
        })
        .collect()
}

fn first_sentence(docs: &str) -> String {
    let first = docs
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let s = first
        .split(". ")
        .next()
        .unwrap_or(first)
        .trim_end_matches('.');
    let s = s.strip_prefix(|_c: char| false).unwrap_or(s);
    // module docs usually start with "module.name is ..." or "module.name wraps ..."
    let s = match s.split_once(' ') {
        Some((head, rest)) if head.contains('.') => rest,
        _ => s,
    };
    let mut out: String = s.chars().take(160).collect();
    if !out.is_empty() {
        let mut c = out.chars();
        out = c
            .next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default();
    }
    out
}

fn default_name(path: &str) -> String {
    let seg = path
        .trim_end_matches("/v2")
        .rsplit('/')
        .next()
        .unwrap_or(path);
    let seg = seg.strip_prefix("caddy-").unwrap_or(seg);
    seg.replace(['-', '_'], " ")
}

fn default_category(path: &str, modules: &[&str]) -> String {
    if path.starts_with("github.com/caddy-dns/")
        || modules.iter().any(|m| m.starts_with("dns.providers."))
    {
        return "dns".into();
    }
    if modules
        .iter()
        .any(|m| m.starts_with("layer4") || m.contains("proxy"))
    {
        return "proxy".into();
    }
    if modules.iter().any(|m| {
        m.contains("auth")
            || m.contains("security")
            || m.contains("rate_limit")
            || m.contains("crowdsec")
            || m.contains("waf")
    }) {
        return "security".into();
    }
    "other".into()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn package_validation_and_names() {
        assert!(valid_package("github.com/caddy-dns/hetzner/v2"));
        assert!(valid_package("github.com/klzgrad/forwardproxy@naive"));
        assert!(!valid_package("github.com/../x"));
        assert!(!valid_package("nope"));
        assert!(!valid_package("github.com/a/b c"));
        assert_eq!(
            default_name("github.com/mholt/caddy-dynamicdns"),
            "dynamicdns"
        );
        assert_eq!(default_name("github.com/caddy-dns/hetzner/v2"), "hetzner");
        assert_eq!(
            default_category("github.com/caddy-dns/x", &["dns.providers.x"]),
            "dns"
        );
        assert_eq!(
            first_sentence(
                "dns.providers.cloudflare wraps the provider implementation as a Caddy module."
            ),
            "Wraps the provider implementation as a Caddy module"
        );
        assert_eq!(go_arch("x86_64"), "amd64");
    }

    /// Downloads a "new build" (the local Caddy served by a tiny HTTP
    /// server), checks it, installs it as the custom build, then rolls back
    /// and resets. Restarts are skipped: there is no container entrypoint
    /// here. Needs CADDY_BIN.
    #[tokio::test]
    async fn install_rollback_reset() {
        let Ok(bin) = std::env::var("CADDY_BIN") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let bin_name = if cfg!(windows) { "caddy.exe" } else { "caddy" };
        let caddy_bin = dir.path().join(bin_name);
        std::fs::copy(&bin, &caddy_bin).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&caddy_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        // A stand-in for caddyserver.com that serves the same binary.
        let payload = std::fs::read(&bin).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = axum::Router::new().route(
            "/api/download",
            axum::routing::get(move || {
                let p = payload.clone();
                async move { p }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let store = Arc::new(
            Store::open_memory(Arc::new(crate::secrets::SecretBox::new(b"k").unwrap())).unwrap(),
        );
        let runner = Arc::new(Runner::new(&caddy_bin));
        let caps = Arc::new(RwLock::new(runner.capabilities().await.unwrap()));
        let admin = Arc::new(Admin::new("127.0.0.1:1"));
        let pipeline = Arc::new(Pipeline::new(crate::apply::PipelineParts {
            store: store.clone(),
            validator: runner.clone(),
            admin: admin.clone(),
            health: Arc::new(crate::apply::DefaultHealth {
                admin: admin.clone(),
                probe_addrs: vec![],
                settle: Duration::ZERO,
            }),
            secrets: Some(Arc::new(crate::secrets::Materializer::new(
                dir.path().join("secrets"),
            ))),
            env: crate::generator::Env {
                admin_listen: "localhost:2019".into(),
                log_dir: crate::secrets::to_slash(&dir.path().join("logs")),
                ..Default::default()
            },
            persist_path: dir.path().join("Caddyfile"),
            caps: caps.clone(),
            notify: None,
        }));
        let mut m = Manager::new(
            store.clone(),
            admin,
            caps.clone(),
            pipeline,
            caddy_bin.clone(),
            "amd64".into(),
            String::new(),
        );
        m.base_url = base;
        m.restart = false;
        let m = Arc::new(m);

        let wait = |m: Arc<Manager>| async move {
            for _ in 0..600 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if let Some(j) = m.current_job() {
                    if j.status != "running" {
                        return j;
                    }
                }
            }
            panic!("job did not finish");
        };

        // A plugin the stand-in can't provide is rejected before anything is installed.
        m.start_install(vec!["github.com/example/nope".into()], vec![], "t".into())
            .unwrap();
        let j = wait(m.clone()).await;
        assert_eq!(j.status, "failed", "{}", j.log.join("\n"));
        assert!(j.error.contains("not present"), "{}", j.error);
        assert!(!m.custom_path().exists());

        // An "upgrade" with the current set succeeds and installs the custom build.
        m.start_install(vec![], vec![], "t".into()).unwrap();
        let j = wait(m.clone()).await;
        assert_eq!(j.status, "done", "{}", j.log.join("\n"));
        assert!(m.custom_path().exists());
        assert!(m.status().await.custom_binary);

        // A second install keeps the previous build; rollback swaps them; reset removes the custom build.
        m.start_install(vec![], vec![], "t".into()).unwrap();
        assert_eq!(wait(m.clone()).await.status, "done");
        assert!(m.prev_path().exists());
        m.start_rollback("t".into()).unwrap();
        assert_eq!(wait(m.clone()).await.status, "done");
        assert!(m.custom_path().exists() && m.prev_path().exists());
        m.start_reset("t".into()).unwrap();
        assert_eq!(wait(m.clone()).await.status, "done");
        assert!(!m.custom_path().exists());
        assert!(store.audit_log("plugins", 10, 0).unwrap().len() >= 3);
    }

    #[test]
    fn installed_list() {
        let mut caps = Capabilities {
            detected: true,
            ..Default::default()
        };
        caps.packages.insert(
            "dns.providers.cloudflare".into(),
            "github.com/caddy-dns/cloudflare@v0.2.4".into(),
        );
        caps.packages.insert(
            "admin.api.load".into(),
            "github.com/caddyserver/caddy/v2@v2.11.7".into(),
        );
        caps.packages.insert(
            "http.handlers.crowdsec".into(),
            "github.com/hslatman/caddy-crowdsec-bouncer@v0.14.1".into(),
        );
        caps.packages.insert(
            "crowdsec".into(),
            "github.com/hslatman/caddy-crowdsec-bouncer@v0.14.1".into(),
        );
        let inst = installed_from(&caps);
        assert_eq!(inst.len(), 2);
        let cs = inst.iter().find(|i| i.path.contains("crowdsec")).unwrap();
        assert_eq!(cs.modules, vec!["crowdsec", "http.handlers.crowdsec"]);
        assert_eq!(cs.name, "CrowdSec bouncer");
        assert_eq!(
            inst.iter()
                .find(|i| i.path.contains("cloudflare"))
                .unwrap()
                .version,
            "v0.2.4"
        );
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// A real Caddy run by the supervisor with one site, a plugin manager
    /// wired to it (short time limits) and a stand-in download service that
    /// serves whatever `payload` holds.
    pub(crate) struct Harness {
        _dir: tempfile::TempDir,
        pub bin: PathBuf,
        pub m: Arc<Manager>,
        pub store: Arc<Store>,
        pub caps: Arc<RwLock<Capabilities>>,
        pub payload: Arc<Mutex<Vec<u8>>>,
        pub sent: Arc<Mutex<Vec<String>>>,
        pub caddyfile: PathBuf,
        stop: tokio::sync::watch::Sender<bool>,
        sup: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        pub async fn start(bin: &str) -> Harness {
            use crate::caddy::control::ProcessHandle;
            let bin = std::fs::canonicalize(bin).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let (admin_port, site_port) = (free_port(), free_port());
            let admin_addr = format!("127.0.0.1:{admin_port}");
            let managed = crate::supervisor::Managed {
                image_bin: bin.clone(),
                caddy_bin: dir.path().join("bin").join("caddy"),
                caddyfile: dir.path().join("Caddyfile"),
                admin_listen: admin_addr.clone(),
                admin_origins: vec![admin_addr.clone()],
                secrets_dir: dir.path().join("secrets"),
                log_dir: dir.path().join("logs"),
                data_dir: dir.path().join("data"),
                config_dir: dir.path().join("config"),
            };
            std::fs::write(
                &managed.caddyfile,
                format!(
                    "{{\n\tadmin {admin_addr} {{\n\t\torigins {admin_addr}\n\t}}\n}}\n:{site_port} {{\n\trespond \"ok\"\n}}\n"
                ),
            )
            .unwrap();
            managed.prepare().unwrap();

            let payload = Arc::new(Mutex::new(std::fs::read(&bin).unwrap()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let p = payload.clone();
            let app = axum::Router::new().route(
                "/api/download",
                axum::routing::get(move || {
                    let p = p.clone();
                    async move { p.lock().unwrap().clone() }
                }),
            );
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

            let admin = Arc::new(Admin::new(&admin_addr));
            let process = Arc::new(ProcessHandle::default());
            let (stop, stop_rx) = tokio::sync::watch::channel(false);
            let sup = tokio::spawn(managed.clone().run(admin.clone(), process.clone(), stop_rx));
            admin
                .wait_ready(Duration::from_secs(20))
                .await
                .expect("caddy up");

            let store = Arc::new(
                Store::open_memory(Arc::new(crate::secrets::SecretBox::new(b"k").unwrap()))
                    .unwrap(),
            );
            let runner = Arc::new(Runner::new(&managed.caddy_bin));
            let caps = Arc::new(RwLock::new(runner.capabilities().await.unwrap()));
            let pipeline = Arc::new(Pipeline::new(crate::apply::PipelineParts {
                store: store.clone(),
                validator: runner.clone(),
                admin: admin.clone(),
                health: Arc::new(crate::apply::DefaultHealth {
                    admin: admin.clone(),
                    probe_addrs: vec![],
                    settle: Duration::ZERO,
                }),
                secrets: None,
                env: crate::generator::Env {
                    admin_listen: admin_addr.clone(),
                    log_dir: crate::secrets::to_slash(&dir.path().join("logs")),
                    ..Default::default()
                },
                persist_path: dir.path().join("Caddyfile.generated"),
                caps: caps.clone(),
                notify: None,
            }));
            let sent = Arc::new(Mutex::new(Vec::<String>::new()));
            let s2 = sent.clone();
            let mut m = Manager::new(
                store.clone(),
                admin.clone(),
                caps.clone(),
                pipeline,
                managed.caddy_bin.clone(),
                String::new(),
                String::new(),
            )
            .with_recovery(
                process.clone(),
                vec![format!("127.0.0.1:{site_port}")],
                Some(Arc::new(move |e: &str, _t: &str, _m: &str| {
                    s2.lock().unwrap().push(e.to_string())
                })),
            );
            m.base_url = base;
            {
                let r = Arc::get_mut(&mut m.restarter).unwrap();
                r.timing.up = Duration::from_secs(10);
                r.timing.down = Duration::from_secs(5);
                r.timing.stop = Duration::from_secs(3);
            }
            Harness {
                bin,
                m: Arc::new(m),
                store,
                caps,
                payload,
                sent,
                caddyfile: managed.caddyfile.clone(),
                _dir: dir,
                stop,
                sup,
            }
        }

        pub async fn finish_job(&self) -> JobState {
            for _ in 0..1200 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if let Some(j) = self.m.current_job() {
                    if j.status != "running" {
                        return j;
                    }
                }
            }
            panic!("job did not finish");
        }

        pub async fn shutdown(self) {
            self.stop.send(true).unwrap();
            let _ = tokio::time::timeout(Duration::from_secs(30), self.sup).await;
        }
    }

    /// The whole update path against a real Caddy run by the supervisor: a
    /// good build is installed and serves; then a build that hangs instead
    /// of serving is installed, is killed, retried once and rolled back, and
    /// the previous build serves again, with a notification. Needs CADDY_BIN
    /// (and a Unix shell for the hanging stand-in).
    #[cfg(unix)]
    #[tokio::test]
    async fn update_rolls_back_a_build_that_hangs() {
        let Ok(bin) = std::env::var("CADDY_BIN") else {
            return;
        };
        let h = Harness::start(&bin).await;
        let (m, bin) = (h.m.clone(), h.bin.clone());
        // 1. A good build: installed, restarted, serving.
        m.start_install(vec![], vec![], "t".into()).unwrap();
        let j = h.finish_job().await;
        assert_eq!(j.status, "done", "{}", j.log.join("\n"));
        m.restarter
            .serving()
            .await
            .expect("serving after a good update");

        // 2. A build that passes every check but hangs instead of serving.
        let mut script = format!(
            "#!/bin/sh\nif [ \"$1\" = run ]; then exec sleep 600; fi\nexec '{}' \"$@\"\n",
            bin.display()
        );
        script.push_str(&format!("# {}\n", "x".repeat(2 << 20)));
        *h.payload.lock().unwrap() = script.into_bytes();
        m.start_install(vec![], vec![], "t".into()).unwrap();
        let j = h.finish_job().await;
        let log = j.log.join("\n");
        assert_eq!(j.status, "failed", "{log}");
        assert!(
            j.error.contains("Rolled back to the previous build"),
            "{}",
            j.error
        );
        assert!(log.contains("restarting once more"), "{log}");
        m.restarter
            .serving()
            .await
            .expect("the previous build serves again");
        assert!(
            m.custom_path().exists(),
            "the previous build is back in place"
        );
        assert!(m.bin_dir().join("caddy-custom.failed").exists());
        assert!(
            Runner::new(m.custom_path()).version().await.is_ok()
                && std::fs::metadata(m.custom_path()).unwrap().len() > 10 << 20,
            "the real binary, not the stand-in"
        );
        assert_eq!(*h.sent.lock().unwrap(), vec![notify::EVENT_CADDY_RECOVERED]);

        h.shutdown().await;
    }
}
