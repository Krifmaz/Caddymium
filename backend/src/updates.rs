//! Caddy update checker and optional automatic updates.
//!
//! A background loop looks for new Caddy releases at the configured
//! interval, remembers what it found (shown on the Plugins page and as a dot
//! in the navigation), sends the "update available" notification once per
//! release and, when automatic updates are on and the clock is inside the
//! maintenance window, installs the release through the plugin manager. That
//! is the same download → verify → validate → swap → restart path as the
//! Update button, so a release that does not validate against the current
//! configuration never replaces the running build, and a build that dies at
//! startup is set aside by the supervisor or the container entrypoint.

use chrono::{DateTime, Local, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::watch;

use crate::model::UpdateSettings;
use crate::notify::{self, Notifier};
use crate::plugins;
use crate::registry::{self, Capabilities};
use crate::store::Store;

const STATE_KEY: &str = "updates_state";
/// Who an automatic install is attributed to in the audit log.
pub const AUTO_USER: &str = "auto-update";
/// How often the loop re-reads the settings and looks at the clock.
const TICK: Duration = Duration::from_secs(60);
/// An install that takes longer than this is reported as timed out.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// What the checker remembers between runs (stored in the database).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateState {
    pub last_check: Option<DateTime<Utc>>,
    /// Newest release tag seen (v2.11.7); empty until a check succeeded.
    pub latest: String,
    /// Version that was running at the last check.
    pub running: String,
    pub update_available: bool,
    /// Release the "update available" notification was sent for.
    pub notified_for: String,
    /// Outcome of the last automatic install.
    pub last_auto: Option<AutoResult>,
    /// Why the last check failed; the previous result is kept meanwhile.
    pub last_error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoResult {
    pub at: DateTime<Utc>,
    pub from: String,
    pub to: String,
    pub ok: bool,
    pub error: String,
}

/// What the API reports: the stored state plus the schedule.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    #[serde(flatten)]
    pub state: UpdateState,
    pub settings: UpdateSettings,
    pub next_check: Option<DateTime<Utc>>,
    pub in_window: bool,
    /// Empty when an automatic install may run now, else the reason not.
    pub auto_blocked: String,
}

pub struct Checker {
    pub store: Arc<Store>,
    pub plugins: Arc<plugins::Manager>,
    pub notifier: Arc<Notifier>,
    pub caps: Arc<RwLock<Capabilities>>,
}

impl Checker {
    pub fn state(&self) -> UpdateState {
        self.store.kv_get_json(STATE_KEY).unwrap_or_default()
    }

    /// The state, after dropping a failed update that a later install (say,
    /// Update Caddy on the Plugins page) or the running version made moot.
    pub fn state_settled(&self) -> UpdateState {
        let mut st = self.state();
        let caps = self.caps();
        if caps.detected && !caps.version_s.is_empty() {
            st.running = caps.version_s.clone();
            st.update_available = is_newer(&st.latest, &caps.version);
        }
        if settle(&mut st) {
            self.save(&st);
        }
        st
    }

    fn save(&self, st: &UpdateState) {
        if let Err(e) = self.store.kv_set_json(STATE_KEY, st) {
            tracing::warn!(err = %e, "could not save the update state");
        }
    }

