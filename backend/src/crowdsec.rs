//! Built-in CrowdSec: the panel installs, configures, runs and updates the
//! CrowdSec security engine next to Caddy, so blocking known attackers needs
//! no extra container.
//!
//! The programs (`crowdsec`, `cscli`) come from CrowdSec's official image on
//! Docker Hub, for this machine's architecture; CrowdSec publishes no other
//! builds that run on the Alpine-based Caddymium image. They live with their
//! configuration, database and rules under `<data>/crowdsec`, so they survive
//! image updates. The engine reads Caddy's access logs, its local API listens
//! on loopback only, and Caddy's bouncer plugin asks it which IPs to block.
//!
//! Updates come in three layers: CrowdSec itself pulls the community
//! blocklist continuously; the panel refreshes the detection rules (the
//! "hub") daily; and new CrowdSec releases are installed the same way Caddy
//! updates are (automatically when automatic updates are on), keeping the
//! previous programs for a rollback.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, watch};

use crate::store::Store;

/// The engine's local API (loopback only; the panel itself uses :8080).
pub const LAPI_LISTEN: &str = "127.0.0.1:8081";
pub const LAPI_URL: &str = "http://127.0.0.1:8081";
/// Named secret holding the bouncer key Caddy uses.
pub const BOUNCER_SECRET: &str = "crowdsec_builtin_key";
const BOUNCER_NAME: &str = "caddymium-caddy";
const MACHINE_NAME: &str = "caddymium";
const IMAGE: &str = "crowdsecurity/crowdsec";
const STATE_KEY: &str = "crowdsec_builtin_state";
const LATEST_KEY: &str = "crowdsec_latest";

/// Detection rules installed on setup.
const COLLECTIONS: &[&str] = &["crowdsecurity/caddy"];
/// syslog-logs also holds the "non-syslog" stage that tags plain log files
/// with their acquisition type (caddy), which the Caddy parser keys on; the
/// official image gets it from its default "linux" collection.
const PARSERS: &[&str] = &[
    "crowdsecurity/syslog-logs",
    "crowdsecurity/whitelists",
    "crowdsecurity/dateparse-enrich",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    installed_version: String,
    configured: bool,
    hub_updated_at: Option<DateTime<Utc>>,
    installed_at: Option<DateTime<Utc>>,
    last_auto: Option<crate::updates::AutoResult>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Latest {
    version: String,
    checked_at: Option<DateTime<Utc>>,
}

/// A running or finished install/update.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Job {
    pub kind: String,
    /// running, done, failed
    pub status: String,
    pub log: Vec<String>,
    pub error: String,
    pub started: Option<DateTime<Utc>>,
    pub finished: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub supported: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    pub enabled: bool,
    pub installed_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub configured: bool,
    pub running: bool,
    pub started_at: Option<DateTime<Utc>>,
    pub restarts: u32,
    pub hub_updated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<Job>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_auto: Option<crate::updates::AutoResult>,
    pub log_tail: Vec<String>,
}

/// One active ban.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Decision {
    pub id: i64,
    pub value: String,
    pub scope: String,
    pub r#type: String,
    pub reason: String,
    pub origin: String,
    pub until: String,
}

#[derive(Default)]
struct Run {
    running: bool,
    started_at: Option<DateTime<Utc>>,
    restarts: u32,
    log: VecDeque<String>,
}

pub struct Manager {
    pub store: Arc<Store>,
    /// <data>/crowdsec
    pub dir: PathBuf,
    /// Where Caddy writes access logs (access-site-N.log).
    pub log_dir: String,
    pub github_token: String,
    /// Caddy runs in this container (all-in-one); the bouncer reaches the
    /// engine on loopback only then.
    managed: bool,
    job: Mutex<Option<Job>>,
    run: Mutex<Run>,
    /// Wakes the supervisor (enable, disable, restart).
    kick: Notify,
}

fn go_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "arm" => "arm",
        other => other,
    }
}

/// A version string like "v1.8.1" from `crowdsec -version` output.
fn parse_version(out: &str) -> String {
    for line in out.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("version:") {
            let v = v.trim();
            let v = v.split('-').next().unwrap_or(v);
            return if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            };
        }
    }
    String::new()
}

impl Manager {
    pub fn new(
        store: Arc<Store>,
        dir: PathBuf,
        log_dir: String,
        github_token: String,
        managed: bool,
    ) -> Self {
        Manager {
            store,
            dir,
            log_dir,
            github_token,
            managed,
            job: Mutex::new(None),
            run: Mutex::new(Run::default()),
            kick: Notify::new(),
        }
    }

    fn supported(&self) -> std::result::Result<(), String> {
        if !(cfg!(target_os = "linux") && matches!(go_arch(), "amd64" | "arm64")) {
            return Err("Built-in CrowdSec runs in the Linux container (amd64 or arm64)".into());
        }
        if !self.managed {
            return Err("Built-in CrowdSec needs the all-in-one image (Caddy in the same container); with Caddy in its own container, use an external CrowdSec".into());
        }
        Ok(())
    }

