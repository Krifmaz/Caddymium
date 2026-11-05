//! All-in-one mode: the panel runs Caddy as a child process and keeps it
//! running. This replaces the caddy container's entrypoint script of the
//! two-container layout.
//!
//! Each time Caddy (re)starts, the newest usable binary is picked: the
//! custom build the plugin manager installed (`caddy-custom` next to the
//! shared binary path) if it runs, else the binary shipped in the image. A
//! custom build that dies right after starting is set aside so the panel
//! never gets stuck without Caddy. The plugin manager restarts Caddy by
//! asking it to exit through the admin API, exactly as before; the loop
//! here starts it again with the new binary.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::watch;

use crate::caddy::control::ProcessHandle;
use crate::caddy::{Admin, Runner};
use crate::plugins::CUSTOM_NAME;
use crate::secrets;

#[derive(Debug, Clone)]
pub struct Managed {
    /// Caddy binary shipped in the image.
    pub image_bin: PathBuf,
    /// Path the panel validates with; the running binary is copied here.
    pub caddy_bin: PathBuf,
    pub caddyfile: PathBuf,
    pub admin_listen: String,
    pub admin_origins: Vec<String>,
    pub secrets_dir: PathBuf,
    pub log_dir: PathBuf,
    /// XDG_DATA_HOME for Caddy (certificates).
    pub data_dir: PathBuf,
    /// XDG_CONFIG_HOME for Caddy (autosave.json).
    pub config_dir: PathBuf,
}

/// Where the config a previous Caddy left behind is kept for import. Uses
/// the name from before the rename when only that file exists.
pub fn autosave_backup(config_dir: &Path) -> PathBuf {
    let dir = config_dir.join("caddy");
    let current = dir.join("autosave.pre-caddymium.json");
    let legacy = dir.join("autosave.pre-caddyhub.json");
    if !current.exists() && legacy.exists() {
        legacy
    } else {
        current
    }
}

impl Managed {
    fn custom_bin(&self) -> PathBuf {
        self.caddy_bin
            .parent()
            .map(|p| p.join(CUSTOM_NAME))
            .unwrap_or_else(|| PathBuf::from(CUSTOM_NAME))
    }

    /// Creates the directories Caddy and the panel share, keeps a copy of a
    /// config left by a previous Caddy (for import), and writes a minimal
    /// config so Caddy can start when there is nothing to serve yet.
    pub fn prepare(&self) -> Result<()> {
        for d in [
            self.caddyfile.parent().unwrap_or(Path::new(".")),
            self.caddy_bin.parent().unwrap_or(Path::new(".")),
            &self.secrets_dir,
            &self.log_dir,
            &self.data_dir,
            &self.config_dir,
        ] {
            std::fs::create_dir_all(d).with_context(|| format!("create {}", d.display()))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&self.secrets_dir, std::fs::Permissions::from_mode(0o700));
        }
        let cfg_dir = self.config_dir.join("caddy");
        let autosave = cfg_dir.join("autosave.json");
        let backup = autosave_backup(&self.config_dir);
        if let Ok(b) = std::fs::read(&autosave) {
            if !b.is_empty()
                && !backup.exists()
                && !String::from_utf8_lossy(&b).contains(&self.admin_listen)
            {
                let _ = std::fs::write(&backup, &b);
            }
        }
        if std::fs::metadata(&self.caddyfile)
            .map(|m| m.len() == 0)
            .unwrap_or(true)
        {
            std::fs::write(&self.caddyfile, self.bootstrap_caddyfile())?;
        }
        Ok(())
    }

    /// A config that only enables the admin API, so the panel can take over.
    pub fn bootstrap_caddyfile(&self) -> String {
        if self.admin_origins.is_empty() {
            format!("{{\n\tadmin {}\n}}\n", self.admin_listen)
        } else {
            format!(
                "{{\n\tadmin {} {{\n\t\torigins {}\n\t}}\n}}\n",
                self.admin_listen,
                self.admin_origins.join(" ")
            )
        }
    }

    /// Picks the binary to run: a working custom build, else the image's.
    pub async fn choose(&self) -> PathBuf {
        let custom = self.custom_bin();
        if custom.is_file() && Runner::new(&custom).version().await.is_ok() {
            return custom;
        }
        self.image_bin.clone()
    }

    fn publish(&self, bin: &Path) -> Result<()> {
        // Share the exact running binary so the panel validates with the
        // same build (and the same plugins) that serves traffic.
        if bin != self.caddy_bin {
            let tmp = self.caddy_bin.with_extension("tmp");
            std::fs::copy(bin, &tmp)
                .with_context(|| format!("copy {} to {}", bin.display(), tmp.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
            }
            std::fs::rename(&tmp, &self.caddy_bin)?;
        }
        Ok(())
    }

    /// Runs Caddy until `shutdown` turns true, restarting it whenever it
    /// exits. On shutdown Caddy is asked to stop gracefully first. `handle`
    /// lets the rest of the panel kill a Caddy that stopped answering.
    pub async fn run(
        self,
        admin: std::sync::Arc<Admin>,
        handle: std::sync::Arc<ProcessHandle>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut kills = handle.kill_requests();
        loop {
            if *shutdown.borrow() {
                return;
            }
            let bin = self.choose().await;
            if let Err(e) = self.publish(&bin) {
                tracing::warn!(err = %e, "could not publish the Caddy binary for validation");
            }
            let version = Runner::new(&bin)
                .version()
                .await
                .map(|v| v.split_whitespace().next().unwrap_or("").to_string())
                .unwrap_or_default();
            tracing::info!(bin = %bin.display(), version, "starting caddy");
            let started = Instant::now();
            let mut cmd = tokio::process::Command::new(&bin);
            cmd.args([
                "run",
                "--config",
                &secrets::to_slash(&self.caddyfile),
                "--adapter",
                "caddyfile",
            ])
            .env("XDG_DATA_HOME", &self.data_dir)
            .env("XDG_CONFIG_HOME", &self.config_dir)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(err = %e, bin = %bin.display(), "cannot start caddy");
                    self.quarantine(&bin, &format!("cannot start: {e}"));
                    if wait_or_shutdown(&mut shutdown, Duration::from_secs(5)).await {
                        return;
                    }
                    continue;
                }
            };
            let generation = handle.started();
            kills.mark_unchanged();
            let mut killed = false;
            let status = tokio::select! {
                s = child.wait() => s.ok(),
                _ = kill_requested(&mut kills, generation) => {
                    tracing::warn!("caddy stopped answering; killing it");
                    killed = true;
                    let _ = child.kill().await;
                    child.wait().await.ok()
                }
                _ = shutdown.changed() => {
                    tracing::info!("stopping caddy");
                    let _ = tokio::time::timeout(Duration::from_secs(10), admin.stop()).await;
                    match tokio::time::timeout(Duration::from_secs(15), child.wait()).await {
                        Ok(_) => {}
                        Err(_) => { let _ = child.kill().await; }
                    }
                    return;
                }
            };
            let code = status.and_then(|s| s.code()).unwrap_or(-1);
            let ran = started.elapsed();
            // A process killed on request is handled by whoever asked (it
            // picks the binary to fall back to); only crashes are set aside.
            if code != 0 && !killed && ran < Duration::from_secs(20) {
                self.quarantine(
                    &bin,
                    &format!("exited with {code} after {:.1}s", ran.as_secs_f64()),
                );
            }
            tracing::info!(code, "caddy exited; restarting");
            let delay = if code == 0 {
                Duration::from_millis(300)
            } else {
                Duration::from_secs(2)
            };
            if wait_or_shutdown(&mut shutdown, delay).await {
                return;
            }
        }
    }

    /// Sets a failing custom build aside so the image's Caddy takes over.
    fn quarantine(&self, bin: &Path, why: &str) {
        if bin == self.custom_bin() {
            let broken = bin.with_extension("broken");
            let _ = std::fs::remove_file(&broken);
            if std::fs::rename(bin, &broken).is_ok() {
                tracing::warn!(
                    why,
                    "custom caddy build set aside as {}; falling back to the image's Caddy",
                    broken.display()
                );
            }
        }
    }
}