    fn caps(&self) -> Capabilities {
        self.caps.read().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn settings(&self) -> UpdateSettings {
        self.store.settings().updates
    }

    /// Looks up the newest release and records whether it is newer than the
    /// running Caddy. Notifies once per new release.
    pub async fn check(&self, force: bool) -> UpdateState {
        let caps = self.caps();
        let latest = self.plugins.latest_caddy(force).await;
        let mut st = self.state();
        st.last_check = Some(Utc::now());
        st.running = caps.version_s.clone();
        if latest.is_empty() {
            st.last_error = "could not fetch the latest release from GitHub".into();
        } else {
            st.last_error.clear();
            st.latest = latest;
        }
        st.update_available = is_newer(&st.latest, &caps.version);
        if st.update_available && st.notified_for != st.latest {
            let how = if self.settings().auto {
                "It will be installed automatically."
            } else {
                "Install it from Plugins → Update Caddy."
            };
            self.notifier.send(
                notify::EVENT_UPDATE_AVAILABLE,
                &format!("Caddy {} is available", st.latest),
                &format!("Running {}. {how}", st.running),
            );
            st.notified_for = st.latest.clone();
        }
        settle(&mut st);
        self.save(&st);
        st
    }

    pub fn report(&self) -> Report {
        let s = self.settings();
        let st = self.state_settled();
        let next_check = s.check.then(|| {
            st.last_check
                .map(|t| t + chrono::Duration::hours(s.interval_hours()))
                .unwrap_or_else(Utc::now)
        });
        let in_window = in_window(&s.window_start, &s.window_end, Local::now().time());
        let auto_blocked = self.auto_blocked(&s, &st);
        Report {
            state: st,
            settings: s,
            next_check,
            in_window,
            auto_blocked,
        }
    }

    /// Empty when an automatic install may run now, else the reason it may not.
    fn auto_blocked(&self, s: &UpdateSettings, st: &UpdateState) -> String {
        if !s.auto {
            return "automatic updates are off".into();
        }
        if !st.update_available {
            return "no update available".into();
        }
        if let Some(a) = &st.last_auto {
            if !a.ok && a.to == st.latest {
                return format!(
                    "the automatic install of {} failed; update from the Plugins page once the cause is fixed, or wait for the next release",
                    st.latest
                );
            }
        }
        if !in_window(&s.window_start, &s.window_end, Local::now().time()) {
            return format!(
                "waiting for the maintenance window ({}–{})",
                s.window_start, s.window_end
            );
        }
        if let Err(e) = self.plugins.writable() {
            return format!("{e:#}");
        }
        if !self.caps().detected {
            return "Caddy has not been detected yet".into();
        }
        if self
            .plugins
            .current_job()
            .is_some_and(|j| j.status == "running")
        {
            return "another plugin operation is running".into();
        }
        String::new()
    }

    /// Starts the upgrade job and waits for it, recording the outcome.
    async fn auto_install(&self, st: &UpdateState) {
        let (from, to) = (st.running.clone(), st.latest.clone());
        tracing::info!(from, to, "installing the Caddy update automatically");
        let job = match self
            .plugins
            .start_install(Vec::new(), Vec::new(), AUTO_USER.into())
        {
            Ok(j) => j,
            Err(e) => {
                self.record_auto(&from, &to, Err(format!("{e:#}")));
                return;
            }
        };
        let deadline = tokio::time::Instant::now() + INSTALL_TIMEOUT;
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            match self.plugins.current_job() {
                Some(j) if j.id == job.id && j.status == "running" => {
                    if tokio::time::Instant::now() > deadline {
                        self.record_auto(&from, &to, Err("the install timed out".into()));
                        return;
                    }
                }
                Some(j) if j.id == job.id => {
                    let r = if j.status == "done" {
                        Ok(())
                    } else {
                        Err(j.error)
                    };
                    self.record_auto(&from, &to, r);
                    return;
                }
                _ => {
                    self.record_auto(&from, &to, Err("the install job disappeared".into()));
                    return;
                }
            }
        }
    }

    fn record_auto(&self, from: &str, to: &str, r: Result<(), String>) {
        let mut st = self.state();
        let ok = r.is_ok();
        let error = r.err().unwrap_or_default();
        st.last_auto = Some(AutoResult {
            at: Utc::now(),
            from: from.into(),
            to: to.into(),
            ok,
            error: error.clone(),
        });
        if ok {
            let caps = self.caps();
            st.running = caps.version_s.clone();
            st.update_available = is_newer(&st.latest, &caps.version);
            tracing::info!(from, to, "caddy updated automatically");
            self.notifier.send(
                notify::EVENT_UPDATE_INSTALLED,
                &format!("Caddy updated to {to}"),
                &format!("Automatic update from {from}. Caddy restarted on the new build."),
            );
        } else {
            tracing::warn!(from, to, error, "automatic caddy update failed");
            self.notifier.send(
                notify::EVENT_UPDATE_FAILED,
                &format!("Caddy update to {to} failed"),
                &format!(
                    "{error}\nThe previous build keeps running. Details are on the Plugins page."
                ),
            );
        }
        self.save(&st);
    }

    /// The background loop: checks when due, installs when allowed.
    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() {
                return;
            }
            let s = self.settings();
            if s.check && self.caps().detected {
                let st = self.state();
                let due = st
                    .last_check
                    .map(|t| Utc::now() - t >= chrono::Duration::hours(s.interval_hours()))
                    .unwrap_or(true);
                let st = if due { self.check(true).await } else { st };
                if st.update_available && self.auto_blocked(&s, &st).is_empty() {
                    self.auto_install(&st).await;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = shutdown.changed() => return,
            }
        }
    }
}

