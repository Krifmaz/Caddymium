//! `#/ddns`: Dynamic DNS, keeping A/AAAA records pointed at this server (settings.ddns).

use crate::app::{self, a, b, s};
use crate::form::{Check, Select, Text};
use crate::pages::deploy::ib;
use crate::pages::domains::{cred_options, id_or_null};
use crate::ui::{toast_err, when, Alert, ErrorBox, Loading, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

const KINDS: &[(&str, &str)] = &[
    ("simple_http", "Ask a website (simple_http)"),
    ("upnp", "Ask the router (UPnP)"),
    ("interface", "Network interface"),
    ("static", "Fixed address"),
];

fn placeholder(kind: &str) -> &'static str {
    match kind {
        "simple_http" => "https://icanhazip.com",
        "interface" => "eth0",
        "static" => "203.0.113.5",
        _ => "",
    }
}

/// Fills in what the form needs: lists, defaults, and each zone's names as text (`names_text`).
pub fn prepare(mut settings: Value) -> Value {
    let d = settings.get("ddns").cloned().filter(Value::is_object).unwrap_or(json!({}));
    let mut d = d;
    if !d["domains"].is_array() {
        d["domains"] = json!([]);
    }
    if !d["ip_sources"].is_array() {
        d["ip_sources"] = json!([]);
    }
    if s(&d, "check_interval").is_empty() {
        d["check_interval"] = json!("5m");
    }
    if d["ipv4"].is_null() {
        d["ipv4"] = json!(true);
    }
    if let Some(l) = d["domains"].as_array_mut() {
        for z in l {
            let names: Vec<String> = a(z, "names").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            z["names_text"] = json!(names.join(", "));
        }
    }
    let cid = d["credential_id"].as_i64().map(|n| n.to_string()).unwrap_or_default();
    d["credential_id"] = json!(cid);
    settings["ddns"] = d;
    settings
}

/// What goes back to `PUT /settings`: names split, empty rows dropped, the account as a number.
pub fn to_save(form: &Value) -> Value {
    let mut body = form.clone();
    let d = &mut body["ddns"];
    let zones: Vec<Value> = a(d, "domains")
        .iter()
        .map(|z| {
            let names: Vec<Value> = s(z, "names_text").split(',').map(str::trim).filter(|x| !x.is_empty()).map(|x| json!(x)).collect();
            json!({ "zone": s(z, "zone").trim(), "names": names })
        })
        .filter(|z| !s(z, "zone").is_empty())
        .collect();
    d["domains"] = Value::Array(zones);
    let sources: Vec<Value> =
        a(d, "ip_sources").into_iter().filter(|x| s(x, "kind") == "upnp" || !s(x, "value").trim().is_empty()).collect();
    d["ip_sources"] = Value::Array(sources);
    d["credential_id"] = id_or_null(&d["credential_id"]);
    if let Some(o) = body.as_object_mut() {
        o.remove("acme_eab_mac_set");
        o.remove("crowdsec_api_key_set");
    }
    body
}

fn push(state: RwSignal<Value>, list: &'static str, item: Value) {
    state.update(|v| {
        if let Some(l) = v["ddns"][list].as_array_mut() {
            l.push(item);
        }
    });
}

fn remove(state: RwSignal<Value>, list: &'static str, at: usize) {
    state.update(|v| {
        if let Some(l) = v["ddns"][list].as_array_mut() {
            if at < l.len() {
                l.remove(at);
            }
        }
    });
}

#[component]
fn Status(st: Value) -> impl IntoView {
    let records = a(&st, "records");
    let rows = records
        .iter()
        .map(|r| {
            let addrs: Vec<String> = a(r, "addresses").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            let state = match r.get("up_to_date").and_then(Value::as_bool) {
                Some(true) => view! { <span class="badge ok">"up to date"</span> }.into_any(),
                Some(false) => view! { <span class="badge warn">"differs"</span> }.into_any(),
                None => view! { <span class="badge">"unknown"</span> }.into_any(),
            };
            view! {
                <tr>
                    <td class="mono">{s(r, "name")}</td>
                    <td class="mono small">{if addrs.is_empty() { "—".to_string() } else { addrs.join(", ") }}</td>
                    <td>{state}</td>
                </tr>
            }
        })
        .collect_view();
    let or_dash = |x: String, d: &str| if x.is_empty() { d.to_string() } else { x };
    view! {
        <div class="card stack">
            <div><h2 style="margin:0">"Status"</h2>
                <div class="muted small">{format!("Checked {} · public address from {}", when(&s(&st, "checked_at")), s(&st, "source"))}</div></div>
            <div class="form-grid">
                <div class="field"><span>"Public IPv4"</span><div class="mono">{or_dash(s(&st, "public_ipv4"), "— (not reachable over IPv4)")}</div></div>
                <div class="field"><span>"Public IPv6"</span><div class="mono">{or_dash(s(&st, "public_ipv6"), "—")}</div></div>
            </div>
            {(!records.is_empty()).then(|| view! {
                <div class="table-wrap"><table class="table">
                    <thead><tr><th>"Name"</th><th>"Resolves to"</th><th>"State"</th></tr></thead>
                    <tbody>{rows}</tbody>
                </table></div>
                <p class="muted small">"“Differs” right after a change is normal: Caddy updates on the next check interval, and DNS caches take a while. The names are resolved from the panel's network, which may differ from the public internet."</p>
            })}
        </div>
    }
}

