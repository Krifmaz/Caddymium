//! Restarting Caddy so that it is serving again, or rolling back until it is.
//!
//! A restart only counts when Caddy is serving afterwards: the admin API
//! answers and every listener of the running config accepts connections
//! (plus the configured probe addresses, the :80/:443 a visitor reaches).
//! When a restart does not get there within the time limit, it is tried
//! once more; then the fallbacks the caller gives (the previous binary, the
//! image's own Caddy) are tried in order. A Caddy that does not answer the
//! stop request is killed through [`Control`], so recovery never needs a
//! container restart.

use anyhow::{Result, anyhow};
use std::sync::Arc;
use std::time::Duration;

use crate::apply::BoxFut;
use crate::caddy::Admin;
use crate::caddy::control::Control;

/// The parts of the admin API a restart uses (fakeable in tests).
pub trait CaddyApi: Send + Sync {
    fn ping<'a>(&'a self) -> BoxFut<'a, Result<()>>;
    fn config<'a>(&'a self) -> BoxFut<'a, Result<Vec<u8>>>;
    fn stop<'a>(&'a self) -> BoxFut<'a, Result<()>>;
}

impl CaddyApi for Admin {
    fn ping<'a>(&'a self) -> BoxFut<'a, Result<()>> {
        Box::pin(Admin::ping(self))
    }
    fn config<'a>(&'a self) -> BoxFut<'a, Result<Vec<u8>>> {
        Box::pin(Admin::config(self))
    }
    fn stop<'a>(&'a self) -> BoxFut<'a, Result<()>> {
        Box::pin(Admin::stop(self))
    }
}

/// Time limits of a restart.
#[derive(Debug, Clone)]
pub struct Timing {
    /// How long the stop request may take before the process is killed.
    pub stop: Duration,
    /// How long Caddy may take to exit after agreeing to stop.
    pub down: Duration,
    /// How long Caddy may take to serve again after a restart.
    pub up: Duration,
    /// Each single probe (admin request, listener dial).
    pub probe: Duration,
    pub poll: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            stop: Duration::from_secs(10),
            down: Duration::from_secs(15),
            up: Duration::from_secs(30),
            probe: Duration::from_secs(3),
            poll: Duration::from_millis(500),
        }
    }
}

/// Something to try when the restarted Caddy does not serve: it changes
/// which binary the supervisor starts next.
pub struct Fallback {
    /// What it restores, for the log and the error ("the previous build").
    pub label: String,
    pub apply: Box<dyn Fn() -> Result<()> + Send + Sync>,
}

/// How a restart ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Serving and the caller's check passed.
    Serving,
    /// The restart did not work; a fallback did. `why` is the last failure.
    RolledBack { to: String, why: String },
    /// Nothing worked: Caddy is not serving.
    Down { why: String },
}