/// True when a failed automatic update no longer matters: the running
/// version already is (or is past) the one that failed, or already the
/// latest release. Its banner is then cleared.
pub fn failure_is_stale(a: &AutoResult, running: &str, latest: &str) -> bool {
    if a.ok {
        return false;
    }
    let run = registry::parse_version(running);
    if !run.valid {
        return false;
    }
    let reached = |v: &str| {
        let v = registry::parse_version(v);
        v.valid && run.compare(&v) != Ordering::Less
    };
    reached(&a.to) || reached(latest)
}

/// Drops a failed automatic update that no longer matters (see
/// [`failure_is_stale`]); true when it did.
fn settle(st: &mut UpdateState) -> bool {
    if st
        .last_auto
        .as_ref()
        .is_some_and(|a| failure_is_stale(a, &st.running, &st.latest))
    {
        st.last_auto = None;
        return true;
    }
    false
}

fn is_newer(latest: &str, running: &registry::version::Version) -> bool {
    let l = registry::parse_version(latest);
    l.valid && running.valid && l.compare(running) == Ordering::Greater
}

/// Parses "HH:MM" (24-hour).
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

/// True when `now` lies inside the daily window. A window that ends before
/// it starts wraps past midnight; an unset window is always open.
pub fn in_window(start: &str, end: &str, now: NaiveTime) -> bool {
    let (Some(a), Some(b)) = (parse_hhmm(start), parse_hhmm(end)) else {
        return true;
    };
    if a == b {
        return true;
    }
    if a < b {
        now >= a && now < b
    } else {
        now >= a || now < b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveTime {
        parse_hhmm(s).unwrap()
    }

    #[test]
    fn window_logic() {
        assert!(in_window("", "", t("12:00")), "no window = always");
        assert!(in_window("03:00", "05:00", t("04:00")));
        assert!(in_window("03:00", "05:00", t("03:00")), "start inclusive");
        assert!(!in_window("03:00", "05:00", t("05:00")), "end exclusive");
        assert!(!in_window("03:00", "05:00", t("12:00")));
        assert!(
            in_window("23:00", "02:00", t("23:30")),
            "wraps past midnight"
        );
        assert!(in_window("23:00", "02:00", t("01:00")));
        assert!(!in_window("23:00", "02:00", t("12:00")));
        assert!(
            in_window("04:00", "04:00", t("12:00")),
            "empty span = always"
        );
        assert!(in_window("bad", "05:00", t("12:00")), "unparsable = open");
    }

    #[test]
    fn newer_versions() {
        let running = registry::parse_version("v2.11.6 h1:abc");
        assert!(is_newer("v2.11.7", &running));
        assert!(is_newer("v2.12.0", &running));
        assert!(!is_newer("v2.11.6", &running));
        assert!(!is_newer("v2.11.5", &running));
        assert!(!is_newer("", &running));
        assert!(!is_newer("v2.11.7", &registry::parse_version("(devel)")));
    }

    fn failed(to: &str) -> AutoResult {
        AutoResult {
            to: to.into(),
            ok: false,
            error: "boom".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_failed_update_is_cleared_once_moot() {
        // Still running the old version, newer release still out: keep it.
        assert!(!failure_is_stale(&failed("v2.11.0"), "v2.10.2", "v2.11.0"));
        // A later (manual) update got there or past it.
        assert!(failure_is_stale(&failed("v2.11.0"), "v2.11.0", "v2.11.0"));
        assert!(failure_is_stale(&failed("v2.11.0"), "v2.11.1", "v2.11.1"));
        // The running version is already the latest (the release was pulled).
        assert!(failure_is_stale(&failed("v2.11.0"), "v2.10.2", "v2.10.2"));
        // Successes and unknown versions are left alone.
        assert!(!failure_is_stale(
            &AutoResult {
                ok: true,
                ..failed("v2.11.0")
            },
            "v2.11.0",
            "v2.11.0"
        ));
        assert!(!failure_is_stale(&failed("v2.11.0"), "", "v2.11.0"));

        let mut st = UpdateState {
            running: "v2.11.0".into(),
            latest: "v2.11.0".into(),
            last_auto: Some(failed("v2.11.0")),
            ..Default::default()
        };
        assert!(settle(&mut st));
        assert!(st.last_auto.is_none());
        assert!(!settle(&mut st));
    }

    #[test]
    fn hhmm() {
        assert!(parse_hhmm("00:00").is_some());
        assert!(parse_hhmm(" 23:59 ").is_some());
        assert!(parse_hhmm("24:00").is_none());
        assert!(
            parse_hhmm("7:00").is_some(),
            "chrono accepts a single digit hour"
        );
        assert!(parse_hhmm("noon").is_none());
    }
}