/// Resolves when a kill of process `generation` is requested.
async fn kill_requested(kills: &mut watch::Receiver<u64>, generation: u64) {
    loop {
        if kills.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
        if *kills.borrow_and_update() == generation {
            return;
        }
    }
}

/// Sleeps for `d`; returns true if shutdown was requested meanwhile.
async fn wait_or_shutdown(shutdown: &mut watch::Receiver<bool>, d: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => *shutdown.borrow(),
        _ = shutdown.changed() => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// Starts a real Caddy under the supervisor, asks it to stop through the
    /// admin API, and checks that it comes back; then shuts down. Needs
    /// CADDY_BIN.
    #[tokio::test]
    async fn restarts_after_stop() {
        let Ok(bin) = std::env::var("CADDY_BIN") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let admin_addr = format!("127.0.0.1:{}", free_port());
        let m = Managed {
            image_bin: PathBuf::from(&bin),
            caddy_bin: dir.path().join("bin").join("caddy"),
            caddyfile: dir.path().join("Caddyfile"),
            admin_listen: admin_addr.clone(),
            admin_origins: vec![admin_addr.clone()],
            secrets_dir: dir.path().join("secrets"),
            log_dir: dir.path().join("logs"),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
        };
        m.prepare().unwrap();
        assert!(m.caddyfile.exists());
        assert_eq!(m.choose().await, PathBuf::from(&bin));
        let admin = Arc::new(Admin::new(&admin_addr));
        let (tx, rx) = watch::channel(false);
        let handle = Arc::new(ProcessHandle::default());
        let task = tokio::spawn(m.clone().run(admin.clone(), handle.clone(), rx));
        admin
            .wait_ready(Duration::from_secs(20))
            .await
            .expect("caddy up");
        assert!(
            m.caddy_bin.is_file(),
            "running binary published for validation"
        );
        admin.stop().await.unwrap();
        tokio::time::sleep(Duration::from_millis(800)).await;
        admin
            .wait_ready(Duration::from_secs(20))
            .await
            .expect("caddy restarted");
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .expect("supervisor exits")
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), admin.ping())
                .await
                .map(|r| r.is_err())
                .unwrap_or(true),
            "caddy stopped on shutdown"
        );
    }
}