    fn bin(&self, name: &str) -> PathBuf {
        self.dir.join("bin").join(name)
    }
    fn etc(&self) -> PathBuf {
        self.dir.join("etc")
    }
    fn config_path(&self) -> PathBuf {
        self.etc().join("config.yaml")
    }

    fn state(&self) -> State {
        self.store.kv_get_json(STATE_KEY).unwrap_or_default()
    }
    fn save_state(&self, s: &State) {
        let _ = self.store.kv_set_json(STATE_KEY, s);
    }

    pub fn turned_on(&self) -> bool {
        self.store.settings().crowdsec.builtin
    }

    fn job_log(&self, line: &str) {
        tracing::info!(target: "crowdsec", "{line}");
        if let Some(j) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            j.log
                .push(format!("{} {line}", Utc::now().format("%H:%M:%S")));
        }
    }

    fn start_job(&self, kind: &str) -> Result<()> {
        let mut j = self.job.lock().unwrap_or_else(|e| e.into_inner());
        if j.as_ref().is_some_and(|j| j.status == "running") {
            return Err(anyhow!("another CrowdSec operation is still running"));
        }
        *j = Some(Job {
            kind: kind.into(),
            status: "running".into(),
            started: Some(Utc::now()),
            ..Default::default()
        });
        Ok(())
    }

    fn finish_job(&self, r: &Result<()>) {
        if let Some(j) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            j.finished = Some(Utc::now());
            match r {
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

    pub fn current_job(&self) -> Option<Job> {
        self.job.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    // ---------------------------------------------------------------- status

    pub async fn status(&self) -> Status {
        let mut st = self.state();
        // Cached only: the page never waits on GitHub (chores_loop refreshes).
        let latest = self
            .store
            .kv_get_json::<Latest>(LATEST_KEY)
            .map(|l| l.version)
            .unwrap_or_default();
        // A failed automatic update that a later install made moot is cleared.
        if st
            .last_auto
            .as_ref()
            .is_some_and(|a| crate::updates::failure_is_stale(a, &st.installed_version, &latest))
        {
            st.last_auto = None;
            let mut fresh = self.state();
            fresh.last_auto = None;
            self.save_state(&fresh);
        }
        let (supported, reason) = match self.supported() {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e),
        };
        let run = self.run.lock().unwrap_or_else(|e| e.into_inner());
        Status {
            supported,
            reason,
            enabled: self.turned_on(),
            installed_version: st.installed_version.clone(),
            update_available: newer(&latest, &st.installed_version),
            latest_version: latest,
            configured: st.configured,
            running: run.running,
            started_at: run.started_at,
            restarts: run.restarts,
            hub_updated_at: st.hub_updated_at,
            job: self.current_job(),
            last_auto: st.last_auto,
            log_tail: run.log.iter().rev().take(60).rev().cloned().collect(),
        }
    }

    /// The newest CrowdSec release tag (cached 6 hours unless `force`).
    pub async fn latest_version(&self, force: bool) -> String {
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
        let mut req = match crate::httpc::builder()
            .timeout(Duration::from_secs(15))
            .build()
        {
            Ok(c) => c.get("https://api.github.com/repos/crowdsecurity/crowdsec/releases/latest"),
            Err(_) => return String::new(),
        };
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
                if !v.is_empty() {
                    let _ = self.store.kv_set_json(
                        LATEST_KEY,
                        &Latest {
                            version: v.clone(),
                            checked_at: Some(Utc::now()),
                        },
                    );
                }
                v
            }
            Err(e) => {
                tracing::debug!(err = %e, "latest crowdsec version unavailable");
                cached.map(|c| c.version).unwrap_or_default()
            }
        }
    }

    // ---------------------------------------------------------------- install

    /// Turns the built-in engine on: installs and sets it up when needed,
    /// then starts it. Runs in the background; see `current_job`.
    pub fn start_enable(self: &Arc<Self>) -> Result<()> {
        self.supported().map_err(|e| anyhow!(e))?;
        self.start_job("enable")?;
        let m = self.clone();
        tokio::spawn(async move {
            let r = m.enable().await;
            m.finish_job(&r);
        });
        Ok(())
    }

    async fn enable(&self) -> Result<()> {
        self.store.update_settings(|s| s.crowdsec.builtin = true)?;
        if !self.bin("crowdsec").is_file() || !self.bin("cscli").is_file() {
            let v = self.latest_version(true).await;
            if v.is_empty() {
                return Err(anyhow!("could not find the latest CrowdSec release"));
            }
            self.install(&v).await?;
        }
        self.setup().await?;
        self.job_log("Starting CrowdSec…");
        self.kick.notify_one();
        Ok(())
    }

    /// Stops the engine and turns it off (files are kept).
    pub fn disable(&self) -> Result<()> {
        self.store.update_settings(|s| s.crowdsec.builtin = false)?;
        self.kick.notify_one();
        Ok(())
    }

    /// Installs the newest release in the background.
    pub fn start_update(self: &Arc<Self>, user: String) -> Result<()> {
        self.supported().map_err(|e| anyhow!(e))?;
        self.start_job("update")?;
        let m = self.clone();
        tokio::spawn(async move {
            let r = m.update(&user).await.map(|_| ());
            m.finish_job(&r);
        });
        Ok(())
    }

    async fn update(&self, user: &str) -> Result<String> {
        let v = self.latest_version(true).await;
        if v.is_empty() {
            return Err(anyhow!("could not find the latest CrowdSec release"));
        }
        let from = self.state().installed_version;
        self.install(&v).await?;
        self.restart();
        self.store
            .audit(user, "crowdsec.update", &v, &format!("from {from}"), "");
        Ok(v)
    }

    /// Refreshes the detection rules in the background.
    pub fn start_hub_update(self: &Arc<Self>) -> Result<()> {
        self.start_job("rules")?;
        let m = self.clone();
        tokio::spawn(async move {
            let r = m.hub_update().await;
            m.finish_job(&r);
        });
        Ok(())
    }

    /// Downloads CrowdSec `version` from the official image and swaps it in,
    /// keeping the previous programs as `.prev`.
    async fn install(&self, version: &str) -> Result<()> {
        let arch = go_arch();
        self.job_log(&format!(
            "Downloading CrowdSec {version} for linux/{arch} from the official image…"
        ));
        let tmp = self.dir.join("tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;
        let r = self.fetch_binaries(version, arch, &tmp).await;
        if let Err(e) = r {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
        #[cfg(unix)]
        for n in ["crowdsec", "cscli"] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(tmp.join(n), std::fs::Permissions::from_mode(0o755))?;
        }
        let out = run_out(
            &tmp.join("crowdsec"),
            &["-version"],
            Duration::from_secs(20),
        )
        .await
        .context("the downloaded CrowdSec does not run")?;
        let got = parse_version(&out);
        self.job_log(&format!(
            "Downloaded build reports {}",
            if got.is_empty() {
                "an unknown version"
            } else {
                &got
            }
        ));
        let bin = self.dir.join("bin");
        std::fs::create_dir_all(&bin)?;
        for n in ["crowdsec", "cscli"] {
            let cur = bin.join(n);
            if cur.exists() {
                let prev = bin.join(format!("{n}.prev"));
                let _ = std::fs::remove_file(&prev);
                std::fs::rename(&cur, &prev)?;
            }
            std::fs::rename(tmp.join(n), &cur)?;
        }
        // The log patterns belong to this version of the engine.
        std::fs::create_dir_all(self.etc())?;
        let patterns = self.etc().join("patterns");
        let _ = std::fs::remove_dir_all(&patterns);
        std::fs::rename(tmp.join("patterns"), &patterns).context("install the log patterns")?;
        let _ = std::fs::remove_dir_all(&tmp);
        let mut st = self.state();
        st.installed_version = if got.is_empty() {
            version.to_string()
        } else {
            got
        };
        st.installed_at = Some(Utc::now());
        self.save_state(&st);
        self.job_log(&format!("Installed CrowdSec {}", st.installed_version));
        Ok(())
    }

    /// Finds usr/local/bin/{crowdsec,cscli} in the image's layers (largest
    /// first; they sit in the biggest one) and extracts them into `dst`.
    async fn fetch_binaries(&self, version: &str, arch: &str, dst: &Path) -> Result<()> {
        let client = crate::httpc::builder()
            .timeout(Duration::from_secs(900))
            .build()?;
        #[derive(Deserialize)]
        struct Tok {
            token: String,
        }
        let tok = client
            .get(format!(
                "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{IMAGE}:pull"
            ))
            .send()
            .await?
            .error_for_status()?
            .json::<Tok>()
            .await?
            .token;
        let reg = format!("https://registry-1.docker.io/v2/{IMAGE}");
        let get_json = |url: String, accept: &'static str| {
            let client = client.clone();
            let tok = tok.clone();
            async move {
                client
                    .get(url)
                    .bearer_auth(tok)
                    .header("Accept", accept)
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<serde_json::Value>()
                    .await
            }
        };
        let index = get_json(
            format!("{reg}/manifests/{version}"),
            "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json",
        )
        .await
        .with_context(|| format!("image {IMAGE}:{version} not found"))?;
        let digest = index["manifests"]
            .as_array()
            .and_then(|ms| {
                ms.iter().find(|m| {
                    m["platform"]["architecture"] == arch && m["platform"]["os"] == "linux"
                })
            })
            .and_then(|m| m["digest"].as_str())
            .ok_or_else(|| anyhow!("no linux/{arch} build of CrowdSec {version}"))?
            .to_string();
        let manifest = get_json(
            format!("{reg}/manifests/{digest}"),
            "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json",
        )
        .await?;
        let layers: Vec<(i64, String)> = manifest["layers"]
            .as_array()
            .map(|ls| {
                ls.iter()
                    .filter_map(|l| Some((l["size"].as_i64()?, l["digest"].as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        // The programs sit in the biggest layer; the log patterns that ship
        // with the default config in a small one. Fetch biggest-first until
        // the programs turn up, then the small layers until the patterns do.
        let mut order = layers.clone();
        order.sort_by_key(|l| std::cmp::Reverse(l.0));
        let mut small = layers;
        small.sort_by_key(|l| l.0);
        small.retain(|l| l.0 <= 8 << 20);
        let mut seen = std::collections::HashSet::new();
        let (mut bins, mut patterns) = (false, false);
        for (size, d) in order.into_iter().chain(small) {
            if bins && patterns {
                break;
            }
            if !seen.insert(d.clone()) || (bins && size > 8 << 20) {
                continue;
            }
            if size > 1 << 20 {
                self.job_log(&format!("Fetching a {} MB layer…", size >> 20));
            }
            let blob = dst.join("layer.tgz");
            let mut resp = client
                .get(format!("{reg}/blobs/{d}"))
                .bearer_auth(&tok)
                .send()
                .await?
                .error_for_status()?;
            let mut f = tokio::fs::File::create(&blob).await?;
            while let Some(chunk) = resp.chunk().await? {
                tokio::io::AsyncWriteExt::write_all(&mut f, &chunk).await?;
            }
            tokio::io::AsyncWriteExt::flush(&mut f).await?;
            drop(f);
            let dst2 = dst.to_path_buf();
            let (b, p) =
                tokio::task::spawn_blocking(move || extract_files(&dst2.join("layer.tgz"), &dst2))
                    .await??;
            let _ = std::fs::remove_file(&blob);
            bins |= b;
            patterns |= p;
        }
        if !bins {
            return Err(anyhow!("the CrowdSec programs were not found in the image"));
        }
        if !patterns {
            return Err(anyhow!(
                "CrowdSec's log patterns were not found in the image"
            ));
        }
        Ok(())
    }

    // ---------------------------------------------------------------- setup

    fn write_config(&self) -> Result<()> {
        let d = self.dir.to_string_lossy().replace('\\', "/");
        for sub in ["etc", "etc/hub", "etc/notifications", "data", "plugins"] {
            std::fs::create_dir_all(self.dir.join(sub))?;
        }
        let config = format!(
            r#"# Written by Caddymium; changes here are overwritten.
common:
  log_media: stdout
  log_level: info
config_paths:
  config_dir: {d}/etc/
  data_dir: {d}/data/
  simulation_path: {d}/etc/simulation.yaml
  hub_dir: {d}/etc/hub/
  index_path: {d}/etc/hub/.index.json
  notification_dir: {d}/etc/notifications/
  plugin_dir: {d}/plugins/
crowdsec_service:
  acquisition_path: {d}/etc/acquis.yaml
  parser_routines: 1
plugin_config:
  user: nobody
  group: nobody
cscli:
  output: human
db_config:
  log_level: info
  type: sqlite
  db_path: {d}/data/crowdsec.db
  use_wal: true
  flush:
    max_items: 5000
    max_age: 7d
api:
  client:
    insecure_skip_verify: false
    credentials_path: {d}/etc/local_api_credentials.yaml
  server:
    log_level: warning
    listen_uri: {LAPI_LISTEN}
    profiles_path: {d}/etc/profiles.yaml
    trusted_ips:
      - 127.0.0.1
      - ::1
    online_client:
      credentials_path: {d}/etc/online_api_credentials.yaml
prometheus:
  enabled: true
  level: full
  listen_addr: 127.0.0.1
  listen_port: 6060
"#
        );
        std::fs::write(self.config_path(), config)?;
        let logs = self.log_dir.trim_end_matches('/');
        std::fs::write(
            self.etc().join("acquis.yaml"),
            format!(
                "# Written by Caddymium: Caddy's access logs.\nfilenames:\n  - {logs}/access-*.log\nlabels:\n  type: caddy\n"
            ),
        )?;
        std::fs::write(
            self.etc().join("profiles.yaml"),
            "name: default_ip_remediation\nfilters:\n  - Alert.Remediation == true && Alert.GetScope() == \"Ip\"\ndecisions:\n  - type: ban\n    duration: 4h\non_success: break\n---\nname: default_range_remediation\nfilters:\n  - Alert.Remediation == true && Alert.GetScope() == \"Range\"\ndecisions:\n  - type: ban\n    duration: 4h\non_success: break\n",
        )?;
        let sim = self.etc().join("simulation.yaml");
        if !sim.exists() {
            std::fs::write(&sim, "simulation: off\n")?;
        }
        for f in ["local_api_credentials.yaml", "online_api_credentials.yaml"] {
            let p = self.etc().join(f);
            if !p.exists() {
                std::fs::write(&p, "")?;
            }
        }
        Ok(())
    }

    async fn cscli(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let cfg = self.config_path().to_string_lossy().to_string();
        let mut all = vec!["-c", cfg.as_str()];
        all.extend_from_slice(args);
        run_out(&self.bin("cscli"), &all, timeout).await
    }

    /// First-run setup (idempotent): config, rules, the panel's machine
    /// account, Central API registration (community blocklist) and the
    /// bouncer key for Caddy.
    async fn setup(&self) -> Result<()> {
        self.job_log("Writing the configuration…");
        self.write_config()?;
        let mut st = self.state();
        let long = Duration::from_secs(300);
        self.job_log("Downloading the detection rules…");
        self.cscli(&["hub", "update"], long).await?;
        let mut install = vec!["collections", "install"];
        install.extend_from_slice(COLLECTIONS);
        self.cscli(&install, long).await?;
        let mut install = vec!["parsers", "install"];
        install.extend_from_slice(PARSERS);
        self.cscli(&install, long).await?;
        st.hub_updated_at = Some(Utc::now());
        if !st.configured {
            self.job_log("Creating the engine's account…");
            let creds = self.etc().join("local_api_credentials.yaml");
            let creds_s = creds.to_string_lossy().to_string();
            self.cscli(
                &[
                    "machines",
                    "add",
                    MACHINE_NAME,
                    "--auto",
                    "--force",
                    "-f",
                    &creds_s,
                    "--url",
                    LAPI_URL,
                ],
                Duration::from_secs(60),
            )
            .await?;
            self.job_log("Registering for the community blocklist…");
            let online = self.etc().join("online_api_credentials.yaml");
            let online_s = online.to_string_lossy().to_string();
            if let Err(e) = self
                .cscli(
                    &["capi", "register", "-f", &online_s],
                    Duration::from_secs(60),
                )
                .await
            {
                // Works without it; only the community blocklist is missing.
                self.job_log(&format!(
                    "Community blocklist registration failed (will work without it): {e:#}"
                ));
            }
        }
        if self.store.secret(BOUNCER_SECRET).is_none() || !st.configured {
            self.job_log("Creating Caddy's bouncer key…");
            let key = crate::auth::random_token(32);
            let _ = self
                .cscli(
                    &["bouncers", "delete", BOUNCER_NAME],
                    Duration::from_secs(30),
                )
                .await;
            self.cscli(
                &["bouncers", "add", BOUNCER_NAME, "-k", &key],
                Duration::from_secs(30),
            )
            .await?;
            self.store.set_secret(BOUNCER_SECRET, &key)?;
        }
        st.configured = true;
        self.save_state(&st);
        Ok(())
    }

    /// Updates the detection rules and restarts the engine.
    async fn hub_update(&self) -> Result<()> {
        if !self.state().configured {
            return Err(anyhow!("CrowdSec is not set up yet"));
        }
        let long = Duration::from_secs(300);
        self.job_log("Updating the detection rules…");
        self.cscli(&["hub", "update"], long).await?;
        let out = self.cscli(&["hub", "upgrade"], long).await?;
        let mut st = self.state();
        st.hub_updated_at = Some(Utc::now());
        self.save_state(&st);
        if !out.trim().is_empty() {
            self.job_log(out.lines().last().unwrap_or("").trim());
        }
        self.restart();
        Ok(())
    }

    /// Restarts the engine (it is started again by the supervisor).
    pub fn restart(&self) {
        let mut r = self.run.lock().unwrap_or_else(|e| e.into_inner());
        r.log.push_back("(restart requested)".into());
        drop(r);
        self.kick.notify_one();
    }

    // ---------------------------------------------------------------- bans

    pub async fn decisions(&self) -> Result<Vec<Decision>> {
        let out = self
            .cscli(
                &["decisions", "list", "-o", "json"],
                Duration::from_secs(30),
            )
            .await?;
        Ok(parse_decisions(&out))
    }

    pub async fn unban(&self, id: i64, user: &str) -> Result<()> {
        self.cscli(
            &["decisions", "delete", "--id", &id.to_string()],
            Duration::from_secs(30),
        )
        .await?;
        self.store
            .audit(user, "crowdsec.unban", &id.to_string(), "", "");
        Ok(())
    }

    pub async fn ban(&self, ip: &str, duration: &str, user: &str) -> Result<()> {
        if ip.parse::<std::net::IpAddr>().is_err() && ip.parse::<ipnet::IpNet>().is_err() {
            return Err(anyhow!("{ip:?} is not an IP address or range"));
        }
        let scope = if ip.contains('/') { "--range" } else { "--ip" };
        let d = if duration.trim().is_empty() {
            "24h"
        } else {
            duration.trim()
        };
        self.cscli(
            &[
                "decisions",
                "add",
                scope,
                ip,
                "--duration",
                d,
                "--reason",
                "banned from Caddymium",
            ],
            Duration::from_secs(30),
        )
        .await?;
        self.store.audit(user, "crowdsec.ban", ip, d, "");
        Ok(())
    }

    // ---------------------------------------------------------------- supervisor

    /// Runs the engine while built-in CrowdSec is enabled and set up,
    /// restarting it when it exits; refreshes rules daily and installs new
    /// releases when automatic updates allow it.
    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        if self.supported().is_err() {
            return;
        }
        let mut backoff = Duration::from_secs(2);
        loop {
            if *shutdown.borrow() {
                return;
            }
            let ready =
                self.turned_on() && self.state().configured && self.bin("crowdsec").is_file();
            if !ready {
                tokio::select! {
                    _ = self.kick.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                    _ = shutdown.changed() => return,
                }
                continue;
            }
            self.maintain().await;
            let started = std::time::Instant::now();
            let mut cmd = tokio::process::Command::new(self.bin("crowdsec"));
            cmd.args(["-c", &self.config_path().to_string_lossy()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    self.push_log(&format!("cannot start crowdsec: {e}"));
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                        _ = shutdown.changed() => return,
                    }
                    continue;
                }
            };
            for pipe in [
                child
                    .stdout
                    .take()
                    .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
                child
                    .stderr
                    .take()
                    .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            ]
            .into_iter()
            .flatten()
            {
                let m = self.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncBufReadExt;
                    let mut lines = tokio::io::BufReader::new(pipe).lines();
                    while let Ok(Some(l)) = lines.next_line().await {
                        m.push_log(&l);
                    }
                });
            }
            {
                let mut r = self.run.lock().unwrap_or_else(|e| e.into_inner());
                r.running = true;
                r.started_at = Some(Utc::now());
            }
            tracing::info!("crowdsec started");
            // CrowdSec only reads log files that exist when it starts, so a
            // new site's access log needs a restart to be watched.
            let logs_at_start = self.log_files();
            let stop = loop {
                tokio::select! {
                    s = child.wait() => {
                        self.push_log(&format!("crowdsec exited: {s:?}"));
                        break false;
                    }
                    _ = self.kick.notified() => break true,
                    _ = shutdown.changed() => break true,
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {
                        if self.log_files() != logs_at_start {
                            self.push_log("(new access log found; restarting to read it)");
                            break true;
                        }
                    }
                }
            };
            if stop {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            {
                let mut r = self.run.lock().unwrap_or_else(|e| e.into_inner());
                r.running = false;
                if !stop {
                    r.restarts += 1;
                }
            }
            if *shutdown.borrow() {
                return;
            }
            if stop {
                backoff = Duration::from_secs(2);
                continue;
            }
            backoff = if started.elapsed() > Duration::from_secs(300) {
                Duration::from_secs(2)
            } else {
                (backoff * 2).min(Duration::from_secs(120))
            };
            tokio::select! {
                _ = tokio::time::sleep(backoff) => {}
                _ = self.kick.notified() => {}
                _ = shutdown.changed() => return,
            }
        }
    }

    /// Caddy's access logs (what the acquisition glob matches), sorted.
    fn log_files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.log_dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n.starts_with("access-") && n.ends_with(".log"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    fn push_log(&self, line: &str) {
        let mut r = self.run.lock().unwrap_or_else(|e| e.into_inner());
        r.log.push_back(line.chars().take(400).collect());
        while r.log.len() > 300 {
            r.log.pop_front();
        }
    }

    /// Daily chores before (re)starting: rules refresh, automatic update.
    async fn maintain(&self) {
        let st = self.state();
        if st
            .hub_updated_at
            .is_none_or(|t| Utc::now() - t > chrono::Duration::hours(24))
        {
            if let Ok(()) = self.start_job("rules") {
                let r = async {
                    let long = Duration::from_secs(300);
                    self.cscli(&["hub", "update"], long).await?;
                    self.cscli(&["hub", "upgrade"], long).await?;
                    let mut st = self.state();
                    st.hub_updated_at = Some(Utc::now());
                    self.save_state(&st);
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                if let Err(e) = &r {
                    tracing::warn!(err = %e, "crowdsec rules refresh failed");
                }
                self.finish_job(&r);
            }
        }
        let u = self.store.settings().updates;
        if !u.auto {
            return;
        }
        let latest = self.latest_version(false).await;
        let st = self.state();
        let failed_before = st
            .last_auto
            .as_ref()
            .is_some_and(|a| !a.ok && a.to == latest);
        if newer(&latest, &st.installed_version)
            && !failed_before
            && crate::updates::in_window(
                &u.window_start,
                &u.window_end,
                chrono::Local::now().time(),
            )
            && self.start_job("update").is_ok()
        {
            let from = st.installed_version.clone();
            let r = self.install(&latest).await;
            let mut st = self.state();
            st.last_auto = Some(crate::updates::AutoResult {
                at: Utc::now(),
                from,
                to: latest.clone(),
                ok: r.is_ok(),
                error: r
                    .as_ref()
                    .err()
                    .map(|e| format!("{e:#}"))
                    .unwrap_or_default(),
            });
            self.save_state(&st);
            self.finish_job(&r);
        }
    }

    /// Ticks the supervisor so it re-checks the daily chores.
    pub async fn chores_loop(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let mut first = true;
        loop {
            let wait = if first {
                Duration::from_secs(90)
            } else {
                Duration::from_secs(6 * 3600)
            };
            first = false;
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = shutdown.changed() => return,
            }
            if self.supported().is_ok() {
                self.latest_version(false).await;
            }
            let st = self.state();
            let due = st
                .hub_updated_at
                .is_none_or(|t| Utc::now() - t > chrono::Duration::hours(24));
            if self.turned_on() && st.configured && due {
                self.restart(); // maintain() runs before the next start
            }
        }
    }
}

fn newer(latest: &str, installed: &str) -> bool {
    let l = crate::registry::parse_version(latest);
    let i = crate::registry::parse_version(installed);
    l.valid && i.valid && l.compare(&i) == std::cmp::Ordering::Greater
}

/// Extracts usr/local/bin/{crowdsec,cscli} and the default config's log
/// patterns (staging/etc/crowdsec/patterns/*) from a gzipped layer tarball
/// into `dst` (patterns into `dst/patterns`). Returns (both programs found,
/// any pattern found).
fn extract_files(layer: &Path, dst: &Path) -> Result<(bool, bool)> {
    let f = std::fs::File::open(layer)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(f)));
    let (mut bins, mut patterns) = (0, false);
    for entry in ar.entries()? {
        let mut e = entry?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        let path = e
            .path()?
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        let out = match path.as_str() {
            "usr/local/bin/crowdsec" => dst.join("crowdsec"),
            "usr/local/bin/cscli" => dst.join("cscli"),
            p => match p.strip_prefix("staging/etc/crowdsec/patterns/") {
                // plain file names only (no nested or escaping paths)
                Some(n) if !n.is_empty() && !n.contains('/') && !n.contains("..") => {
                    std::fs::create_dir_all(dst.join("patterns"))?;
                    dst.join("patterns").join(n)
                }
                _ => continue,
            },
        };
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut e, &mut w)?;
        if out.parent() == Some(dst) {
            bins += 1;
        } else {
            patterns = true;
        }
    }
    Ok((bins == 2, patterns))
}

