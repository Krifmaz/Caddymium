//! `#/security`: protections for every site at once (CrowdSec, bots, no-index, rate limits, home only, login
//! wall), per-site overrides, sign-in with two-factor, shared passwords, home networks, the crawler list and
//! the built-in CrowdSec.

use crate::app::{self, a, b, i, s};
use crate::form::{with_list, Check, ListField, Select, Text};
use crate::icons::icon;
use crate::pages::deploy::ib;
use crate::ui::{sure, toast_err, when, Alert, ErrorBox, Loading, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;

const BOUNCER: &str = "github.com/hslatman/caddy-crowdsec-bouncer";
const COLS: &[(&str, &str)] =
    &[("crowdsec", "CrowdSec"), ("bots", "Bots & AI"), ("privacy", "No-index"), ("rate_limit", "Rate limit"), ("home_only", "Home only")];

fn login_label(v: &str) -> &'static str {
    match v {
        "basic" => "Password",
        "sso" => "Sign-in + 2FA",
        _ => "No login",
    }
}

/// The editable copy of `/security`'s `security`: lists made, saved passwords blank.
pub fn editable(sec: &Value) -> Value {
    let mut s = sec.clone();
    for k in ["home_networks", "bots_allow"] {
        if !s[k].is_array() {
            s[k] = json!([]);
        }
    }
    let users: Vec<Value> =
        a(sec, "basic_auth_users").iter().map(|u| json!({ "username": s_(u, "username"), "password": "", "saved": true })).collect();
    s["basic_auth_users"] = Value::Array(users);
    s
}

fn s_(v: &Value, k: &str) -> String {
    s(v, k)
}

/// What `PUT /security` takes from the form.
pub fn to_save(sec: &Value) -> Value {
    let mut body = sec.clone();
    let trimmed = |v: &Value| -> Value {
        Value::Array(
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str())
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(|x| json!(x))
                .collect(),
        )
    };
    body["home_networks"] = trimmed(&sec["home_networks"]);
    body["bots_allow"] = trimmed(&sec["bots_allow"]);
    body["basic_auth_users"] = Value::Array(
        a(sec, "basic_auth_users")
            .iter()
            .filter(|u| !s(u, "username").trim().is_empty())
            .map(|u| json!({ "username": s(u, "username").trim(), "password": s(u, "password") }))
            .collect(),
    );
    let ev = i(sec, "rate_limit_events");
    body["rate_limit_events"] = json!(if ev > 0 { ev } else { 600 });
    let hours = i(&sec["sso"], "session_hours");
    body["sso"]["session_hours"] = json!(if hours > 0 { hours } else { 168 });
    body
}

/// The sign-in page address to suggest: `auth.` + the first site's domain.
pub fn suggest_portal(sites: &[Value]) -> String {
    let h = sites.iter().find_map(|x| a(x, "hosts").first().and_then(|h| h.as_str().map(str::to_string))).unwrap_or_default();
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() > 2 {
        format!("auth.{}", parts[parts.len() - 2..].join("."))
    } else if !h.is_empty() {
        format!("auth.{h}")
    } else {
        "auth.example.com".into()
    }
}

