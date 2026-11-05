use super::*;
use crate::model::{KIND_PROXY, ProxyConfig, Site, SiteConfig};
use std::sync::Arc;

fn store() -> Store {
    Store::open_memory(Arc::new(crate::secrets::SecretBox::new(b"k").unwrap())).unwrap()
}

fn add_site(s: &Store, host: &str) -> Site {
    s.create_site(Site {
        name: host.into(),
        hosts: vec![host.into()],
        kind: KIND_PROXY.into(),
        enabled: true,
        config: SiteConfig {
            proxy: Some(ProxyConfig {
                upstreams: vec!["app:80".into()],
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap()
}

fn ids(s: &Store) -> Vec<String> {
    list(s).into_iter().map(|x| x.id).collect()
}

#[test]
fn a_fresh_install_gets_the_recommended_defaults() {
    let s = store();
    settle(&s).unwrap();
    let d = s.settings().security.defaults;
    assert!(d.bots && d.privacy && d.crowdsec);
    add_site(&s, "a.example.com");
    settle(&s).unwrap();
    assert!(
        s.settings().security.defaults.bots,
        "settled once, never re-pinned"
    );
    assert!(list(&s).is_empty());
}

#[test]
fn an_update_never_switches_protections_on_for_existing_sites() {
    let s = store();
    let a = add_site(&s, "a.example.com");
    add_site(&s, "b.example.com");
    settle(&s).unwrap();
    let d = s.settings().security.defaults;
    assert!(!d.bots && !d.privacy && !d.crowdsec, "{d:?}");
    assert_eq!(ids(&s), vec!["bots", "privacy", "crowdsec"]);
    assert_eq!(list(&s)[0].sites.len(), 2);

    // One site: an override; the suggestion stays for the other.
    apply(&s, "bots", Some(a.id)).unwrap();
    assert_eq!(s.site(a.id).unwrap().config.access.shields.bots, Some(true));
    let bots = list(&s).into_iter().find(|x| x.id == "bots").unwrap();
    assert_eq!(bots.sites.len(), 1);
    assert_eq!(bots.sites[0].name, "b.example.com");

    // All sites: the default.
    apply(&s, "privacy", None).unwrap();
    assert!(s.settings().security.defaults.privacy);
    dismiss(&s, "crowdsec").unwrap();
    assert_eq!(ids(&s), vec!["bots"]);

    // Restarting changes nothing.
    settle(&s).unwrap();
    assert!(s.settings().security.defaults.privacy);
    assert_eq!(ids(&s), vec!["bots"]);
    assert!(apply(&s, "nope", None).is_err());
}

#[test]
fn a_saved_security_page_is_kept() {
    let s = store();
    add_site(&s, "a.example.com");
    s.audit("admin", "security.update", "", "", "");
    settle(&s).unwrap();
    let d = s.settings().security.defaults;
    assert!(d.bots && d.privacy, "the owner's choice stays");
    assert!(list(&s).is_empty());
}

#[test]
fn a_later_recommendation_becomes_a_suggestion() {
    let s = store();
    add_site(&s, "a.example.com");
    s.kv_set_json(
        KEY,
        &Baseline {
            known: vec!["bots".into(), "privacy".into()],
            ..Default::default()
        },
    )
    .unwrap();
    settle(&s).unwrap();
    let d = s.settings().security.defaults;
    assert!(d.bots && d.privacy, "known ones untouched");
    assert!(!d.crowdsec, "the new one is pinned off");
    assert_eq!(ids(&s), vec!["crowdsec"]);
}
