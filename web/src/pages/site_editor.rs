//! `#/sites/<id>` and `#/sites/new`: one site, from where it lives to what it does, access, headers and raw
//! Caddyfile; with app templates and "Find app".

use crate::app::{self, a, b, i, s};
use crate::form::{get_path, set_path, with_list, Area, Check, KvField, ListField, Select, Text};
use crate::icons::icon;
use crate::pages::deploy::ib;
use crate::sites::{blank, from_template, join_host, known_domains, payload, prepare, split_host, templates, HostRow, KINDS};
use crate::ui::{dialog, sure, toast_err, Alert, Avatar, ErrorBox, Loading};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

const SHIELDS: &[(&str, &str)] = &[
    ("crowdsec", "Block known attackers (CrowdSec)"),
    ("bots", "Block AI crawlers and scrapers"),
    ("privacy", "Keep out of search engines"),
    ("rate_limit", "Limit requests per visitor"),
    ("home_only", "Home only"),
];
const APP_PRESETS: &[(&str, &str)] =
    &[("vaultwarden", "Vaultwarden / Bitwarden apps"), ("jellyfin", "Jellyfin apps"), ("plex", "Plex apps"), ("immich", "Immich apps")];

fn opts(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect()
}

fn login_name(v: &str) -> &'static str {
    match v {
        "sso" => "sign-in + 2FA",
        "basic" => "shared password",
        _ => "no login wall",
    }
}

/// A `<details>` section of the editor.
fn section(ic: &str, title: &str, desc: impl Fn() -> String + Send + Sync + 'static, open: bool, body: AnyView) -> impl IntoView {
    view! {
        <details class="card editor-sec" open=open>
            <summary>
                <span class="s-ic" inner_html=icon(ic)></span>
                <span class="grow"><strong>{title.to_string()}</strong><br/><small class="muted">{desc}</small></span>
            </summary>
            <div class="stack" style="margin-top:14px">{body}</div>
        </details>
    }
}

/// The site's hostnames as "name . [domain ▾]" rows; a host outside the known domains is a full hostname.
#[component]
fn HostInput(state: RwSignal<Value>, domains: Vec<String>, default_domain: String) -> impl IntoView {
    let fallback = if default_domain.is_empty() { domains.first().cloned().unwrap_or_default() } else { default_domain };
    let to_row = {
        let (domains, fallback) = (domains.clone(), fallback.clone());
        move |h: &str| {
            if domains.is_empty() {
                HostRow { custom: true, full: h.to_string(), ..Default::default() }
            } else if h.is_empty() {
                HostRow { domain: fallback.clone(), ..Default::default() }
            } else {
                split_host(h, &domains)
            }
        }
    };
    let hosts: Vec<String> = a(&state.get_untracked(), "hosts").iter().filter_map(|h| h.as_str().map(str::to_string)).collect();
    let hosts = if hosts.is_empty() { vec![String::new()] } else { hosts };
    let rows = RwSignal::new(hosts.iter().map(|h| to_row(h)).collect::<Vec<_>>());
    let write = move || {
        let joined: Vec<Value> = rows.get_untracked().iter().map(|r| json!(join_host(r))).collect();
        state.update(|v| v["hosts"] = Value::Array(joined));
    };
    let count = Memo::new(move |_| rows.with(Vec::len));
    let shape = Memo::new(move |_| rows.with(|r| r.iter().map(|x| x.custom).collect::<Vec<_>>()));
    let has_domains = !domains.is_empty();
    let list = move || {
        shape.get();
        let n = count.get();
        let domains = domains.clone();
        (0..n)
            .map(|at| {
                let custom = rows.with_untracked(|r| r.get(at).map(|x| x.custom).unwrap_or(true));
                let field = if custom {
                    view! {
                        <input class="input grow" type="text" placeholder="app.example.com" aria-label="Hostname"
                            prop:value=move || rows.with(|r| r.get(at).map(|x| x.full.clone()).unwrap_or_default())
                            on:input=move |e| { let t = event_target_value(&e); rows.update_untracked(|r| if let Some(x) = r.get_mut(at) { x.full = t }); rows.notify(); write(); }/>
                    }
                    .into_any()
                } else {
                    view! {
                        <input class="input grow" style="font-weight:550" type="text" placeholder="app" aria-label="Subdomain"
                            prop:value=move || rows.with(|r| r.get(at).map(|x| x.sub.clone()).unwrap_or_default())
                            on:input=move |e| { let t = event_target_value(&e); rows.update_untracked(|r| if let Some(x) = r.get_mut(at) { x.sub = t }); rows.notify(); write(); }/>
                        <span class="muted" style="font-weight:700">"."</span>
                    }
                    .into_any()
                };
                let picker = has_domains.then(|| {
                    let options = domains
                        .iter()
                        .map(|d| {
                            let (d1, d2) = (d.clone(), d.clone());
                            view! { <option value=d1 selected=move || rows.with(|r| r.get(at).is_some_and(|x| !x.custom && x.domain == d2))>{d.clone()}</option> }
                        })
                        .collect_view();
                    let pick = move |e: leptos::ev::Event| {
                        let v = event_target_value(&e);
                        rows.update(|r| {
                            let Some(x) = r.get_mut(at) else { return };
                            if v == "__custom" {
                                x.full = join_host(x);
                                x.custom = true;
                            } else {
                                if x.custom {
                                    let sp = split_host(&x.full, std::slice::from_ref(&v));
                                    x.sub = if sp.custom { String::new() } else { sp.sub };
                                    x.custom = false;
                                }
                                x.domain = v;
                            }
                        });
                        write();
                    };
                    view! {
                        <select class="input" style="width:auto;max-width:52%" aria-label="Domain" on:change=pick>
                            {options}
                            <option value="__custom" selected=move || rows.with(|r| r.get(at).is_some_and(|x| x.custom))>"Other / full hostname…"</option>
                        </select>
                    }
                });
                let preview = move || {
                    rows.with(|r| r.get(at).cloned()).filter(|x| !x.custom && has_domains).map(|x| {
                        if x.sub.trim().is_empty() {
                            view! { <div class="muted small">"→ "<span class="mono">{x.domain.clone()}</span>" (the domain itself; type a name like “jellyfin” for a subdomain)"</div> }.into_any()
                        } else {
                            view! { <div class="muted small">"→ "<span class="mono">{join_host(&x)}</span></div> }.into_any()
                        }
                    })
                };
                view! {
                    <div class="row" style="gap:6px;flex-wrap:nowrap">
                        {field}
                        {picker}
                        {move || (count.get() > 1).then(|| view! {
                            <button class="btn sm" type="button" aria-label="Remove" title="Remove"
                                on:click=move |_| { rows.update(|r| { if at < r.len() { r.remove(at); } }); write(); } inner_html=icon("x")></button>
                        })}
                    </div>
                    {preview}
                }
            })
            .collect_view()
    };
    view! {
        <div class="stack" style="gap:6px">
            {list}
            <div><button class="btn sm" type="button" on:click=move |_| { rows.update(|r| r.push(to_row(""))); write(); }
                inner_html=ib("plus", "Add another hostname")></button></div>
        </div>
    }
}