/// `#/ddns`.
#[component]
pub fn Ddns() -> impl IntoView {
    let state = RwSignal::new(Value::Null);
    let creds = RwSignal::new(Vec::<Value>::new());
    let meta = RwSignal::new(Value::Null);
    let status = RwSignal::new(None::<Value>);
    let checking = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let check = move || {
        checking.set(true);
        rt::spawn(async move {
            match api::get("/ddns/status").await {
                Ok(v) => status.set(Some(v)),
                Err(e) => toast_err(e),
            }
            checking.set(false);
        });
    };
    rt::spawn(async move {
        if let Ok(m) = app::meta().await {
            meta.set(m);
        }
        creds.set(api::get("/dns-credentials").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default());
        match api::get("/settings").await {
            Ok(v) => {
                let v = prepare(v);
                let on = b(&v["ddns"], "enabled");
                state.set(v);
                if on {
                    check();
                }
            }
            Err(e) => error.set(Some(e.message)),
        }
    });
    let save = move |_| {
        error.set(None);
        busy.set(true);
        let body = to_save(&state.get_untracked());
        rt::spawn(async move {
            match api::put("/settings", body).await {
                Ok(v) => {
                    let v = prepare(v);
                    let on = b(&v["ddns"], "enabled");
                    state.set(v);
                    toast("Saved. Apply to update Caddy.", "ok");
                    app::refresh_status();
                    if on {
                        check();
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };
    let admin = app::is_admin();
    let warnings = move || {
        let m = meta.get();
        let feature_missing = a(&m, "features").iter().any(|f| s(f, "id") == "dynamic_dns" && !b(f, "available"));
        let cid = s(&state.get()["ddns"], "credential_id");
        let prov = creds.get().into_iter().find(|c| crate::app::i(c, "id").to_string() == cid).map(|c| s(&c, "provider"));
        let pm = prov.and_then(|p| a(&m, "dns_providers").into_iter().find(|x| s(x, "id") == p)).filter(|p| !b(p, "available"));
        view! {
            {feature_missing.then(|| view! {
                <Alert kind="warn">"The " <code>"caddy-dynamicdns"</code> " plugin isn't in your Caddy build. "
                    <a href="#/plugins">"Install it under Plugins"</a> "; until then these settings are saved but skipped."</Alert>
            })}
            {pm.map(|p| view! {
                <Alert kind="warn">{format!("The {} DNS plugin isn't installed. ", s(&p, "name"))}<a href="#/plugins">"Add it under Plugins"</a>"."</Alert>
            })}
        }
    };
    let zones = move || {
        let n = a(&state.get()["ddns"], "domains").len();
        (0..n)
            .map(|at| {
                view! {
                    <div class="row">
                        <div style="max-width:240px"><Text state=state path=format!("ddns.domains.{at}.zone") label="Zone" placeholder="example.com"/></div>
                        <div class="grow"><Text state=state path=format!("ddns.domains.{at}.names_text") label="Names" placeholder="@, www, home"/></div>
                        <button class="btn sm" type="button" on:click=move |_| remove(state, "domains", at)>"Remove"</button>
                    </div>
                }
            })
            .collect_view()
    };
    let sources = move || {
        let list = a(&state.get()["ddns"], "ip_sources");
        let kinds: Vec<(String, String)> = KINDS.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect();
        list.iter()
            .enumerate()
            .map(|(at, src)| {
                let kind = s(src, "kind");
                view! {
                    <div class="row">
                        <div style="max-width:260px"><Select state=state path=format!("ddns.ip_sources.{at}.kind") label="Source" options=kinds.clone()/></div>
                        {(kind != "upnp").then(|| view! {
                            <div class="grow"><Text state=state path=format!("ddns.ip_sources.{at}.value") label="Value" placeholder=placeholder(&kind)/></div>
                        })}
                        <button class="btn sm" type="button" on:click=move |_| remove(state, "ip_sources", at)>"Remove"</button>
                    </div>
                }
            })
            .collect_view()
    };
    // Rows are drawn by count/kind only, so typing doesn't redraw (and blur) them.
    let zone_count = Memo::new(move |_| a(&state.get()["ddns"], "domains").len());
    let source_kinds = Memo::new(move |_| a(&state.get()["ddns"], "ip_sources").iter().map(|x| s(x, "kind")).collect::<Vec<_>>());
    let zones = move || {
        zone_count.get();
        untrack(zones)
    };
    let sources = move || {
        source_kinds.get();
        untrack(sources)
    };
    let enabled = Memo::new(move |_| b(&state.get()["ddns"], "enabled"));
    let form = move || {
        if state.get().is_null() {
            return error.get().map(|e| view! { <ErrorBox message=e/> }.into_any()).unwrap_or_else(|| view! { <Loading/> }.into_any());
        }
        let opts = cred_options(&creds.get());
        let no_creds = creds.get().is_empty();
        view! {
            <fieldset class="stack plain" disabled=!admin>
                <div class="card stack">
                    <Check state=state path="ddns.enabled" label="Enable Dynamic DNS"/>
                    {move || enabled.get().then(|| view! {
                        <Select state=state path="ddns.credential_id" label="DNS provider account" options=opts.clone()
                            hint=if no_creds { "Add an account under Domains & DNS first. The token needs permission to edit A/AAAA records." } else { "" }/>
                        <div class="field">
                            <span>"Records to keep updated"</span>
                            {zones}
                            <div><button class="btn sm" type="button" on:click=move |_| push(state, "domains", json!({ "zone": "", "names": [], "names_text": "@" }))
                                inner_html=ib("plus", "Add zone")></button></div>
                            <small>"The zone, then the names inside it separated by commas: " <code>"@"</code> " is the zone itself, "
                                <code>"www"</code> " becomes www.example.com."</small>
                        </div>
                        <Check state=state path="ddns.dynamic_domains" label="Also update every hostname served by Caddy that belongs to these zones"/>
                        <div class="field">
                            <span>"How to find the public address (first one that works)"</span>
                            {sources}
                            <div><button class="btn sm" type="button" on:click=move |_| push(state, "ip_sources", json!({ "kind": "simple_http", "value": "https://icanhazip.com" }))
                                inner_html=ib("plus", "Add source")></button></div>
                            <small>"Empty = Caddy's default (asks icanhazip.com). Use UPnP if your router supports it, or an interface name when the server has a public address itself."</small>
                        </div>
                        <div class="form-grid">
                            <Text state=state path="ddns.check_interval" label="Check every" placeholder="5m"/>
                            <Text state=state path="ddns.ttl" label="Record TTL" placeholder="provider default (e.g. 1h)"/>
                        </div>
                        <div class="row wrap">
                            <Check state=state path="ddns.ipv4" label="IPv4 (A records)"/>
                            <Check state=state path="ddns.ipv6" label="IPv6 (AAAA records)"/>
                            <Check state=state path="ddns.update_only" label="Only update records that already exist"/>
                        </div>
                    })}
                </div>
            </fieldset>
            <Alert kind="info">"Dynamic DNS keeps your records correct; HTTPS certificates still come from the domain's validation settings. For a home connection, DNS validation under Domains & DNS avoids opening port 80."</Alert>
            {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
            {admin.then(|| view! { <div><button class="btn primary" on:click=save disabled=move || busy.get()>"Save"</button></div> })}
        }
        .into_any()
    };
    let loaded = Memo::new(move |_| !state.get().is_null() || error.get().is_some());
    let form = move || {
        loaded.get();
        untrack(form)
    };
    view! {
        <PageHead title="Dynamic DNS" text="Keep your domain pointed at this server when your public IP changes. Caddy checks the address on a schedule and updates the A/AAAA records through your DNS provider's API, using the same accounts as DNS validation.">
            <button class="btn" on:click=move |_| check() disabled=move || checking.get() inner_html=ib("refresh", "Check now")></button>
        </PageHead>
        <div class="stack">
            {warnings}
            {move || status.get().map(|st| view! { <Status st=st/> })}
            {form}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddns_round_trip() {
        let v = prepare(json!({ "ddns": { "domains": [{ "zone": "example.com", "names": ["@", "www"] }], "credential_id": 4 } }));
        assert_eq!(s(&v["ddns"]["domains"][0], "names_text"), "@, www");
        assert_eq!(s(&v["ddns"], "check_interval"), "5m");
        let mut f = v.clone();
        f["ddns"]["domains"].as_array_mut().unwrap().push(json!({ "zone": " ", "names_text": "x" }));
        f["ddns"]["ip_sources"] = json!([{ "kind": "upnp" }, { "kind": "static", "value": "" }]);
        let out = to_save(&f);
        assert_eq!(out["ddns"]["domains"], json!([{ "zone": "example.com", "names": ["@", "www"] }]));
        assert_eq!(out["ddns"]["ip_sources"], json!([{ "kind": "upnp" }]));
        assert_eq!(out["ddns"]["credential_id"], json!(4));
    }
}
