//! Updates never change what sites do.
//!
//! A new version may recommend a protection for every site (block AI
//! crawlers, keep sites out of search engines…). On a fresh install those
//! recommendations are simply the defaults. On an install that already
//! serves sites they are not switched on by the update: the stored default
//! is pinned to "off" and the recommendation is offered as a suggestion the
//! user can apply to all sites or to one site, or dismiss.
//!
//! Version 0.6.0 shipped bot blocking, privacy (robots.txt and headers) and
//! CrowdSec on by default, which switched them on for existing sites. An
//! install that has never saved its Security page gets its sites back as
//! they were before, with the three offered as suggestions.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::model::{SecuritySettings, ShieldOverrides, Shields};
use crate::shields;
use crate::store::Store;

const KEY: &str = "security_baseline";

/// A protection a version recommends for every site.
pub struct Recommendation {
    pub id: &'static str,
    /// The version that introduced it.
    pub since: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    get: fn(&Shields) -> bool,
    set: fn(&mut Shields, bool),
    set_site: fn(&mut ShieldOverrides, bool),
}

/// Every recommendation so far, oldest first. A new one is appended here
/// (with its field's default on in `SecuritySettings::default`); existing
/// installs then get it as a suggestion instead.
pub static RECOMMENDATIONS: &[Recommendation] = &[
    Recommendation {
        id: "bots",
        since: "0.6.0",
        title: "Block AI crawlers and scrapers",
        body: "Turns away bots that copy your sites for AI training, by their user agent (list refreshed daily).",
        get: |s| s.bots,
        set: |s, v| s.bots = v,
        set_site: |o, v| o.bots = Some(v),
    },
    Recommendation {
        id: "privacy",
        since: "0.6.0",
        title: "Keep sites out of search engines and AI datasets",
        body: "Serves a robots.txt that disallows everything, adds X-Robots-Tag noindex/noai and Referrer-Policy, and hides the Server header.",
        get: |s| s.privacy,
        set: |s, v| s.privacy = v,
        set_site: |o, v| o.privacy = Some(v),
    },
    Recommendation {
        id: "crowdsec",
        since: "0.6.0",
        title: "Block known attackers (CrowdSec)",
        body: "Blocks addresses CrowdSec has flagged. Takes effect once CrowdSec is turned on on the Security page.",
        get: |s| s.crowdsec,
        set: |s, v| s.crowdsec = v,
        set_site: |o, v| o.crowdsec = Some(v),
    },
];

fn find(id: &str) -> Result<&'static Recommendation> {
    RECOMMENDATIONS
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| anyhow!("unknown suggestion {id:?}"))
}

/// What this install knows about the recommendations (stored).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Baseline {
    /// Recommendations this install has seen (adopted or suggested).
    pub known: Vec<String>,
    /// Recommendations offered as suggestions (pinned off by an update).
    pub suggested: Vec<String>,
    /// Suggestions the user dismissed.
    pub dismissed: Vec<String>,
}

/// A suggestion for the Security page card.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Suggestion {
    pub id: String,
    pub title: String,
    pub body: String,
    pub since: String,
    /// Sites that don't have it, by id and name.
    pub sites: Vec<SiteRef>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SiteRef {
    pub id: i64,
    pub name: String,
}

/// Runs at every start, before anything is generated: pins new
/// recommendations off on installs that already serve sites.
pub fn settle(store: &Store) -> Result<()> {
    let stored: Option<Baseline> = store.kv_get_json(KEY);
    let existing =
        store.sites().map(|s| !s.is_empty()).unwrap_or(false) || store.last_good().is_ok();
    let mut b = stored.clone().unwrap_or_default();
    let new: Vec<&Recommendation> = RECOMMENDATIONS
        .iter()
        .filter(|r| !b.known.iter().any(|k| k == r.id))
        .collect();
    if new.is_empty() {
        return Ok(());
    }
    // The first run with this feature on an install whose owner already
    // chose the Security settings by saving that page: keep their choice.
    let chosen = stored.is_none() && security_saved(store);
    let pin = existing && !chosen;
    if pin {
        let ids: Vec<&str> = new.iter().map(|r| r.id).collect();
        store.update_settings(|s| {
            for r in &new {
                (r.set)(&mut s.security.defaults, false);
            }
        })?;
        tracing::info!(
            ?ids,
            "new Security defaults are offered as suggestions instead of switching on for existing sites"
        );
    }
    for r in new {
        b.known.push(r.id.to_string());
        if pin {
            b.suggested.push(r.id.to_string());
        }
    }
    store.kv_set_json(KEY, &b)
}

/// Whether the Security page was ever saved.
fn security_saved(store: &Store) -> bool {
    store
        .audit_log("security.update", 50, 0)
        .map(|v| v.iter().any(|e| e.action == "security.update"))
        .unwrap_or(false)
}

/// Open suggestions: suggested, not dismissed, and not yet on everywhere.
pub fn list(store: &Store) -> Vec<Suggestion> {
    let b: Baseline = store.kv_get_json(KEY).unwrap_or_default();
    let sec = store.settings().security;
    let sites = store.sites().unwrap_or_default();
    RECOMMENDATIONS
        .iter()
        .filter(|r| b.suggested.iter().any(|s| s == r.id))
        .filter(|r| !b.dismissed.iter().any(|s| s == r.id))
        .filter_map(|r| {
            let without: Vec<SiteRef> = sites
                .iter()
                .filter(|s| !(r.get)(&effective(&sec, s)))
                .map(|s| SiteRef {
                    id: s.id,
                    name: s.name.clone(),
                })
                .collect();
            (!(r.get)(&sec.defaults) && !without.is_empty()).then(|| Suggestion {
                id: r.id.into(),
                title: r.title.into(),
                body: r.body.into(),
                since: r.since.into(),
                sites: without,
            })
        })
        .collect()
}

fn effective(sec: &SecuritySettings, s: &crate::model::Site) -> Shields {
    let e = shields::effective(sec, &s.config.access);
    Shields {
        crowdsec: e.crowdsec,
        bots: e.bots,
        rate_limit: e.rate_limit,
        home_only: e.home_only,
        login: e.login,
        privacy: e.privacy,
    }
}

/// Applies a suggestion to every site (it becomes the default), or to one
/// site (an override). The change waits in Review & apply like any edit.
pub fn apply(store: &Store, id: &str, site: Option<i64>) -> Result<()> {
    let r = find(id)?;
    match site {
        None => {
            store.update_settings(|s| (r.set)(&mut s.security.defaults, true))?;
        }
        Some(sid) => {
            let mut s = store.site(sid)?;
            (r.set_site)(&mut s.config.access.shields, true);
            s.updated_at = chrono::Utc::now();
            store.update_site(&s)?;
        }
    }
    Ok(())
}

/// Hides a suggestion for good.
pub fn dismiss(store: &Store, id: &str) -> Result<()> {
    find(id)?;
    let mut b: Baseline = store.kv_get_json(KEY).unwrap_or_default();
    if !b.dismissed.iter().any(|d| d == id) {
        b.dismissed.push(id.to_string());
    }
    store.kv_set_json(KEY, &b)
}

#[cfg(test)]
mod tests;
