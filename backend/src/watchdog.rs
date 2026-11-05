//! The watchdog: restarts Caddy when it stops serving.
//!
//! Every few seconds it checks that Caddy serves (the admin API answers and
//! the listeners accept connections, see [`Restarter::serving`]). When that
//! has failed for a whole minute while Caddy should be running (it was
//! detected and no plugin job or other restart is under way), it restarts
//! Caddy through the plugin manager: killed if it does not answer, rolled
//! back to the image's Caddy if the custom build won't serve. The last
//! restart is remembered and shown in the sidebar.
//!
//! [`Restarter::serving`]: crate::recovery::Restarter::serving

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::watch;

use crate::notify;
use crate::plugins::Manager;
use crate::recovery::Outcome;
use crate::registry::Capabilities;
use crate::store::Store;

const STATE_KEY: &str = "watchdog_last";

/// The watchdog's last restart.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Restart {
    pub at: DateTime<Utc>,
    /// What was wrong ("the admin API does not answer: …").
    pub reason: String,
    /// Whether Caddy serves again.
    pub ok: bool,
    /// What happened, in plain words.
    pub outcome: String,
}

pub struct Watchdog {
    pub store: Arc<Store>,
    pub plugins: Arc<Manager>,
    pub caps: Arc<RwLock<Capabilities>>,
    /// How long Caddy may fail before it is restarted.
    pub grace: Duration,
    /// How often it checks.
    pub tick: Duration,
}

impl Watchdog {
    pub fn new(store: Arc<Store>, plugins: Arc<Manager>, caps: Arc<RwLock<Capabilities>>) -> Self {
        Watchdog {
            store,
            plugins,
            caps,
            grace: Duration::from_secs(60),
            tick: Duration::from_secs(5),
        }
    }

    /// The last restart, if any.
    pub fn last(store: &Store) -> Option<Restart> {
        store.kv_get_json(STATE_KEY)
    }

    /// Whether Caddy should be serving now: it was detected, and nothing
    /// that stops it on purpose (an install, a restart) is running.
    fn should_run(&self) -> bool {
        let detected = self.caps.read().map(|c| c.detected).unwrap_or(false);
        detected
            && !self
                .plugins
                .current_job()
                .is_some_and(|j| j.status == "running")
            && self.plugins.restarter.lock.try_lock().is_ok()
    }

    /// Checks until `shutdown` turns true.
    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let mut failing_since: Option<tokio::time::Instant> = None;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.tick) => {}
                _ = shutdown.changed() => return,
            }
            if *shutdown.borrow() {
                return;
            }
            if !self.should_run() {
                failing_since = None;
                continue;
            }
            let reason = match self.plugins.restarter.serving().await {
                Ok(()) => {
                    failing_since = None;
                    continue;
                }
                Err(e) => format!("{e:#}"),
            };
            let since = *failing_since.get_or_insert_with(tokio::time::Instant::now);
            if since.elapsed() < self.grace {
                continue;
            }
            tracing::warn!(reason, "caddy has not served for a minute; restarting it");
            self.restart(&reason).await;
            failing_since = None;
        }
    }

    /// Restarts Caddy now, records and notifies the outcome.
    pub async fn restart(&self, reason: &str) {
        let secs = self.grace.as_secs();
        let why = format!("Caddy has not been serving for {secs}s ({reason})");
        let (ok, outcome) = match self.plugins.recover(&why).await {
            Ok(Outcome::Serving) => {
                let msg = "Caddy was restarted and is serving again".to_string();
                self.plugins.send(
                    notify::EVENT_CADDY_RECOVERED,
                    "Caddy restarted by the watchdog",
                    &format!(
                        "{why}.\n{msg}. Nothing to do; if it keeps happening, check the Caddy log."
                    ),
                );
                (true, msg)
            }
            // The plugin manager has already notified these two.
            Ok(Outcome::RolledBack { to, .. }) => (
                true,
                format!("Caddy was restarted on {to} and is serving again"),
            ),
            Ok(Outcome::Down { why }) => (false, format!("Caddy could not be restarted: {why}")),
            Err(e) => {
                tracing::info!(err = %e, "watchdog restart skipped");
                return;
            }
        };
        let r = Restart {
            at: Utc::now(),
            reason: reason.to_string(),
            ok,
            outcome,
        };
        if let Err(e) = self.store.kv_set_json(STATE_KEY, &r) {
            tracing::warn!(err = %e, "could not save the watchdog state");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::tests::Harness;

    /// The Caddy process freezes (SIGSTOP: it holds its ports but answers
    /// nothing, like a hung process). The watchdog notices, waits out the
    /// grace period, kills it and the supervisor starts a fresh one, which
    /// serves. Needs CADDY_BIN and Unix signals.
    #[cfg(unix)]
    #[tokio::test]
    async fn restarts_a_frozen_caddy() {
        let Ok(bin) = std::env::var("CADDY_BIN") else {
            return;
        };
        let h = Harness::start(&bin).await;
        let wd = Arc::new(Watchdog {
            grace: Duration::from_secs(3),
            tick: Duration::from_millis(500),
            ..Watchdog::new(h.store.clone(), h.m.clone(), h.caps.clone())
        });
        let (stop, rx) = watch::channel(false);
        let task = tokio::spawn(wd.clone().run(rx));

        // Healthy: nothing happens.
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(Watchdog::last(&h.store).is_none());

        let pid = std::process::Command::new("pgrep")
            .arg("-f")
            .arg(h.caddyfile.to_string_lossy().as_ref())
            .output()
            .unwrap();
        let pid = String::from_utf8_lossy(&pid.stdout)
            .lines()
            .next()
            .expect("caddy pid")
            .trim()
            .to_string();
        assert!(
            std::process::Command::new("kill")
                .args(["-STOP", &pid])
                .status()
                .unwrap()
                .success()
        );

        let mut last = None;
        for _ in 0..120 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            last = Watchdog::last(&h.store);
            if last.is_some() {
                break;
            }
        }
        let last = last.expect("the watchdog restarted caddy");
        assert!(last.ok, "{last:?}");
        assert!(last.reason.contains("does not answer"), "{last:?}");
        h.m.restarter.serving().await.expect("serving again");
        assert_eq!(*h.sent.lock().unwrap(), vec![notify::EVENT_CADDY_RECOVERED]);
        assert_eq!(h.m.current_job().unwrap().kind, "watchdog");

        stop.send(true).unwrap();
        let _ = task.await;
        h.shutdown().await;
    }
}