/// Runs a program and returns stdout; fails with stderr on a non-zero exit.
async fn run_out(bin: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let out = tokio::time::timeout(
        timeout,
        tokio::process::Command::new(bin)
            .args(args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow!("{} timed out", bin.display()))?
    .with_context(|| format!("run {}", bin.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err
            .lines()
            .chain(stdout.lines())
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        return Err(anyhow!(
            "{} {} failed: {msg}",
            bin.file_name().unwrap_or_default().to_string_lossy(),
            args.iter()
                .find(|a| !a.starts_with('-') && !a.contains('/'))
                .unwrap_or(&"")
        ));
    }
    Ok(stdout)
}

/// Parses `cscli decisions list -o json` (alerts, each with decisions).
fn parse_decisions(json: &str) -> Vec<Decision> {
    let v: serde_json::Value = match serde_json::from_str(json.trim()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for alert in v.as_array().into_iter().flatten() {
        let scenario = alert["scenario"].as_str().unwrap_or("").to_string();
        for d in alert["decisions"].as_array().into_iter().flatten() {
            out.push(Decision {
                id: d["id"].as_i64().unwrap_or(0),
                value: d["value"].as_str().unwrap_or("").to_string(),
                scope: d["scope"].as_str().unwrap_or("").to_string(),
                r#type: d["type"].as_str().unwrap_or("").to_string(),
                reason: d["scenario"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| scenario.clone()),
                origin: d["origin"].as_str().unwrap_or("").to_string(),
                until: d["until"]
                    .as_str()
                    .or_else(|| d["duration"].as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("version: v1.8.1-0123abcd\nCodename: alphaga\n"),
            "v1.8.1"
        );
        assert_eq!(parse_version("version: 1.8.1\n"), "v1.8.1");
        assert_eq!(parse_version("nothing"), "");
        assert!(newer("v1.8.2", "v1.8.1"));
        assert!(!newer("v1.8.1", "v1.8.1"));
        assert!(!newer("", "v1.8.1"));
    }

    #[test]
    fn decisions_json() {
        let j = r#"[{"id":7,"scenario":"crowdsecurity/http-probing","decisions":[
            {"id":42,"origin":"crowdsec","scenario":"crowdsecurity/http-probing","scope":"Ip","type":"ban","value":"203.0.113.9","duration":"3h59m"}]},
            {"id":8,"scenario":"manual","decisions":[{"id":43,"origin":"cscli","scenario":"","scope":"Ip","type":"ban","value":"198.51.100.1","until":"2026-10-14T00:00:00Z"}]}]"#;
        let d = parse_decisions(j);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].id, 43);
        assert_eq!(d[0].reason, "manual", "falls back to the alert's scenario");
        assert_eq!(d[1].value, "203.0.113.9");
        assert_eq!(d[1].until, "3h59m");
        assert!(parse_decisions("null").is_empty());
    }

    /// Downloads, sets up and runs the real engine, feeds it attack traffic
    /// in Caddy's access-log format and checks that it bans the attacker.
    /// Linux only; set CROWDSEC_E2E=1 (downloads ~85 MB).
    #[tokio::test]
    async fn end_to_end() {
        if std::env::var("CROWDSEC_E2E").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let store = Arc::new(
            Store::open_memory(Arc::new(crate::secrets::SecretBox::new(b"k").unwrap())).unwrap(),
        );
        let m = Arc::new(Manager::new(
            store.clone(),
            dir.path().join("crowdsec"),
            logs.to_string_lossy().to_string(),
            String::new(),
            true,
        ));
        let (tx, rx) = watch::channel(false);
        let sup = tokio::spawn(m.clone().run(rx));

        m.start_enable().unwrap();
        for _ in 0..600 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if m.current_job().is_some_and(|j| j.status != "running") {
                break;
            }
        }
        let job = m.current_job().unwrap();
        assert_eq!(
            job.status,
            "done",
            "enable failed: {}\n{}",
            job.error,
            job.log.join("\n")
        );
        assert!(store.settings().crowdsec.builtin);
        assert!(store.secret(BOUNCER_SECRET).is_some(), "bouncer key stored");
        let st = m.status().await;
        assert!(st.installed_version.starts_with('v'), "{st:?}");

        // Wait until the local API answers.
        let mut up = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if m.cscli(&["lapi", "status"], Duration::from_secs(10))
                .await
                .is_ok()
            {
                up = true;
                break;
            }
        }
        assert!(
            up,
            "local API never came up:\n{}",
            m.status().await.log_tail.join("\n")
        );
        let bouncers = m
            .cscli(&["bouncers", "list", "-o", "json"], Duration::from_secs(10))
            .await
            .unwrap();
        assert!(bouncers.contains(BOUNCER_NAME), "{bouncers}");

        // Manual ban and unban.
        m.ban("198.51.100.7", "1h", "test").await.unwrap();
        let d = m.decisions().await.unwrap();
        let ban = d
            .iter()
            .find(|d| d.value == "198.51.100.7")
            .expect("manual ban listed");
        m.unban(ban.id, "test").await.unwrap();
        assert!(
            !m.decisions()
                .await
                .unwrap()
                .iter()
                .any(|d| d.value == "198.51.100.7")
        );

        // An attacker probing for secrets, in Caddy's JSON access-log format.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let log = logs.join("access-site-1.log");
        let mut lines = String::new();
        for (i, path) in [
            "/.env",
            "/.git/config",
            "/wp-login.php",
            "/.aws/credentials",
            "/config.php.bak",
            "/backup.sql",
            "/.htpasswd",
            "/admin/.env",
            "/phpinfo.php",
            "/server-status",
        ]
        .iter()
        .cycle()
        .take(40)
        .enumerate()
        {
            let ts = Utc::now().timestamp_micros() as f64 / 1e6 + i as f64 * 0.01;
            lines.push_str(&format!(
                r#"{{"level":"info","ts":{ts},"logger":"http.log.access.log0","msg":"handled request","request":{{"remote_ip":"203.0.113.50","remote_port":"40000","client_ip":"203.0.113.50","proto":"HTTP/1.1","method":"GET","host":"app.example.com","uri":"{path}?x={i}","headers":{{"User-Agent":["Mozilla/5.0 (X11; Linux x86_64)"]}},"tls":{{}}}},"bytes_read":0,"user_id":"","duration":0.001,"size":0,"status":404,"resp_headers":{{}}}}"#
            ));
            lines.push('\n');
        }
        // A new site's log appears: the supervisor restarts the engine so it
        // reads it.
        let before = m.status().await.started_at;
        std::fs::write(&log, "").unwrap();
        let mut restarted = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let s = m.status().await;
            if s.running && s.started_at != before {
                restarted = true;
                break;
            }
        }
        assert!(restarted, "engine not restarted for the new access log");
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if m.cscli(&["lapi", "status"], Duration::from_secs(10))
                .await
                .is_ok()
            {
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
            f.write_all(lines.as_bytes()).unwrap();
        }
        let mut banned = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if m.decisions()
                .await
                .unwrap_or_default()
                .iter()
                .any(|d| d.value == "203.0.113.50")
            {
                banned = true;
                break;
            }
        }
        let metrics = m
            .cscli(&["metrics"], Duration::from_secs(20))
            .await
            .unwrap_or_default();
        assert!(
            banned,
            "attacker not banned\nmetrics:\n{metrics}\nlog:\n{}",
            m.run
                .lock()
                .unwrap()
                .log
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Turning it off stops the engine.
        m.disable().unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!m.status().await.running);
        tx.send(true).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(20), sup).await;
    }

    #[test]
    fn config_files_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::open_memory(Arc::new(crate::secrets::SecretBox::new(b"k").unwrap())).unwrap(),
        );
        let m = Manager::new(
            store,
            dir.path().join("crowdsec"),
            "/data/caddy/logs/".into(),
            String::new(),
            true,
        );
        m.write_config().unwrap();
        let cfg = std::fs::read_to_string(m.config_path()).unwrap();
        assert!(cfg.contains("listen_uri: 127.0.0.1:8081"));
        assert!(cfg.contains("/crowdsec/data/crowdsec.db"));
        let acq = std::fs::read_to_string(m.etc().join("acquis.yaml")).unwrap();
        assert!(acq.contains("  - /data/caddy/logs/access-*.log"));
        assert!(acq.contains("type: caddy"));
        assert!(m.etc().join("profiles.yaml").is_file());
    }
}