/// `#/security`.
#[component]
pub fn Security() -> impl IntoView {
    let d = RwSignal::new(Value::Null);
    let sec = RwSignal::new(Value::Null);
    let ext = RwSignal::new(Value::Null);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let decisions = RwSignal::new(Vec::<Value>::new());
    let sessions = RwSignal::new(Vec::<Value>::new());
    let ban_ip = RwSignal::new(String::new());
    let ban_for = RwSignal::new("24h".to_string());
    let show_log = RwSignal::new(false);
    let pick = RwSignal::new(json!({}));
    let admin = app::is_admin();
    let alive = Arc::new(AtomicBool::new(true));
    let watching = Arc::new(AtomicBool::new(false));
    {
        let alive = alive.clone();
        on_cleanup(move || alive.store(false, Relaxed));
    }

    let load_side = move || {
        rt::spawn(async move {
            let running = b(&d.get_untracked()["crowdsec"], "running");
            decisions.set(if running {
                api::get("/crowdsec/decisions").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
            } else {
                vec![]
            });
            sessions.set(api::get("/security/sessions").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default());
        });
    };
    fn watch(d: RwSignal<Value>, alive: Arc<AtomicBool>, watching: Arc<AtomicBool>, done: impl Fn() + Copy + 'static) {
        if watching.swap(true, Relaxed) {
            return;
        }
        fn tick(d: RwSignal<Value>, alive: Arc<AtomicBool>, watching: Arc<AtomicBool>, done: impl Fn() + Copy + 'static) {
            rt::after(2500, move || {
                rt::spawn(async move {
                    if !alive.load(Relaxed) {
                        watching.store(false, Relaxed);
                        return;
                    }
                    let Ok(cs) = api::get("/crowdsec").await else {
                        watching.store(false, Relaxed);
                        return;
                    };
                    d.update(|v| v["crowdsec"] = cs.clone());
                    if s(&cs["job"], "status") == "running" {
                        tick(d, alive, watching, done);
                        return;
                    }
                    watching.store(false, Relaxed);
                    if s(&cs["job"], "status") == "failed" {
                        toast(&format!("CrowdSec: {}", s(&cs["job"], "error")), "err");
                    } else {
                        toast("CrowdSec is ready.", "ok");
                    }
                    done();
                    app::refresh_status();
                });
            });
        }
        tick(d, alive, watching, done);
    }
    // takes a /security answer
    let take = move |data: Value| {
        sec.set(editable(&data["security"]));
        let x = &data["crowdsec_external"];
        ext.set(json!({ "enabled": b(x, "enabled"), "api_url": s(x, "api_url"), "api_key": "" }));
        d.set(data);
    };
    let load = {
        let (alive, watching) = (alive.clone(), watching.clone());
        move || {
            let (alive, watching) = (alive.clone(), watching.clone());
            rt::spawn(async move {
                match api::get("/security").await {
                    Ok(v) => {
                        let running = s(&v["crowdsec"]["job"], "status") == "running";
                        take(v);
                        if running {
                            let reload = move || {
                                rt::spawn(async move {
                                    if let Ok(v) = api::get("/security").await {
                                        take(v);
                                        load_side();
                                    }
                                });
                            };
                            watch(d, alive, watching, reload);
                        }
                    }
                    Err(e) => error.set(Some(e.message)),
                }
                load_side();
            });
        }
    };
    load();
    let reload = move || {
        rt::spawn(async move {
            if let Ok(v) = api::get("/security").await {
                take(v);
                load_side();
            }
        });
    };

    let save = move |_| {
        error.set(None);
        busy.set(true);
        let body = json!({ "security": to_save(&sec.get_untracked()), "crowdsec_external": ext.get_untracked() });
        rt::spawn(async move {
            match api::put("/security", body).await {
                Ok(v) => {
                    take(v);
                    toast("Saved. Apply on Review & apply to update Caddy.", "ok");
                    app::refresh_status();
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };
    let save_button = move || {
        admin.then(
            || view! { <div class="row"><button class="btn primary sm" on:click=save disabled=move || busy.get()>"Save"</button></div> },
        )
    };

    let set_site = move |id: i64, overrides: Value, key: &'static str, val: Value| {
        let mut o = overrides.as_object().cloned().unwrap_or_default();
        o.insert(key.to_string(), val);
        o.retain(|_, v| !v.is_null());
        rt::spawn(async move {
            match api::put(&format!("/security/sites/{id}"), Value::Object(o)).await {
                Ok(v) => {
                    take(v);
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
        });
    };
    let suggest = move |sid: String, site: Option<i64>| {
        busy.set(true);
        rt::spawn(async move {
            let body = site.map(|x| json!({ "site_id": x })).unwrap_or(json!({}));
            match api::post(&format!("/security/suggestions/{sid}/apply"), body).await {
                Ok(v) => {
                    take(v);
                    toast(
                        if site.is_some() {
                            "Turned on for that site. Apply on Review & apply to update Caddy."
                        } else {
                            "Turned on for all sites. Apply on Review & apply to update Caddy."
                        },
                        "ok",
                    );
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };
    let crowdsec = {
        let (alive, watching) = (alive.clone(), watching.clone());
        move |action: &'static str| {
            busy.set(true);
            let (alive, watching) = (alive.clone(), watching.clone());
            rt::spawn(async move {
                match api::post_empty(&format!("/crowdsec/{action}")).await {
                    Ok(cs) => {
                        let running = s(&cs["job"], "status") == "running";
                        d.update(|v| v["crowdsec"] = cs);
                        if running {
                            show_log.set(true);
                            watch(d, alive, watching, reload);
                        } else {
                            reload();
                        }
                        if action == "disable" {
                            toast("CrowdSec turned off.", "ok");
                        }
                    }
                    Err(e) => toast_err(e),
                }
                busy.set(false);
            });
        }
    };

    let suggestions = move || {
        let list = a(&d.get(), "suggestions");
        (admin && !list.is_empty()).then(|| {
            let rows = list
                .into_iter()
                .map(|sg| {
                    let id = s(&sg, "id");
                    let sites = a(&sg, "sites");
                    let who = if sites.len() == 1 { s(&sites[0], "name") } else { format!("{} sites", sites.len()) };
                    let (id1, id2, id3, id4) = (id.clone(), id.clone(), id.clone(), id.clone());
                    let many = sites.len() > 1;
                    let site_opts = sites.iter().map(|x| view! { <option value=i(x, "id").to_string()>{s(x, "name")}</option> }).collect_view();
                    view! {
                        <div class="row wrap" style="gap:12px;padding-top:10px;border-top:1px solid var(--border)">
                            <div style="flex:1 1 260px"><strong>{s(&sg, "title")}</strong>
                                <div class="muted small">{format!("{} Not on for {who}.", s(&sg, "body"))}</div></div>
                            <div class="row wrap">
                                <button class="btn primary sm" disabled=move || busy.get() on:click=move |_| suggest(id1.clone(), None)>"Turn on for all sites"</button>
                                {many.then(|| view! {
                                    <select class="input" style="width:auto;max-width:200px" aria-label="Site"
                                        on:change=move |e| { let v = event_target_value(&e); let k = id2.clone(); pick.update(|p| p[k] = json!(v)); }>
                                        <option value="">"One site…"</option>{site_opts}
                                    </select>
                                    <button class="btn sm" disabled=move || busy.get() || s(&pick.get(), &id3).is_empty()
                                        on:click=move |_| suggest(id4.clone(), s(&pick.get_untracked(), &id4).parse().ok())>"Turn on"</button>
                                })}
                                <button class="btn sm" on:click=move |_| {
                                    let id = id.clone();
                                    rt::spawn(async move {
                                        match api::post_empty(&format!("/security/suggestions/{id}/dismiss")).await {
                                            Ok(v) => take(v),
                                            Err(e) => toast_err(e),
                                        }
                                    });
                                }>"Dismiss"</button>
                            </div>
                        </div>
                    }
                })
                .collect_view();
            view! {
                <div class="card stack">
                    <div><h2 style="margin:0">"Suggested protections"</h2>
                        <div class="muted small">"Newer versions recommend these. Updates never switch them on for sites you already have; turn them on here if you want them."</div></div>
                    {rows}
                </div>
            }
        })
    };

    let defaults = move || {
        let dd = d.get_untracked();
        let cs_ready = b(&dd["crowdsec"], "running") && b(&dd["plugins"], "crowdsec");
        let rl_plugin = b(&dd["plugins"], "rate_limit");
        let bots_on = Memo::new(move |_| b(&sec.get()["defaults"], "bots"));
        let rl_on = Memo::new(move |_| b(&sec.get()["defaults"], "rate_limit"));
        let tailscale = move || b(&sec.get(), "tailscale_is_home");
        view! {
            <div class="card stack">
                <div><h2 style="margin:0">"Protection for all sites"</h2><div class="muted small">"What every site gets unless it says otherwise below."</div></div>
                <div class="stack" style="gap:14px">
                    <div><Check state=sec path="defaults.crowdsec" label="Block known attackers"
                        hint="CrowdSec bans IPs that scan, brute-force or exploit, plus a community blocklist."/>
                        {(!cs_ready).then(|| view! { <small class="warn-text">"Not active yet: set it up below."</small> })}</div>
                    <div><Check state=sec path="defaults.bots" label="Block AI crawlers and scrapers"
                        hint=format!("{} known AI crawlers (GPTBot, ClaudeBot, Bytespider, Perplexity…), refreshed daily.", i(&dd["bots"], "ai_count"))/>
                        {move || bots_on.get().then(|| view! {
                            <div style="margin-left:48px"><Check state=sec path="bots_block_all"
                                label=format!("Also block search engines, SEO tools, archivers and link previews ({} more)", i(&d.get_untracked()["bots"], "other_count"))/></div>
                        })}</div>
                    <Check state=sec path="defaults.privacy" label="Keep out of search engines and AI datasets"
                        hint="Serves a robots.txt that disallows everything, sends noindex/noai headers and hides the server name."/>
                    <div><Check state=sec path="defaults.rate_limit" label="Limit requests per visitor" hint="Slows down password guessing and scraping."/>
                        {(!rl_plugin).then(|| view! { <small class="warn-text">"Needs the rate-limit plugin."</small> })}
                        {move || rl_on.get().then(|| view! {
                            <div style="margin-left:48px" class="stack">
                                <div class="row wrap">
                                    <div style="max-width:140px"><Text state=sec path="rate_limit_events" kind="number" label="Requests"/></div>
                                    <div style="max-width:120px"><Text state=sec path="rate_limit_window" label="per"/></div>
                                </div>
                                <Check state=sec path="rate_limit_exempt_home" label="Never limit devices at home"/>
                            </div>
                        })}</div>
                    <div>
                        <Check state=sec path="defaults.home_only" label="Home only"/>
                        <small class="muted" style="display:block;margin-left:48px">{move || format!("Only devices on your home network{} get in. Best for admin pages; rarely right for every site.",
                            if tailscale() { " or Tailscale" } else { "" })}</small>
                    </div>
                    <Select state=sec path="defaults.login" label="Login wall"
                        options=vec![("".into(), "No login wall".into()), ("sso".into(), "Sign-in with two-factor".into()), ("basic".into(), "Shared password".into())]
                        hint="Asked before the site's own login. Phone and TV apps can't pass a login wall; pick the app per site below to leave their paths open."/>
                </div>
                {move || error.get().map(|e| view! { <ErrorBox title="The security settings can't be saved" message=e/> })}
                {save_button}
            </div>
        }
    };

    let per_site = move || {
        let dd = d.get();
        let sites = a(&dd, "sites");
        let presets = a(&dd, "app_presets");
        let default_login = s(&sec.get_untracked()["defaults"], "login");
        let body = if sites.is_empty() {
            view! { <p class="muted">"No sites yet."</p> }.into_any()
        } else {
            let rows = sites
                .into_iter()
                .map(|x| {
                    let id = i(&x, "id");
                    let ov = x["overrides"].clone();
                    let eff = x["effective"].clone();
                    let name = if s(&x, "name").is_empty() { a(&x, "hosts").first().and_then(|h| h.as_str().map(str::to_string)).unwrap_or_default() } else { s(&x, "name") };
                    let hosts: Vec<String> = a(&x, "hosts").iter().filter_map(|h| h.as_str().map(str::to_string)).collect();
                    let pills = COLS
                        .iter()
                        .map(|(k, _)| {
                            let v = b(&eff, k);
                            let own = ov.get(*k).is_some_and(|x| !x.is_null());
                            let def = b(&sec.get_untracked()["defaults"], k);
                            let cur = ov.get(*k).cloned().unwrap_or(Value::Null);
                            let next = if !own { json!(!v) } else if cur.as_bool() == Some(!def) { Value::Null } else { json!(!cur.as_bool().unwrap_or(false)) };
                            let ov2 = ov.clone();
                            let title = if own { "Set for this site (click to change)" } else { "Default (click to override)" };
                            view! {
                                <td><button class="pill" class:on=v class:inherit=!own disabled=!admin title=title
                                    on:click=move |_| set_site(id, ov2.clone(), k, next.clone())>{if v { "On" } else { "Off" }}</button></td>
                            }
                        })
                        .collect_view();
                    let login_cur = ov.get("login").and_then(|v| v.as_str()).map(str::to_string).unwrap_or("__default".into());
                    let ov3 = ov.clone();
                    let ov4 = ov.clone();
                    let apps_cur = s(&ov, "apps");
                    let has_login = !s(&eff, "login").is_empty();
                    let login_opts = [("__default", format!("Default ({})", login_label(&default_login))), ("", "No login".into()), ("sso", "Sign-in + 2FA".into()), ("basic", "Password".into())]
                        .into_iter()
                        .map(|(v, l)| view! { <option value=v selected=login_cur == v>{l}</option> })
                        .collect_view();
                    let mut app_list: Vec<(String, String)> = vec![("".into(), "Browser only".into())];
                    app_list.extend(presets.iter().map(|p| (s(p, "id"), s(p, "name"))));
                    if apps_cur == "custom" {
                        app_list.push(("custom".into(), "Custom paths".into()));
                    }
                    let app_opts = app_list.into_iter().map(|(v, l)| { let sel = v == apps_cur; view! { <option value=v selected=sel>{l}</option> } }).collect_view();
                    view! {
                        <tr class:off=!b(&x, "enabled")>
                            <td style="white-space:normal;min-width:150px"><a href=format!("#/sites/{id}")><strong>{name}</strong></a>
                                <div class="muted small truncate">{hosts.join(", ")}</div></td>
                            {pills}
                            <td><select class="input mini" disabled=!admin
                                on:change=move |e| { let v = event_target_value(&e); set_site(id, ov3.clone(), "login", if v == "__default" { Value::Null } else { json!(v) }); }>{login_opts}</select></td>
                            <td><select class="input mini" disabled=!admin || !has_login
                                title=if has_login { "Which app paths skip the login wall" } else { "Only matters with a login wall" }
                                on:change=move |e| set_site(id, ov4.clone(), "apps", json!(event_target_value(&e)))>{app_opts}</select></td>
                        </tr>
                    }
                })
                .collect_view();
            let heads = COLS.iter().map(|(_, l)| view! { <th>{*l}</th> }).collect_view();
            view! {
                <div class="table-wrap"><table class="table matrix">
                    <thead><tr><th>"Site"</th>{heads}<th>"Login"</th><th>"Apps"</th></tr></thead>
                    <tbody>{rows}</tbody>
                </table></div>
                <p class="muted small" style="margin:0">"Per-site extras (allowed sign-in accounts, custom app paths, extra IP rules) are in each site's editor under Access."</p>
            }
            .into_any()
        };
        view! {
            <div class="card stack">
                <div><h2 style="margin:0">"Per site"</h2><div class="muted small">"Click a switch to override it for one site; click again to go back to the default (shown faded)."</div></div>
                {body}
            </div>
        }
    };

    let sign_in = move || {
        let dd = d.get_untracked();
        let portal = suggest_portal(&a(&dd, "sites"));
        let sso_used = a(&dd, "sites").iter().any(|x| b(x, "enabled") && s(&x["effective"], "login") == "sso");
        let members: Vec<String> = a(&dd, "users")
            .iter()
            .filter(|u| !b(u, "disabled"))
            .map(|u| format!("{}{}", s(u, "username"), if b(u, "totp_enabled") { " ✓2FA" } else { "" }))
            .collect();
        let no_portal = Memo::new(move |_| s(&sec.get()["sso"], "portal_host").is_empty());
        let end = move |id: String| {
            rt::spawn(async move {
                if id == "all"
                    && !sure("Sign everyone out?", "Everyone signed in to the protected sites has to sign in again.", "Sign out", true)
                        .await
                {
                    return;
                }
                match api::del(&format!("/security/sessions/{id}")).await {
                    Ok(_) => {
                        reload();
                        toast("Signed out.", "ok");
                    }
                    Err(e) => toast_err(e),
                }
            });
        };
        let session_rows = move || {
            let list = sessions.get();
            (!list.is_empty()).then(|| {
                let rows = list
                    .into_iter()
                    .map(|x| {
                        let id = s(&x, "id");
                        view! {
                            <tr>
                                <td>{s(&x, "username")}{b(&x, "mfa").then(|| view! { " "<span class="badge ok">"2FA"</span> })}</td>
                                <td class="mono small">{s(&x, "ip")}</td>
                                <td class="hide-sm small truncate" style="max-width:220px">{s(&x, "user_agent")}</td>
                                <td class="small">{when(&s(&x, "last_seen"))}</td>
                                <td class="right">{admin.then(|| view! { <button class="btn sm" on:click=move |_| end(id.clone())>"Sign out"</button> })}</td>
                            </tr>
                        }
                    })
                    .collect_view();
                view! {
                    <div class="table-wrap"><table class="table">
                        <thead><tr><th>"Account"</th><th>"From"</th><th class="hide-sm">"Device"</th><th>"Last seen"</th><th></th></tr></thead>
                        <tbody>{rows}</tbody>
                    </table></div>
                }
            })
        };
        let n_sessions = i(&dd["sso"], "sessions");
        let cookie = s(&dd["sso"], "cookie_domain");
        view! {
            <div class="card stack">
                <div><h2 style="margin:0">"Sign-in with two-factor"</h2>
                    <div class="muted small">"One sign-in page for all your sites, using Caddymium accounts. Signing in once covers every protected site for the session length."</div></div>
                <div class="form-grid">
                    <Text state=sec path="sso.portal_host" label="Sign-in page address" placeholder=portal.clone()
                        hint=format!("A new name under your domain, e.g. {portal}. Caddy serves it with HTTPS; Dynamic DNS keeps it pointed home.")/>
                    <Select state=sec path="sso.session_hours" label="Stay signed in for"
                        options=vec![("12".into(), "12 hours".into()), ("24".into(), "1 day".into()), ("168".into(), "7 days".into()), ("720".into(), "30 days".into()), ("2160".into(), "90 days".into())]/>
                </div>
                <Check state=sec path="sso.require_2fa" label="Require a code from an authenticator app (accounts without one set it up at their first sign-in)"/>
                {move || (sso_used && no_portal.get()).then(|| view! {
                    <Alert kind="warn">"Some sites use sign-in, but there's no sign-in page address yet. Those sites stay closed until you set one and apply."</Alert>
                })}
                <div class="small"><strong>"Who can sign in: "</strong>{format!("every active Caddymium account ({}). ", if members.is_empty() { "none".to_string() } else { members.join(", ") })}
                    "Add family or friends as " <strong>"Member"</strong> " on " <a href="#/users">"Users"</a> ": they can use your sites but not this panel. Limit a site to certain accounts in its editor."</div>
                <div class="row wrap">
                    <span class="small muted">{move || format!("{n_sessions} active sign-in(s){}", if no_portal.get() { String::new() } else { format!(" · cookie for {cookie}") })}</span>
                    {(n_sessions > 0 && admin).then(|| view! { <button class="btn sm" style="margin-left:auto" on:click=move |_| end("all".into()) inner_html=ib("log-out", "Sign everyone out")></button> })}
                </div>
                {session_rows}
                {save_button}
            </div>
        }
    };

    let passwords = move || {
        let count = Memo::new(move |_| a(&sec.get(), "basic_auth_users").len());
        view! {
            <div class="card stack">
                <div><h2 style="margin:0">"Shared password accounts"</h2>
                    <div class="muted small">"For the simpler \"Shared password\" login wall: the browser asks for a name and password. No two-factor; prefer sign-in where you can."</div></div>
                {move || (0..count.get()).map(|at| {
                    let saved = b(&sec.get_untracked()["basic_auth_users"][at], "saved");
                    view! {
                        <div class="row">
                            <div style="max-width:200px"><Text state=sec path=format!("basic_auth_users.{at}.username") label="Name"/></div>
                            <div class="grow" style="max-width:300px"><Text state=sec path=format!("basic_auth_users.{at}.password") label="Password" kind="password"
                                placeholder=if saved { "•••••• (unchanged)" } else { "8+ characters" }/></div>
                            <button class="btn sm" style="align-self:flex-end" title="Remove account"
                                on:click=move |_| with_list(sec, "basic_auth_users", |l| { if at < l.len() { l.remove(at); } }) inner_html=icon("trash")></button>
                        </div>
                    }
                }).collect_view()}
                <div class="row">
                    <button class="btn sm" on:click=move |_| with_list(sec, "basic_auth_users", |l| l.push(json!({ "username": "", "password": "", "saved": false })))
                        inner_html=ib("plus", "Add account")></button>
                </div>
                {save_button}
            </div>
        }
    };

    let home = move || {
        let vis = d.get_untracked()["visitors"].clone();
        let note = b(&vis, "available").then(|| {
            let (public, home, gw) = (i(&vis, "public"), i(&vis, "home"), i(&vis, "docker_gateway"));
            if public > 0 {
                Some(view! { <Alert kind="ok">{format!("Real visitor addresses reach Caddy ({public} recent requests from the internet, {home} from home), so home-only and rate limits can tell visitors apart.")}</Alert> }.into_any())
            } else if home > 0 && gw * 2 > home {
                Some(view! { <Alert kind="warn">"Every recent visitor appears to come from a Docker address, so home-only would let everyone in and rate limits would treat everyone as one visitor. This happens when Docker's userland proxy hides real addresses; publish the ports with the default iptables setup."</Alert> }.into_any())
            } else {
                None
            }
        });
        view! {
            <div class="card stack">
                <div><h2 style="margin:0">"Home networks"</h2><div class="muted small">"Used by \"Home only\" and to keep home devices out of rate limits."</div></div>
                <ListField state=sec path="home_networks" placeholder="10.0.1.0/24" add_label="Add network"/>
                <Check state=sec path="tailscale_is_home" label="Count Tailscale devices as home (100.64.0.0/10)"/>
                {note}
                {save_button}
            </div>
        }
    };

    let bots = move || {
        let dd = d.get_untracked();
        let fetched = s(&dd["bots"], "fetched_at");
        let refresh = move |_| {
            busy.set(true);
            rt::spawn(async move {
                match api::post_empty("/security/bots/refresh").await {
                    Ok(r) => {
                        let n = i(&r, "ai_count");
                        toast(
                            &if b(&r, "changed") {
                                format!("List updated: {n} AI crawlers.")
                            } else {
                                format!("Already current: {n} AI crawlers.")
                            },
                            "ok",
                        );
                        reload();
                        app::refresh_status();
                    }
                    Err(e) => toast_err(e),
                }
                busy.set(false);
            });
        };
        view! {
            <div class="card stack">
                <div class="row wrap">
                    <div class="grow"><h2 style="margin:0">"Crawler list"</h2>
                        <div class="muted small">{format!("AI crawler names come from the community-maintained ai.robots.txt list, refreshed daily. {}",
                            if fetched.is_empty() { "Using the built-in copy until the first refresh.".to_string() } else { format!("Last refreshed {}.", when(&fetched)) })}</div></div>
                    {admin.then(|| view! { <button class="btn sm" on:click=refresh disabled=move || busy.get() inner_html=ib("refresh", "Refresh now")></button> })}
                </div>
                <ListField state=sec path="bots_allow" label="Never block these user agents" placeholder="Uptime-Kuma" add_label="Add exception"
                    hint="Part of a user-agent string, e.g. your uptime monitor. Your own apps don't need this: they don't look like crawlers."/>
                {save_button}
            </div>
        }
    };

    let crowdsec_card = {
        let crowdsec = crowdsec.clone();
        move || {
            let dd = d.get();
            let cs = dd["crowdsec"].clone();
            let job_running = s(&cs["job"], "status") == "running";
            let line = if !b(&cs, "supported") {
                s(&cs, "reason")
            } else if b(&cs, "running") {
                let since = s(&cs, "started_at");
                let hub = s(&cs, "hub_updated_at");
                format!(
                    "Running {}{} · rules updated {}",
                    s(&cs, "installed_version"),
                    if since.is_empty() { String::new() } else { format!(" since {}", when(&since)) },
                    if hub.is_empty() { "never".to_string() } else { when(&hub) }
                )
            } else if b(&cs, "enabled") {
                "Starting…".into()
            } else {
                "Built in and off. Turning it on downloads CrowdSec (about 85 MB) and sets it up; no extra container needed.".into()
            };
            let (c1, c2, c3, c4) = (crowdsec.clone(), crowdsec.clone(), crowdsec.clone(), crowdsec.clone());
            let actions = (admin && b(&cs, "supported")).then(|| {
                if b(&cs, "enabled") {
                    view! {
                        {b(&cs, "update_available").then(|| view! {
                            <button class="btn sm" on:click=move |_| c1("update") disabled=move || busy.get() || job_running
                                inner_html=ib("download", &format!("Update to {}", s(&cs, "latest_version")))></button>
                        })}
                        <button class="btn sm" title="Refresh the detection rules now" on:click=move |_| c2("rules") disabled=move || busy.get() || job_running inner_html=ib("refresh", "Rules")></button>
                        <button class="btn sm danger" on:click=move |_| c3("disable") disabled=move || busy.get()>"Turn off"</button>
                    }
                    .into_any()
                } else {
                    view! { <button class="btn primary sm" on:click=move |_| c4("enable") disabled=move || busy.get() || job_running>"Turn on"</button> }.into_any()
                }
            });
            let install = move |_| {
                rt::spawn(async move {
                    if !sure("Install the CrowdSec bouncer?", "Download a Caddy build that adds the CrowdSec bouncer (keeps your other plugins) and restart Caddy? Sites are down for a few seconds.", "Install", false).await {
                        return;
                    }
                    match api::post("/plugins/install", json!({ "add": [BOUNCER], "remove": [] })).await {
                        Ok(_) => toast("Building Caddy with the bouncer; follow it on the Plugins page.", "ok"),
                        Err(e) => toast_err(e),
                    }
                });
            };
            let job = cs["job"].clone();
            let job_view = job.is_object().then(|| {
                let st = s(&job, "status");
                let cls = match st.as_str() {
                    "failed" => "badge err",
                    "done" => "badge ok",
                    _ => "badge accent",
                };
                let lines: Vec<String> = a(&job, "log").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
                view! {
                    <div class="row wrap small"><span class=cls>{format!("{}: {st}", s(&job, "kind"))}</span>
                        <button class="btn sm" on:click=move |_| show_log.update(|x| *x = !*x)>{move || if show_log.get() { "Hide log" } else { "Show log" }}</button></div>
                    {(st == "failed").then(|| view! { <ErrorBox title="CrowdSec setup failed" message=s(&job, "error")/> })}
                    {move || show_log.get().then(|| view! { <pre class="code-block scroll-55">{lines.join("\n")}</pre> })}
                }
            });
            let last = cs["last_auto"].clone();
            let running = b(&cs, "running");
            let tail: Vec<String> = a(&cs, "log_tail").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            let decisions_view = running.then(|| {
                let unban = move |id: i64, value: String| {
                    rt::spawn(async move {
                        match api::del(&format!("/crowdsec/decisions/{id}")).await {
                            Ok(v) => {
                                decisions.set(v.as_array().cloned().unwrap_or_default());
                                toast(&format!("Unbanned {value}."), "ok");
                            }
                            Err(e) => toast_err(e),
                        }
                    });
                };
                let ban = move |_| {
                    let body = json!({ "ip": ban_ip.get_untracked().trim(), "duration": ban_for.get_untracked() });
                    rt::spawn(async move {
                        match api::post("/crowdsec/decisions", body).await {
                            Ok(v) => {
                                decisions.set(v.as_array().cloned().unwrap_or_default());
                                ban_ip.set(String::new());
                                toast("Banned.", "ok");
                            }
                            Err(e) => toast_err(e),
                        }
                    });
                };
                view! {
                    <h3 style="font-size:14px;margin:6px 0 0">"Banned right now"</h3>
                    {move || {
                        let list = decisions.get();
                        if list.is_empty() {
                            view! { <p class="muted small" style="margin:0">"Nobody is banned by your CrowdSec at the moment. The community blocklist is applied on top and isn't listed here."</p> }.into_any()
                        } else {
                            let rows = list.into_iter().map(|x| {
                                let (id, value) = (i(&x, "id"), s(&x, "value"));
                                view! {
                                    <tr><td class="mono small">{value.clone()}</td><td class="small">{s(&x, "reason").replace("crowdsecurity/", "")}</td>
                                        <td class="hide-sm small">{s(&x, "until")}</td>
                                        <td class="right">{admin.then(|| view! { <button class="btn sm" on:click=move |_| unban(id, value.clone())>"Unban"</button> })}</td></tr>
                                }
                            }).collect_view();
                            view! { <div class="table-wrap"><table class="table"><thead><tr><th>"Address"</th><th>"Why"</th><th class="hide-sm">"Until"</th><th></th></tr></thead><tbody>{rows}</tbody></table></div> }.into_any()
                        }
                    }}
                    {admin.then(|| view! {
                        <div class="row wrap">
                            <input class="input" type="text" placeholder="IP or range to ban" style="max-width:220px"
                                prop:value=move || ban_ip.get() on:input=move |e| ban_ip.set(event_target_value(&e))/>
                            <select class="input" style="max-width:120px" on:change=move |e| ban_for.set(event_target_value(&e))>
                                <option value="4h">"4 hours"</option><option value="24h" selected=true>"1 day"</option>
                                <option value="168h">"1 week"</option><option value="8760h">"1 year"</option>
                            </select>
                            <button class="btn sm" on:click=ban disabled=move || ban_ip.get().trim().is_empty()>"Ban"</button>
                        </div>
                    })}
                    {(!tail.is_empty()).then(|| view! { <details><summary class="small muted">"Engine log"</summary><pre class="code-block scroll-55">{tail.join("\n")}</pre></details> })}
                }
            });
            let key_set = b(&dd["crowdsec_external"], "key_set");
            let ext_on = Memo::new(move |_| b(&ext.get(), "enabled"));
            view! {
                <div class="card stack">
                    <div class="row wrap">
                        <span class="s-ic" inner_html=icon("shield-check")></span>
                        <div class="grow"><h2 style="margin:0">"CrowdSec"</h2><div class="muted small">{line}</div></div>
                        {actions}
                    </div>
                    {(b(&cs, "enabled") && !b(&dd["plugins"], "crowdsec")).then(|| view! {
                        <Alert kind="warn">"Caddy doesn't have the CrowdSec bouncer plugin yet, so bans aren't enforced. "
                            {admin.then(|| view! { <button class="btn sm" on:click=install>"Install it"</button> })}</Alert>
                    })}
                    {(last.is_object() && !b(&last, "ok")).then(|| view! {
                        <ErrorBox title=format!("Automatic update to {} failed", s(&last, "to")) message=s(&last, "error")/>
                    })}
                    {job_view}
                    {decisions_view}
                    <details>
                        <summary class="small muted">"Use an external CrowdSec instead"</summary>
                        <div class="stack" style="margin-top:10px">
                            <Check state=ext path="enabled" label="Use a CrowdSec running elsewhere (ignored while the built-in one is on)"/>
                            {move || ext_on.get().then(|| view! {
                                <div class="form-grid">
                                    <Text state=ext path="api_url" label="Local API URL" placeholder="http://crowdsec:8080"/>
                                    <Text state=ext path="api_key" label="Bouncer API key" kind="password" placeholder=if key_set { "•••••• (saved)" } else { "" }/>
                                </div>
                            })}
                            {save_button}
                        </div>
                    </details>
                </div>
            }
        }
    };

    let loaded = Memo::new(move |_| !d.get().is_null());
    let page = move || {
        if !loaded.get() {
            return match error.get() {
                Some(e) => view! { <ErrorBox title="Security can't be loaded" message=e/> }.into_any(),
                None => view! { <Loading/> }.into_any(),
            };
        }
        untrack(|| {
            view! {
                <div class="stack">
                    {suggestions}
                    {defaults()}
                    {per_site}
                    {sign_in()}
                    {passwords()}
                    {home()}
                    {bots()}
                    {crowdsec_card.clone()}
                </div>
            }
            .into_any()
        })
    };
    view! {
        <PageHead title="Security" text="Protections for every site at once. Each site can turn any of them on or off for itself.">
            {move || (b(&d.get(), "pending_changes") && app::can_edit()).then(|| view! { <a class="btn primary" href="#/changes" inner_html=ib("rocket", "Review & apply")></a> })}
        </PageHead>
        {page}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_form_round_trip() {
        let sec =
            json!({ "basic_auth_users": [{ "username": "fam", "hash": "$2b..." }], "sso": { "session_hours": 0 }, "rate_limit_events": 0 });
        let e = editable(&sec);
        assert_eq!(e["basic_auth_users"][0], json!({ "username": "fam", "password": "", "saved": true }));
        assert_eq!(e["home_networks"], json!([]));
        let mut f = e.clone();
        f["home_networks"] = json!([" 10.0.1.0/24 ", ""]);
        f["basic_auth_users"].as_array_mut().unwrap().push(json!({ "username": " ", "password": "x" }));
        let out = to_save(&f);
        assert_eq!(out["home_networks"], json!(["10.0.1.0/24"]));
        assert_eq!(out["basic_auth_users"], json!([{ "username": "fam", "password": "" }]));
        assert_eq!((out["rate_limit_events"].clone(), out["sso"]["session_hours"].clone()), (json!(600), json!(168)));
        assert_eq!(suggest_portal(&[json!({ "hosts": ["tv.home.example.com"] })]), "auth.example.com");
        assert_eq!(suggest_portal(&[]), "auth.example.com");
    }
}