/// A callback that verifies the restarted Caddy is the one expected (and
/// refreshes what the panel knows about it).
pub type Check<'a> = &'a (dyn Fn() -> BoxFut<'a, Result<()>> + Send + Sync);

pub struct Restarter {
    pub api: Arc<dyn CaddyApi>,
    pub control: Arc<dyn Control>,
    /// host:port addresses that must accept connections when Caddy serves
    /// HTTP (e.g. 127.0.0.1:80). Their host is also used to reach the ports
    /// the running config listens on.
    pub probe_addrs: Vec<String>,
    pub timing: Timing,
    /// One restart at a time (updates, the watchdog, rollbacks).
    pub lock: tokio::sync::Mutex<()>,
}

impl Restarter {
    pub fn new(
        api: Arc<dyn CaddyApi>,
        control: Arc<dyn Control>,
        probe_addrs: Vec<String>,
    ) -> Restarter {
        Restarter {
            api,
            control,
            probe_addrs,
            timing: Timing::default(),
            lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Checks once that Caddy is serving: the admin API answers and every
    /// listener accepts connections.
    pub async fn serving(&self) -> Result<()> {
        let t = self.timing.probe;
        match tokio::time::timeout(t, self.api.ping()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(anyhow!("the admin API does not answer: {e}")),
            Err(_) => return Err(anyhow!("the admin API does not answer (timed out)")),
        }
        let cfg = match tokio::time::timeout(t, self.api.config()).await {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => return Err(anyhow!("cannot read the running config: {e}")),
            Err(_) => return Err(anyhow!("cannot read the running config (timed out)")),
        };
        for addr in self.listeners(&cfg) {
            match tokio::time::timeout(t, tokio::net::TcpStream::connect(&addr)).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Err(anyhow!("listener {addr} does not answer: {e}")),
                Err(_) => return Err(anyhow!("listener {addr} does not answer (timed out)")),
            }
        }
        Ok(())
    }

    /// The addresses to dial: the probe addresses plus every TCP port the
    /// running config's HTTP servers listen on, at the probe host. None when
    /// the config serves no HTTP (a fresh install).
    pub fn listeners(&self, cfg: &[u8]) -> Vec<String> {
        let ports = http_ports(cfg);
        if ports.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<String> = self.probe_addrs.clone();
        let Some(host) = self.probe_addrs.first().and_then(|a| probe_host(a)) else {
            return out;
        };
        for p in ports {
            let a = join_host_port(&host, p);
            if !out.contains(&a) {
                out.push(a);
            }
        }
        out
    }

    /// Polls [`serving`](Self::serving) until it passes or `within` elapses.
    pub async fn wait_serving(&self, within: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let r = self.serving().await;
            if r.is_ok() || tokio::time::Instant::now() >= deadline {
                return r.map_err(|e| {
                    anyhow!(
                        "Caddy is not serving {}s after the restart: {e}",
                        within.as_secs()
                    )
                });
            }
            tokio::time::sleep(self.timing.poll).await;
        }
    }

    /// Stops Caddy so its supervisor starts it again: asks through the admin
    /// API, and kills the process when it does not answer or does not exit.
    pub async fn bounce(&self, log: &(dyn Fn(&str) + Send + Sync), force: bool) {
        let gen0 = self.control.generation();
        if !force {
            match tokio::time::timeout(self.timing.stop, self.api.stop()).await {
                Ok(Ok(())) => {
                    if self.wait_restarted(gen0).await {
                        return;
                    }
                    log(&format!(
                        "Caddy did not exit within {}s of agreeing to stop; stopping the process",
                        self.timing.down.as_secs()
                    ));
                }
                Ok(Err(e)) => log(&format!(
                    "Caddy did not accept the stop request ({e}); stopping the process"
                )),
                Err(_) => log(&format!(
                    "Caddy did not answer the stop request within {}s; stopping the process",
                    self.timing.stop.as_secs()
                )),
            }
        }
        match self.control.kill() {
            Ok(()) => {
                self.wait_restarted(gen0).await;
            }
            Err(e) => log(&format!("Could not stop the Caddy process: {e:#}")),
        }
    }

    /// True once a new process started (or, when that can't be told, once
    /// the old one stopped answering).
    async fn wait_restarted(&self, gen0: Option<u64>) -> bool {
        let deadline = tokio::time::Instant::now() + self.timing.down;
        while tokio::time::Instant::now() < deadline {
            match gen0 {
                Some(g) => {
                    if self.control.generation().is_some_and(|n| n != g) {
                        return true;
                    }
                }
                None => {
                    let quick = tokio::time::timeout(self.timing.probe, self.api.ping()).await;
                    if !matches!(quick, Ok(Ok(()))) {
                        return true;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200).min(self.timing.poll)).await;
        }
        false
    }

    /// Restarts Caddy and makes sure it serves: one restart, one retry, then
    /// each fallback in order. `check` runs once Caddy serves and may reject
    /// it (say, the wrong version came up).
    pub async fn restart(
        &self,
        log: &(dyn Fn(&str) + Send + Sync),
        check: Check<'_>,
        fallbacks: &[Fallback],
    ) -> Outcome {
        let _g = self.lock.lock().await;
        log("Restarting Caddy…");
        self.bounce(log, false).await;
        let mut why = match self.up_and_checked(check).await {
            Ok(()) => return Outcome::Serving,
            Err(e) => format!("{e:#}"),
        };
        log(&format!("{why}; restarting once more"));
        self.bounce(log, true).await;
        match self.up_and_checked(check).await {
            Ok(()) => return Outcome::Serving,
            Err(e) => why = format!("{e:#}"),
        }
        for f in fallbacks {
            log(&format!("{why}; rolling back to {}", f.label));
            if let Err(e) = (f.apply)() {
                log(&format!("Could not restore {}: {e:#}", f.label));
                continue;
            }
            self.bounce(log, true).await;
            match self.wait_serving(self.timing.up).await {
                Ok(()) => {
                    log(&format!("Caddy is serving again on {}", f.label));
                    return Outcome::RolledBack {
                        to: f.label.clone(),
                        why,
                    };
                }
                Err(e) => log(&format!("{} does not serve either: {e:#}", f.label)),
            }
        }
        Outcome::Down { why }
    }

    async fn up_and_checked(&self, check: Check<'_>) -> Result<()> {
        self.wait_serving(self.timing.up).await?;
        check().await
    }
}

/// TCP ports the HTTP servers of a running config listen on.
fn http_ports(cfg: &[u8]) -> Vec<u16> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(cfg) else {
        return Vec::new();
    };
    let Some(servers) = v.pointer("/apps/http/servers").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    let mut ports = Vec::new();
    for s in servers.values() {
        for l in s
            .get("listen")
            .and_then(|l| l.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(p) = l.as_str().and_then(listen_port) {
                if !ports.contains(&p) {
                    ports.push(p);
                }
            }
        }
    }
    ports.sort_unstable();
    ports
}

/// The (first) TCP port of a Caddy listen address: ":443", "0.0.0.0:80",
/// "tcp/:8080", ":8000-8010". UDP and unix sockets have none.
fn listen_port(l: &str) -> Option<u16> {
    let l = l.trim();
    let rest = match l.split_once('/') {
        Some((net, rest)) if net.starts_with("tcp") => rest,
        Some(_) => return None,
        None => l,
    };
    let port = rest.rsplit_once(':').map(|(_, p)| p)?;
    port.split('-').next()?.parse().ok()
}

fn probe_host(addr: &str) -> Option<String> {
    let (h, _) = addr.rsplit_once(':')?;
    Some(h.to_string())
}

fn join_host_port(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

#[cfg(test)]
mod tests;