/// "Find app": scans Docker and the LAN, and hands the picked result to `pick`.
fn scan(pick: impl Fn(Value) + Clone + Send + Sync + 'static) {
    let target = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let results = RwSignal::new(None::<Vec<Value>>);
    let scanned = RwSignal::new(Vec::<String>::new());
    let show_other = RwSignal::new(false);
    let run = move || {
        busy.set(true);
        error.set(None);
        results.set(None);
        rt::spawn(async move {
            match api::post("/scan", json!({ "target": target.get_untracked() })).await {
                Ok(r) => {
                    results.set(Some(a(&r, "results")));
                    scanned.set(a(&r, "scanned").iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
                    if b(&r, "timed_out") {
                        error.set(Some("The scan hit its time limit; results may be incomplete. Try a narrower range.".into()));
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };
    run();
    dialog(
        "Find a running app",
        true,
        move |c| {
            let closer = StoredValue::new_local(c);
            let list = move || {
                let pick = pick.clone();
                results.get().filter(|_| !busy.get()).map(move |res| {
                    let web: Vec<Value> = res.iter().filter(|r| !s(r, "scheme").is_empty()).cloned().collect();
                    let other: Vec<Value> = res.iter().filter(|r| s(r, "scheme").is_empty()).cloned().collect();
                    let groups = [("container", "Docker containers", true), ("network", "On your network", false)]
                        .into_iter()
                        .map(|(ic, title, docker)| {
                            let items: Vec<Value> = web.iter().filter(|r| (s(r, "source") == "docker") == docker).cloned().collect();
                            let n = items.len();
                            let pick = pick.clone();
                            (n > 0).then(move || {
                                let rows = items
                                    .into_iter()
                                    .map(|r| {
                                        let label = [s(&r, "guess"), s(&r, "name"), s(&r, "host")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
                                        let https = s(&r, "scheme") == "https";
                                        let (pick, rr) = (pick.clone(), r.clone());
                                        view! {
                                            <button class="prow" style="width:100%;text-align:left;font:inherit;color:inherit;cursor:pointer"
                                                on:click=move |_| { closer.with_value(|c| c.close()); pick(rr.clone()); }>
                                                <Avatar name=label.clone() size=34/>
                                                <div class="grow" style="min-width:0">
                                                    <div class="truncate"><strong>{label.clone()}</strong></div>
                                                    <div class="mono small truncate">{s(&r, "upstream")}</div>
                                                </div>
                                                <span class="badge">{if https { "HTTPS" } else { "HTTP" }}</span>
                                                <span class="btn sm primary" style="pointer-events:none">"Use"</span>
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                view! {
                                    <div class="row muted small" style="gap:6px;font-weight:600;text-transform:uppercase;padding-top:8px">
                                        <span inner_html=icon(ic)></span>{format!("{title} ({n})")}</div>
                                    {rows}
                                }
                            })
                        })
                        .collect_view();
                    let n_other = other.len();
                    view! {
                        {if web.is_empty() {
                            view! {
                                <div class="empty"><p>"No web apps found."</p>
                                    <p class="small muted">"If your apps are on another subnet, enter its range above (e.g. 10.0.2.0/24) or add it under Settings → Proxies & DNS → Find app ranges."</p></div>
                            }.into_any()
                        } else {
                            view! { <div class="stack" style="gap:6px;max-height:52vh;overflow-y:auto">{groups}</div> }.into_any()
                        }}
                        {(n_other > 0).then(|| view! {
                            <button class="btn sm" on:click=move |_| show_other.update(|x| *x = !*x)>{format!("{n_other} other open port(s) (not web servers)")}</button>
                            {move || show_other.get().then(|| view! {
                                <div class="row wrap" style="gap:6px">{other.iter().map(|r| view! { <span class="badge mono">{format!("{}:{}", s(r, "host"), i(r, "port"))}</span> }).collect_view()}</div>
                            })}
                        })}
                    }
                })
            };
            view! {
                <div class="stack">
                    <p class="muted" style="margin:0">"Looks for web apps in Docker and on your network, and fills in the address for you."</p>
                    <form class="row" on:submit=move |e| { e.prevent_default(); run(); }>
                        <input class="input grow" type="text" placeholder="Empty = Docker + your network · or an IP, name, or range like 10.0.2.0/24"
                            prop:value=move || target.get() on:input=move |e| target.set(event_target_value(&e))/>
                        <button class="btn primary" type="submit" disabled=move || busy.get()>{move || if busy.get() { "Scanning…" } else { "Scan" }}</button>
                    </form>
                    {move || if busy.get() {
                        view! { <div class="row"><span class="spinner"></span><span class="muted">"Scanning… this takes a few seconds."</span></div> }.into_any()
                    } else {
                        let sc = scanned.get();
                        (!sc.is_empty()).then(|| view! {
                            <div class="row wrap small muted" style="gap:6px">"Scanned: "{sc.into_iter().map(|x| view! { <span class="badge">{x}</span> }).collect_view()}</div>
                        }).into_any()
                    }}
                    {move || error.get().map(|e| view! { <Alert kind="warn">{e}</Alert> })}
                    {list}
                </div>
            }
        },
        |c| view! { <button class="btn" on:click=move |_| c.close()>"Close"</button> },
    );
}

/// A tri-state select for a shield: Default (the Security page's), On or Off.
fn tri(state: RwSignal<Value>, key: &'static str, label: &'static str, defaults: RwSignal<Value>) -> impl IntoView {
    let path = format!("config.access.shields.{key}");
    let p2 = path.clone();
    let cur = move |want: &'static str| {
        let v = get_path(&state.get(), &path);
        let now = match v.as_bool() {
            Some(true) => "on",
            Some(false) => "off",
            None => "__default",
        };
        now == want
    };
    let change = move |e: leptos::ev::Event| {
        let v = match event_target_value(&e).as_str() {
            "on" => json!(true),
            "off" => json!(false),
            _ => Value::Null,
        };
        state.update(|s| set_path(s, &p2, v));
    };
    let def = move || {
        let d = defaults.get();
        if d.is_null() {
            "Default".to_string()
        } else {
            format!("Default ({})", if b(&d, key) { "on" } else { "off" })
        }
    };
    let cur2 = cur.clone();
    let cur3 = cur.clone();
    view! {
        <label class="row" style="gap:10px;font-size:13.5px"><span class="grow">{label}</span>
            <select class="input" style="max-width:150px;padding:4px 8px" on:change=change>
                <option value="__default" selected=move || cur("__default")>{def}</option>
                <option value="on" selected=move || cur2("on")>"On"</option>
                <option value="off" selected=move || cur3("off")>"Off"</option>
            </select>
        </label>
    }
}

/// `#/sites/<id>`.
#[component]
pub fn SiteEditor(id: String) -> impl IntoView {
    let is_new = id == "new";
    let site_id: i64 = id.parse().unwrap_or(0);
    let state = RwSignal::new(Value::Null);
    let domains = RwSignal::new(Vec::<String>::new());
    let default_domain = RwSignal::new(String::new());
    let snippets = RwSignal::new(Vec::<Value>::new());
    let sec_defaults = RwSignal::new(Value::Null);
    let meta = RwSignal::new(Value::Null);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let raw_check = RwSignal::new(None::<Value>);
    let host_key = RwSignal::new(0u32);
    let editor = app::can_edit();
    {
        let id = id.clone();
        rt::spawn(async move {
            if let Ok(m) = app::meta().await {
                meta.set(m);
            }
            snippets.set(api::get("/snippets").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default());
            let site = if is_new { Ok(blank()) } else { api::get(&format!("/sites/{id}")).await };
            let doms = api::get("/domains").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            let all = api::get("/sites").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            let st = api::get("/settings").await.unwrap_or(json!({}));
            let list = known_domains(&doms, &all, &s(&st, "default_domain"));
            let dd = s(&st, "default_domain");
            default_domain.set(if dd.is_empty() { list.first().cloned().unwrap_or_default() } else { dd });
            domains.set(list);
            sec_defaults.set(st["security"]["defaults"].clone());
            match site {
                Ok(v) => state.set(prepare(v)),
                Err(e) => {
                    toast_err(e);
                    app::navigate("/sites");
                }
            }
        });
    }
    let available =
        move |fid: &str| !a(&meta.get(), "features").iter().any(|f| s(f, "id") == fid && f.get("available") == Some(&Value::Bool(false)));

    let save = move |review: bool| {
        error.set(None);
        busy.set(true);
        let body = payload(&state.get_untracked());
        rt::spawn(async move {
            let r = if is_new { api::post("/sites", body).await } else { api::put(&format!("/sites/{site_id}"), body).await };
            match r {
                Ok(saved) => {
                    toast("Saved. Apply to make it live.", "ok");
                    app::refresh_status();
                    if review {
                        app::navigate("/changes");
                    } else if is_new {
                        app::navigate(&format!("/sites/{}", i(&saved, "id")));
                    } else {
                        state.set(prepare(saved));
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };
    let remove = move |_| {
        rt::spawn(async move {
            let name = s(&state.get_untracked(), "name");
            if !sure(&format!("Delete {name}?"), "It leaves Caddy with the next apply. History keeps the old Caddyfiles.", "Delete", true)
                .await
            {
                return;
            }
            match api::del(&format!("/sites/{site_id}")).await {
                Ok(_) => {
                    toast("Deleted", "ok");
                    app::refresh_status();
                    app::navigate("/sites");
                }
                Err(e) => toast_err(e),
            }
        });
    };
    let duplicate = move |_| {
        rt::spawn(async move {
            match api::post_empty(&format!("/sites/{site_id}/clone")).await {
                Ok(c) => {
                    toast("Copy created (disabled)", "ok");
                    app::navigate(&format!("/sites/{}", i(&c, "id")));
                }
                Err(e) => toast_err(e),
            }
        });
    };
    let use_pick = move |r: Value| {
        let upstream = s(&r, "upstream");
        let https = s(&r, "scheme") == "https";
        let label = [s(&r, "guess"), s(&r, "host")].into_iter().find(|x| !x.trim().is_empty()).unwrap_or_default();
        let slug = label.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).find(|w| w.len() > 1).unwrap_or("").to_string();
        let (dl, dd) = (domains.get_untracked(), default_domain.get_untracked());
        state.update(|v| {
            let ups: Vec<Value> =
                a(&v["config"]["proxy"], "upstreams").into_iter().filter(|u| !u.as_str().unwrap_or("").trim().is_empty()).collect();
            let mut next = if ups.len() > 1 { ups } else { vec![] };
            next.push(json!(upstream));
            v["config"]["proxy"]["upstreams"] = Value::Array(next);
            if https {
                v["config"]["proxy"]["tls_skip_verify"] = json!(true);
            }
            if s(v, "name").is_empty() && !label.is_empty() {
                v["name"] = json!(label.trim());
            }
            let empty = a(v, "hosts").iter().all(|h| {
                let h = h.as_str().unwrap_or("").trim();
                h.is_empty() || dl.iter().any(|d| d == h)
            });
            if empty && !slug.is_empty() && !dd.is_empty() && !slug.starts_with(|c: char| c.is_ascii_digit()) {
                v["hosts"] = json!([format!("{slug}.{dd}")]);
            }
        });
        host_key.update(|k| *k += 1);
        toast(&format!("Using {upstream}{}", if https { " (self-signed certificates allowed)" } else { "" }), "ok");
    };
    let apply_template = move |tid: String| {
        let Some((tid, name, t)) = templates().into_iter().find(|t| t.0 == tid) else { return };
        let cur = state.get_untracked();
        let dl = domains.get_untracked();
        let typed = a(&cur, "hosts").iter().any(|h| {
            let h = h.as_str().unwrap_or("").trim();
            !h.is_empty() && !dl.iter().any(|d| d == h)
        });
        state.set(from_template(&t, tid, name, &cur, typed, &default_domain.get_untracked()));
        host_key.update(|k| *k += 1);
        toast(&format!("Filled in {name} defaults. Check the address and the upstream (container name:port)."), "ok");
    };

    let kind = Memo::new(move |_| s(&state.get(), "kind"));
    let loaded = Memo::new(move |_| !state.get().is_null());

    let head = move || {
        let v = state.get();
        let hosts: Vec<String> =
            a(&v, "hosts").iter().filter_map(|h| h.as_str()).map(str::trim).filter(|h| !h.is_empty()).map(str::to_string).collect();
        let name = s(&v, "name");
        let av = if !name.is_empty() { name.clone() } else { hosts.first().cloned().unwrap_or("?".into()) };
        view! {
            <Avatar name=av size=48/>
            <div class="grow" style="min-width:0">
                <h1 class="truncate" style="margin:0;font-size:22px">{if is_new { "New site".to_string() } else { name }}</h1>
                <div class="muted small truncate">{if hosts.is_empty() { "Add a hostname below".to_string() } else { hosts.join(", ") }}</div>
            </div>
        }
    };

    let kind_fields = move || {
        let k = kind.get();
        untrack(|| {
            match k.as_str() {
            "proxy" => view! {
                <div class="field">
                    <span class="row">"Upstream address"<button class="btn sm" style="margin-left:auto" type="button" on:click=move |_| scan(use_pick)
                        inner_html=ib("radar", "Find app")></button></span>
                    <ListField state=state path="config.proxy.upstreams" placeholder="jellyfin:8096" add_label="Add upstream"/>
                    <small>"Where the app listens, as address:port. For a Docker container on a network shared with Caddy, use its container name (jellyfin:8096). For anything else, use the machine's IP (10.0.1.5:32400). Add more than one to load-balance."</small>
                </div>
                <Check state=state path="config.proxy.websocket" label="WebSockets / streaming"/>
                <Check state=state path="config.proxy.tls_skip_verify" label="Upstream uses a self-signed certificate"/>
                <Check state=state path="config.proxy.h2c" label="gRPC / h2c upstream"/>
            }
            .into_any(),
            "static" => view! {
                <Text state=state path="config.static.root" label="Folder" placeholder="/srv/www" hint="Path inside the Caddy container."/>
                <Check state=state path="config.static.browse" label="Show directory listings"/>
                <Check state=state path="config.static.spa" label="Single-page app (fall back to index.html)"/>
            }
            .into_any(),
            "redirect" => view! {
                <div class="form-grid">
                    <Text state=state path="config.redirect.to" label="Redirect to" placeholder="https://example.com"/>
                    <Select state=state path="config.redirect.code" label="Type" options=opts(&[("302", "Temporary (302)"), ("301", "Permanent (301)"), ("307", "Temporary, keep method (307)"), ("308", "Permanent, keep method (308)")])/>
                </div>
                <Check state=state path="config.redirect.keep_path" label="Keep the path (/foo → target/foo)"/>
            }
            .into_any(),
            "respond" => view! {
                <div class="form-grid">
                    <Text state=state path="config.respond.status" kind="number" label="Status code"/>
                    <Text state=state path="config.respond.content_type" label="Content type" placeholder="text/plain (optional)"/>
                </div>
                <Area state=state path="config.respond.body" label="Body" rows=4/>
            }
            .into_any(),
            "forward_proxy" => {
                let host = move || a(&state.get(), "hosts").iter().filter_map(|h| h.as_str()).map(str::trim).find(|h| !h.is_empty()).unwrap_or("proxy.example.com").to_string();
                let logins = Memo::new(move |_| a(&get_path(&state.get(), "config.forward_proxy"), "basic_auth").len());
                view! {
                    {move || (!available("forward_proxy")).then(|| view! {
                        <Alert kind="warn">"The " <code>"forwardproxy"</code> " plugin isn't in your Caddy build. " <a href="#/plugins">"Install it under Plugins"</a> ", then apply."</Alert>
                    })}
                    <p class="muted small" style="margin:0">"Point devices at " <span class="mono">{move || format!("https://{}:443", host())}</span>
                        " as their HTTPS proxy. Logins are kept as plain text because the plugin compares them as-is; use a dedicated password."</p>
                    <div class="field"><span>"Logins"</span>
                        {move || (0..logins.get()).map(|at| view! {
                            <div class="row">
                                <div style="max-width:220px"><Text state=state path=format!("config.forward_proxy.basic_auth.{at}.username") label="Username"/></div>
                                <div style="max-width:240px"><Text state=state path=format!("config.forward_proxy.basic_auth.{at}.password") label="Password" kind="password"/></div>
                                <button class="btn sm" type="button" title="Remove login" style="align-self:flex-end"
                                    on:click=move |_| with_list(state, "config.forward_proxy.basic_auth", |l| { if at < l.len() { l.remove(at); } }) inner_html=icon("trash")></button>
                            </div>
                        }).collect_view()}
                        <div><button class="btn sm" type="button" on:click=move |_| with_list(state, "config.forward_proxy.basic_auth", |l| l.push(json!({ "username": "", "password": "" })))
                            inner_html=ib("plus", "Add login")></button></div>
                        <small>"Without a login the proxy is open to anyone who finds it. Combine with the IP rules under Access control."</small>
                    </div>
                    <Check state=state path="config.forward_proxy.hide_ip" label="Hide client IPs (no X-Forwarded-For)"/>
                    <Check state=state path="config.forward_proxy.hide_via" label="Hide the Via header"/>
                    <div class="form-grid">
                        <Text state=state path="config._ports" label="Allowed destination ports" placeholder="80, 443 (empty = any)"/>
                        <Text state=state path="config.forward_proxy.probe_resistance" label="Probe resistance secret" placeholder="secret.localhost (optional)"
                            hint="Only requests for this hostname reveal the proxy; to everyone else it looks like a normal site."/>
                        <Text state=state path="config.forward_proxy.dial_timeout" label="Dial timeout" placeholder="30s"/>
                        <Text state=state path="config.forward_proxy.serve_pac" label="Serve a PAC file at" placeholder="/proxy.pac (optional)"/>
                        <Text state=state path="config.forward_proxy.upstream" label="Chain to another proxy" placeholder="https://user:pass@host:port (optional)"/>
                    </div>
                    <ListField state=state path="config.forward_proxy.acl" label="Access rules (checked in order)" placeholder="deny 10.0.0.0/8" add_label="Add rule"
                        hint="allow or deny followed by a hostname, IP, range or all, e.g. deny 192.168.0.0/16 then allow all."/>
                }
                .into_any()
            }
            _ => view! { <Alert kind="info">"Write your directives under " <strong>"Custom Caddyfile"</strong> " below. They go inside this site's block."</Alert> }.into_any(),
        }
        })
    };

    let kinds = move || {
        KINDS
            .iter()
            .map(|(k, ic, label, desc)| {
                let k2 = *k;
                view! {
                    <label class="kind" class:active=move || kind.get() == k2>
                        <input type="radio" name="kind" value=k2 prop:checked=move || kind.get() == k2
                            on:change=move |_| state.update(|v| v["kind"] = json!(k2))/>
                        <span class="k-ic" inner_html=icon(ic)></span>
                        <strong>{*label}</strong><span class="muted small">{*desc}</span>
                    </label>
                }
            })
            .collect_view()
    };

    let lb = move || {
        (kind.get() == "proxy").then(|| {
            let body = view! {
                <div class="form-grid">
                    <Select state=state path="config.proxy.lb_policy" label="Load balancing policy"
                        options=opts(&[("", "Default (random)"), ("round_robin", "round_robin"), ("least_conn", "least_conn"), ("first", "first"), ("ip_hash", "ip_hash"), ("client_ip_hash", "client_ip_hash"), ("uri_hash", "uri_hash"), ("random_choose 2", "random_choose 2")])/>
                    <Text state=state path="config.proxy.lb_try_duration" label="Retry for" placeholder="e.g. 5s"/>
                    <Text state=state path="config.proxy.health_uri" label="Health check path" placeholder="/health"/>
                    <Text state=state path="config.proxy.health_interval" label="Check interval" placeholder="30s"/>
                    <Text state=state path="config.proxy.max_fails" kind="number" label="Mark down after fails" placeholder="1"/>
                    <Text state=state path="config.proxy.fail_duration" label="Remember fails for" placeholder="30s"/>
                    <Text state=state path="config.proxy.dial_timeout" label="Dial timeout" placeholder="e.g. 5s"/>
                    <Text state=state path="config.proxy.response_header_timeout" label="Response header timeout" placeholder="e.g. 30s"/>
                    <Text state=state path="config.proxy.tls_server_name" label="Upstream TLS server name" placeholder="(optional)"/>
                    <Text state=state path="config.proxy.flush_interval" label="Flush interval" placeholder="-1 for streaming"/>
                </div>
                <KvField state=state path="config.proxy.header_up" label="Headers sent to the upstream" key_placeholder="Header" value_placeholder="Value (or -Header to remove)" add_label="Add header"/>
                <KvField state=state path="config.proxy.header_down" label="Headers changed in the response" key_placeholder="Header" add_label="Add header"/>
            }
            .into_any();
            section("activity", "Load balancing & health checks", || "Policies, retries, timeouts and upstream headers".into(), false, body)
        })
    };

    let routes = move || {
        let count = Memo::new(move |_| a(&get_path(&state.get(), "config"), "routes").len());
        let rows = move || {
            (0..count.get())
                .map(|at| {
                    let base = format!("config.routes.{at}");
                    let rk = Memo::new({
                        let base = base.clone();
                        move |_| s(&get_path(&state.get(), &base), "kind")
                    });
                    let set_kind = {
                        let base = base.clone();
                        move |e: leptos::ev::Event| {
                            let k = event_target_value(&e);
                            state.update(|v| {
                                let mut r = get_path(v, &base);
                                r["kind"] = json!(k);
                                let dflt = match k.as_str() {
                                    "proxy" => json!({ "upstreams": [""] }),
                                    "static" => json!({ "root": "" }),
                                    "redirect" => json!({ "to": "", "code": 302 }),
                                    _ => json!({ "status": 200, "body": "" }),
                                };
                                if !r[&k].is_object() {
                                    r[&k] = dflt;
                                }
                                set_path(v, &base, r);
                            });
                        }
                    };
                    let b2 = base.clone();
                    let inner = move || {
                        let base = b2.clone();
                        match rk.get().as_str() {
                            "proxy" => view! { <ListField state=state path=format!("{base}.proxy.upstreams") placeholder="localhost:9000" add_label="Add upstream"/> }.into_any(),
                            "static" => view! { <Text state=state path=format!("{base}.static.root") label="Folder" placeholder="/srv/files"/> }.into_any(),
                            "redirect" => view! { <Text state=state path=format!("{base}.redirect.to") label="Redirect to" placeholder="https://…"/> }.into_any(),
                            _ => view! {
                                <div class="form-grid">
                                    <Text state=state path=format!("{base}.respond.status") kind="number" label="Status"/>
                                    <Text state=state path=format!("{base}.respond.body") label="Body"/>
                                </div>
                            }
                            .into_any(),
                        }
                    };
                    let kind_opts = [("proxy", "Reverse proxy"), ("static", "File server"), ("redirect", "Redirect"), ("respond", "Response")]
                        .into_iter()
                        .map(|(v, l)| view! { <option value=v selected=move || rk.get() == v>{l}</option> })
                        .collect_view();
                    view! {
                        <div class="card stack" style="padding:12px;gap:8px">
                            <div class="row wrap">
                                <div style="max-width:220px"><Text state=state path=format!("{base}.path") label="Path" placeholder="/api/*" mono=true/></div>
                                <label class="field" style="max-width:190px"><span>"Handler"</span><select class="input" on:change=set_kind>{kind_opts}</select></label>
                                <div style="align-self:flex-end"><Check state=state path=format!("{base}.strip_prefix") label="Strip prefix"/></div>
                                <button class="btn sm" style="margin-left:auto;align-self:flex-end" type="button" title="Remove route"
                                    on:click=move |_| with_list(state, "config.routes", |l| { if at < l.len() { l.remove(at); } }) inner_html=icon("trash")></button>
                            </div>
                            {inner}
                        </div>
                    }
                })
                .collect_view()
        };
        let body = view! {
            <p class="muted small" style="margin:0">"For example " <code>"/api/*"</code> " to a backend. Checked in order, before the main handler."</p>
            {rows}
            <div><button class="btn sm" type="button"
                on:click=move |_| with_list(state, "config.routes", |l| l.push(json!({ "path": "/api/*", "kind": "proxy", "strip_prefix": false, "proxy": { "upstreams": [""] } })))
                inner_html=ib("plus", "Add route")></button></div>
        }
        .into_any();
        section(
            "route",
            "Path routes",
            move || match count.get() {
                0 => "Send specific paths somewhere else".to_string(),
                n => format!("{n} route(s)"),
            },
            false,
            body,
        )
    };

    let https = move || {
        let mode = Memo::new(move |_| s(&state.get()["config"]["tls"], "mode"));
        let redirect_on = move || state.get()["config"]["http_redirect"] != json!(false);
        let body = view! {
            <div class="form-grid">
                <Select state=state path="config.tls.mode" label="Certificate"
                    options=opts(&[("auto", "Automatic (Let's Encrypt / domain settings)"), ("internal", "Caddy internal CA (LAN / self-signed)"), ("custom", "My own certificate files"), ("off", "Off (plain HTTP only)")])/>
                <Select state=state path="config.www_redirect" label="www handling"
                    options=opts(&[("", "Leave as is"), ("to_apex", "Redirect www → bare domain"), ("to_www", "Redirect bare domain → www")])/>
            </div>
            {move || (mode.get() == "custom").then(|| view! {
                <div class="form-grid">
                    <Text state=state path="config.tls.cert_file" label="Certificate file" placeholder="/certs/site.crt"/>
                    <Text state=state path="config.tls.key_file" label="Key file" placeholder="/certs/site.key"/>
                </div>
            })}
            {move || (mode.get() != "off").then(|| view! {
                <label class="switch"><input type="checkbox" prop:checked=redirect_on
                    on:change=move |e| { let on = event_target_checked(&e); state.update(|v| v["config"]["http_redirect"] = if on { Value::Null } else { json!(false) }); }/><i></i>
                    <span>"Redirect HTTP to HTTPS"</span></label>
            })}
        }
        .into_any();
        section("shield-check", "HTTPS & redirects", || "Certificate source, HTTP→HTTPS and www handling".into(), false, body)
    };

    let access = move || {
        let sh = move || state.get()["config"]["access"]["shields"].clone();
        let login = Memo::new(move |_| {
            let v = sh();
            match v.get("login") {
                Some(Value::String(l)) => l.clone(),
                _ => s(&sec_defaults.get(), "login"),
            }
        });
        let apps = Memo::new(move |_| s(&sh(), "apps"));
        let login_cur = move |want: &'static str| {
            let v = sh();
            let now = match v.get("login") {
                Some(Value::String(l)) => l.clone(),
                _ => "__default".into(),
            };
            now == want
        };
        let login_change = move |e: leptos::ev::Event| {
            let v = event_target_value(&e);
            state.update(|s| set_path(s, "config.access.shields.login", if v == "__default" { Value::Null } else { json!(v) }));
        };
        let login_default = move || {
            let d = sec_defaults.get();
            if d.is_null() {
                "Default".to_string()
            } else {
                format!("Default ({})", login_name(&s(&d, "login")))
            }
        };
        let users = Memo::new(move |_| a(&state.get()["config"]["access"], "basic_auth").len());
        let has_fa = Memo::new(move |_| state.get()["config"]["access"]["forward_auth"].is_object());
        let has_rl = Memo::new(move |_| state.get()["config"]["access"]["rate_limit"].is_object());
        let shields = SHIELDS.iter().map(|(k, l)| tri(state, k, l, sec_defaults)).collect_view();
        let (lc1, lc2, lc3, lc4) = (login_cur, login_cur, login_cur, login_cur);
        let body = view! {
            <div class="field">
                <span>"Protection " <a class="small" href="#/security" style="font-weight:400">"(defaults on the Security page)"</a></span>
                <div class="shields">
                    {shields}
                    <label class="row" style="gap:10px;font-size:13.5px"><span class="grow">"Login wall"</span>
                        <select class="input" style="max-width:150px;padding:4px 8px" on:change=login_change>
                            <option value="__default" selected=move || lc1("__default")>{login_default}</option>
                            <option value="" selected=move || lc2("")>"No login wall"</option>
                            <option value="sso" selected=move || lc3("sso")>"Sign-in with two-factor"</option>
                            <option value="basic" selected=move || lc4("basic")>"Shared password"</option>
                        </select></label>
                </div>
            </div>
            {move || (!login.get().is_empty()).then(|| {
                let mut app_opts = vec![("".to_string(), "None: the login wall covers everything".to_string())];
                app_opts.extend(opts(APP_PRESETS));
                app_opts.push(("custom".into(), "Custom paths…".into()));
                view! {
                    <div class="form-grid">
                        <Select state=state path="config.access.shields.apps" label="Phone and TV apps" options=app_opts
                            hint="Apps can't get past a login wall. Picking the app leaves the paths it uses to the app's own sign-in, while the browser pages stay behind the wall."/>
                        {move || (login.get() == "sso").then(|| view! {
                            <ListField state=state path="config.access.shields.sso_users" label="Only these accounts" placeholder="username" add_label="Add account"
                                hint="Empty: every Caddymium account can sign in to this site."/>
                        })}
                    </div>
                    {move || (apps.get() == "custom").then(|| view! {
                        <div class="form-grid">
                            <Select state=state path="config.access.shields.app_mode" label="The login wall covers"
                                options=opts(&[("except", "Everything except these paths"), ("only", "Only these paths")])/>
                            <ListField state=state path="config.access.shields.app_paths" label="Paths" placeholder="/api/*" add_label="Add path"/>
                        </div>
                    })}
                }
            })}
            <div class="field"><span>"Extra passwords for this site only"</span>
                {move || (0..users.get()).map(|at| {
                    let set = Memo::new(move |_| s(&state.get()["config"]["access"]["basic_auth"][at], "hash").starts_with("$2"));
                    view! {
                        <div class="row">
                            <div style="max-width:220px"><Text state=state path=format!("config.access.basic_auth.{at}.username") label="Username"/></div>
                            {move || if set.get() {
                                view! {
                                    <span class="badge ok" style="align-self:center">"password set"</span>
                                    <button class="btn sm" type="button" style="align-self:center"
                                        on:click=move |_| state.update(|v| v["config"]["access"]["basic_auth"][at]["hash"] = json!(""))>"Change"</button>
                                }.into_any()
                            } else {
                                view! { <div style="max-width:240px"><Text state=state path=format!("config.access.basic_auth.{at}.hash") label="New password" kind="password"/></div> }.into_any()
                            }}
                            <button class="btn sm" type="button" title="Remove user" style="align-self:flex-end"
                                on:click=move |_| with_list(state, "config.access.basic_auth", |l| { if at < l.len() { l.remove(at); } }) inner_html=icon("trash")></button>
                        </div>
                    }
                }).collect_view()}
                <div><button class="btn sm" type="button" on:click=move |_| with_list(state, "config.access.basic_auth", |l| l.push(json!({ "username": "", "hash": "" })))
                    inner_html=ib("plus", "Add user")></button></div>
            </div>
            <div class="form-grid">
                <ListField state=state path="config.access.allow_cidrs" label="Only allow these IPs / ranges" placeholder="192.168.1.0/24" add_label="Add range"
                    hint="Empty = everyone. Use private_ranges for LAN only."/>
                <ListField state=state path="config.access.deny_cidrs" label="Block these IPs / ranges" placeholder="203.0.113.7" add_label="Add range"/>
            </div>
            <div class="field"><span>"Single sign-on (forward auth)"</span>
                {move || if has_fa.get() {
                    let preset = move |e: leptos::ev::Event| {
                        let p = event_target_value(&e);
                        state.update(|v| {
                            let fa = &mut v["config"]["access"]["forward_auth"];
                            fa["preset"] = json!(p);
                            if p == "authelia" {
                                fa["uri"] = json!("/api/authz/forward-auth");
                                fa["copy_headers"] = json!(["Remote-User", "Remote-Groups", "Remote-Email", "Remote-Name"]);
                            } else if p == "authentik" {
                                fa["uri"] = json!("/outpost.goauthentik.io/auth/caddy");
                                fa["copy_headers"] = json!(["X-Authentik-Username", "X-Authentik-Groups", "X-Authentik-Email", "X-Authentik-Name", "X-Authentik-Uid"]);
                            }
                        });
                    };
                    let cur = move |p: &'static str| s(&state.get()["config"]["access"]["forward_auth"], "preset") == p;
                    view! {
                        <div class="form-grid">
                            <label class="field"><span>"Provider"</span><select class="input" on:change=preset>
                                <option value="authelia" selected=move || cur("authelia")>"Authelia"</option>
                                <option value="authentik" selected=move || cur("authentik")>"Authentik"</option>
                                <option value="custom" selected=move || cur("custom")>"Other"</option>
                            </select></label>
                            <Text state=state path="config.access.forward_auth.upstream" label="Auth server address" placeholder="authelia:9091"/>
                            <Text state=state path="config.access.forward_auth.uri" label="Verify URI"/>
                        </div>
                        <div><button class="btn sm" type="button"
                            on:click=move |_| state.update(|v| { v["config"]["access"].as_object_mut().map(|o| o.remove("forward_auth")); })>"Remove SSO"</button></div>
                    }.into_any()
                } else {
                    view! {
                        <div><button class="btn sm" type="button" inner_html=ib("fingerprint", "Protect with Authelia / Authentik")
                            on:click=move |_| state.update(|v| v["config"]["access"]["forward_auth"] = json!({ "preset": "authelia", "upstream": "", "uri": "/api/authz/forward-auth", "copy_headers": ["Remote-User", "Remote-Groups", "Remote-Email", "Remote-Name"] }))></button></div>
                    }.into_any()
                }}
            </div>
            <div class="field"><span>"Rate limit "{move || (!available("rate_limit")).then(|| view! { <span class="badge warn">"plugin not installed"</span> })}</span>
                {move || if has_rl.get() {
                    view! {
                        <div class="row wrap">
                            <div style="max-width:130px"><Text state=state path="config.access.rate_limit.events" kind="number" label="Requests"/></div>
                            <div style="max-width:120px"><Text state=state path="config.access.rate_limit.window" label="per"/></div>
                            <span class="muted" style="align-self:center">"per client"</span>
                            <button class="btn sm" type="button" style="align-self:flex-end"
                                on:click=move |_| state.update(|v| { v["config"]["access"].as_object_mut().map(|o| o.remove("rate_limit")); })>"Remove"</button>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div><button class="btn sm" type="button" disabled=move || !available("rate_limit") inner_html=ib("plus", "Add rate limit")
                            on:click=move |_| state.update(|v| v["config"]["access"]["rate_limit"] = json!({ "events": 100, "window": "1m", "key": "" }))></button></div>
                    }.into_any()
                }}
            </div>
        }
        .into_any();
        section("lock", "Access control", || "Protection, login wall, IP rules and rate limits".into(), false, body)
    };

    let headers = move || {
        let comp = move |alg: &'static str| {
            let on = move || a(&state.get()["config"], "compression").iter().any(|x| x.as_str() == Some(alg));
            let change = move |e: leptos::ev::Event| {
                let want = event_target_checked(&e);
                with_list(state, "config.compression", |l| {
                    l.retain(|x| x.as_str() != Some(alg));
                    if want {
                        l.push(json!(alg));
                    }
                });
            };
            view! { <label class="switch"><input type="checkbox" prop:checked=on on:change=change/><i></i><span>{format!("{alg} compression")}</span></label> }
        };
        let body = view! {
            <div class="row wrap" style="gap:20px">{comp("zstd")}{comp("gzip")}</div>
            <KvField state=state path="config.headers_set" label="Set response headers" key_placeholder="Header" add_label="Add header"/>
            <ListField state=state path="config.headers_remove" label="Remove response headers" placeholder="Server" add_label="Add header"/>
            <div class="form-grid">
                <Text state=state path="config.max_body_size" label="Max upload size" placeholder="e.g. 100MB (empty = unlimited)"/>
                <Select state=state path="config._access_log" label="Access log" options=opts(&[("default", "Default (from settings)"), ("on", "On"), ("off", "Off")])/>
            </div>
        }
        .into_any();
        section("gauge", "Headers, compression & limits", || "Response headers, gzip/zstd, upload size, logging".into(), false, body)
    };

    let maintenance = move || {
        let on = Memo::new(move |_| b(&state.get()["config"], "maintenance"));
        let body = view! {
            <Check state=state path="config.maintenance" label="Show a maintenance page instead of the site (HTTP 503)"/>
            {move || on.get().then(|| view! { <Area state=state path="config.maintenance_body" label="Message" rows=3 placeholder="We'll be right back."/> })}
        }
        .into_any();
        section(
            "construction",
            "Maintenance mode",
            move || {
                if on.get() {
                    "On: visitors see a maintenance page".into()
                } else {
                    "Temporarily show a “be right back” page".into()
                }
            },
            false,
            body,
        )
    };

    let custom = move || {
        let raw_set = Memo::new(move |_| !s(&state.get()["config"], "raw").is_empty());
        let open = untrack(|| kind.get() == "none" || raw_set.get());
        let check_raw = move |_| {
            raw_check.set(None);
            let text = s(&state.get_untracked()["config"], "raw");
            rt::spawn(async move {
                let r = api::post("/config/validate", json!({ "text": text, "context": "site" }))
                    .await
                    .unwrap_or_else(|e| json!({ "valid": false, "error": e.message }));
                raw_check.set(Some(r));
            });
        };
        let snips = move || {
            let list = snippets.get();
            (!list.is_empty()).then(|| {
                let rows = list
                    .into_iter()
                    .map(|sn| {
                        let name = s(&sn, "name");
                        let (n1, n2) = (name.clone(), name.clone());
                        let on = move || a(&state.get()["config"], "snippets").iter().any(|x| x.as_str() == Some(n1.as_str()));
                        let change = move |e: leptos::ev::Event| {
                            let want = event_target_checked(&e);
                            let n = n2.clone();
                            with_list(state, "config.snippets", |l| {
                                l.retain(|x| x.as_str() != Some(n.as_str()));
                                if want {
                                    l.push(json!(n));
                                }
                            });
                        };
                        view! { <label class="switch"><input type="checkbox" prop:checked=on on:change=change/><i></i>
                        <span><span class="mono">{name}</span>" "<span class="muted small">{s(&sn, "description")}</span></span></label> }
                    })
                    .collect_view();
                view! { <div class="field"><span>"Include snippets"</span><div class="stack" style="gap:8px">{rows}</div></div> }
            })
        };
        let body = view! {
            {snips}
            <Area state=state path="config.raw" label="Extra directives (inside this site's block)" rows=8
                placeholder="# anything Caddy supports, e.g.\nheader /static/* Cache-Control \"max-age=86400\""/>
            <div class="row wrap">
                <button class="btn sm" type="button" on:click=check_raw inner_html=ib("check", "Check syntax")></button>
                {move || raw_check.get().map(|c| if b(&c, "valid") {
                    view! { <span class="badge ok">"looks valid"</span> }.into_any()
                } else {
                    view! { <ErrorBox message=s(&c, "error")/> }.into_any()
                })}
            </div>
        }
        .into_any();
        section(
            "code",
            "Custom Caddyfile",
            move || if raw_set.get() { "In use".into() } else { "Snippets and extra directives for anything not covered above".into() },
            open,
            body,
        )
    };

    let template_card = move || {
        is_new.then(|| {
        let options = templates().into_iter().map(|(id, name, _)| view! { <option value=id>{name}</option> }).collect_view();
        view! {
            <div class="card row wrap tpl">
                <span class="s-ic" inner_html=icon("sparkles")></span>
                <div class="grow"><strong>"Start from a template"</strong><div class="muted small">"Sensible defaults for common self-hosted apps."</div></div>
                <select class="input" style="max-width:240px" on:change=move |e| apply_template(event_target_value(&e))>
                    <option value="">"Choose an app…"</option>{options}
                </select>
            </div>
        }
    })
    };

    let form = move || {
        if !loaded.get() {
            return view! { <Loading/> }.into_any();
        }
        untrack(|| {
            view! {
                <fieldset class="stack plain" disabled=!editor>
                    {template_card()}
                    <div class="card stack">
                        <div class="row">
                            <div class="grow"><h2 style="margin:0">"Basics"</h2><div class="muted small">"Where this site lives."</div></div>
                            <Check state=state path="enabled" label="Enabled"/>
                        </div>
                        <div class="field"><span>"Hostnames"</span>
                            {move || {
                                host_key.get();
                                let (d, dd) = (domains.get_untracked(), default_domain.get_untracked());
                                untrack(|| view! { <HostInput state=state domains=d default_domain=dd/> })
                            }}
                            <small>"The address people type in their browser. HTTPS certificates are obtained automatically."</small>
                        </div>
                        <div class="form-grid">
                            <Text state=state path="name" label="Display name" placeholder="Defaults to the hostname"/>
                            <Text state=state path="_tags" label="Tags" placeholder="media, home"/>
                        </div>
                    </div>
                    <div class="card stack">
                        <div><h2 style="margin:0">"What should this site do?"</h2><div class="muted small">"Pick how Caddy handles requests."</div></div>
                        <div class="kinds">{kinds()}</div>
                        {kind_fields}
                    </div>
                    {lb}
                    {routes()}
                    {https()}
                    {access()}
                    {headers()}
                    {maintenance()}
                    {custom()}
                </fieldset>
            }
            .into_any()
        })
    };

    let (s1, s2) = (save, save);
    view! {
        <div class="stack" style="gap:14px;margin-bottom:22px">
            <a class="btn sm" style="align-self:flex-start" href="#/sites" inner_html=ib("arrow-left", "Sites")></a>
            <div class="row wrap" style="gap:14px">
                {head}
                {(!is_new && editor).then(|| view! {
                    <button class="btn sm" on:click=duplicate inner_html=ib("copy-plus", "Duplicate")></button>
                    <button class="btn sm danger" on:click=remove inner_html=ib("trash", "Delete")></button>
                })}
            </div>
        </div>
        {form}
        {editor.then(|| view! {
            <div class="actionbar">
                {move || match error.get() {
                    Some(e) => view! { <div class="grow"><ErrorBox title="The site can't be saved" message=e/></div> }.into_any(),
                    None => view! { <span class="muted small grow hide-sm">"Saved changes go live after you review and apply."</span> }.into_any(),
                }}
                <button class="btn" on:click=move |_| s1(false) disabled=move || busy.get()>"Save"</button>
                <button class="btn primary" on:click=move |_| s2(true) disabled=move || busy.get() inner_html=format!("Save & review{}", icon("arrow-right"))></button>
            </div>
        })}
    }
}
