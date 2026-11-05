//! `#/domains`: how certificates are validated per domain, and the DNS provider accounts for DNS validation.

use crate::app::{self, a, b, i, s};
use crate::form::{Check, Select, Text};
use crate::pages::deploy::ib;
use crate::ui::{dialog, load, reload, show, sure, toast_err, Alert, ErrorBox, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

/// A challenge as people know it.
pub fn challenge_label(c: &str) -> &str {
    match c {
        "http-01" => "HTTP",
        "tls-alpn-01" => "TLS-ALPN",
        "dns-01" => "DNS",
        other => other,
    }
}

const CHALLENGES: &[(&str, &str)] = &[
    ("http-01", "HTTP (default; port 80 must be reachable from the internet)"),
    ("tls-alpn-01", "TLS-ALPN (port 443 must be reachable)"),
    ("dns-01", "DNS (uses your DNS provider's API; works without open ports)"),
];

/// A number from a select or a field ("" and "0" are none).
pub fn id_or_null(v: &Value) -> Value {
    let n = match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(t) => t.parse().unwrap_or(0),
        _ => 0,
    };
    if n > 0 {
        Value::from(n)
    } else {
        Value::Null
    }
}

/// (value, label) options for the DNS accounts, "Choose…" first.
pub fn cred_options(creds: &[Value]) -> Vec<(String, String)> {
    std::iter::once((String::new(), "Choose…".to_string())).chain(creds.iter().map(|c| (i(c, "id").to_string(), s(c, "name")))).collect()
}

fn edit_domain(d: Value, creds: Vec<Value>, done: impl Fn() + Clone + 'static) {
    let id = i(&d, "id");
    let state =
        RwSignal::new(if id == 0 { json!({ "name": "", "challenge": "http-01", "credential_id": "", "wildcard": false }) } else { d });
    let error = RwSignal::new(None::<String>);
    let no_creds = creds.is_empty();
    let opts = cred_options(&creds);
    let challenges: Vec<(String, String)> = CHALLENGES.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect();
    dialog(
        if id == 0 { "Add domain" } else { "Edit domain" },
        false,
        move |_| {
            let dns = move || s(&state.get(), "challenge") == "dns-01";
            view! {
                <div class="stack">
                    <Text state=state path="name" label="Domain" placeholder="example.com" hint="Applies to this domain and all its subdomains."/>
                    <Select state=state path="challenge" label="How to prove you own it" options=challenges.clone()/>
                    {move || dns().then(|| view! {
                        <Select state=state path="credential_id" label="DNS account" options=opts.clone()
                            hint=if no_creds { "Add a DNS provider account first." } else { "" }/>
                        <Check state=state path="wildcard" label="Use one wildcard certificate (*.domain) for all subdomains"/>
                    })}
                    {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
                </div>
            }
        },
        move |c| {
            let c2 = c.clone();
            let save = move |_| {
                let (c, d) = (c.clone(), done.clone());
                let mut body = state.get_untracked();
                body["credential_id"] = if s(&body, "challenge") == "dns-01" { id_or_null(&body["credential_id"]) } else { Value::Null };
                rt::spawn(async move {
                    let r = if id == 0 { api::post("/domains", body).await } else { api::put(&format!("/domains/{id}"), body).await };
                    match r {
                        Ok(_) => {
                            c.close();
                            toast("Domain saved. Apply to use it.", "ok");
                            d();
                            app::refresh_status();
                        }
                        Err(e) => error.set(Some(e.message)),
                    }
                });
            };
            view! {
                <button class="btn" on:click=move |_| c2.close()>"Cancel"</button>
                <button class="btn primary" on:click=save>"Save"</button>
            }
        },
    );
}

fn edit_cred(c: Value, providers: Vec<Value>, done: impl Fn() + Clone + 'static) {
    let id = i(&c, "id");
    let first = providers.iter().find(|p| b(p, "available")).or(providers.first()).map(|p| s(p, "id")).unwrap_or_default();
    let set = c.get("set").cloned().unwrap_or(Value::Null);
    let state = RwSignal::new(if id == 0 {
        json!({ "name": "", "provider": first, "fields": {} })
    } else {
        json!({ "name": s(&c, "name"), "provider": s(&c, "provider"), "fields": c.get("fields").cloned().unwrap_or(json!({})) })
    });
    let error = RwSignal::new(None::<String>);
    let opts: Vec<(String, String)> = providers
        .iter()
        .map(|p| (s(p, "id"), format!("{}{}", s(p, "name"), if b(p, "available") { "" } else { " (plugin not installed)" })))
        .collect();
    dialog(
        if id == 0 { "Add DNS account" } else { "Edit DNS account" },
        false,
        move |_| {
            let body = move || {
                let pid = s(&state.get(), "provider");
                let Some(p) = providers.iter().find(|p| s(p, "id") == pid).cloned() else { return ().into_any() };
                let docs = s(&p, "docs_url");
                let (missing, plugin, privilege, pname) = (!b(&p, "available"), s(&p, "plugin"), s(&p, "privilege"), s(&p, "name"));
                let fields = a(&p, "fields")
                    .into_iter()
                    .map(|f| {
                        let (key, label, secret) = (s(&f, "key"), s(&f, "label"), b(&f, "secret"));
                        let optional = !b(&f, "required") && !label.to_lowercase().contains("optional");
                        let ph = if secret && b(&set, &key) { "•••••• (saved; leave empty to keep)".to_string() } else { s(&f, "default") };
                        view! {
                            <Text state=state path=format!("fields.{key}") label=format!("{label}{}", if optional { " (optional)" } else { "" })
                                kind=if secret { "password" } else { "text" } placeholder=ph hint=s(&f, "help")/>
                        }
                    })
                    .collect_view();
                view! {
                    {missing.then(|| view! {
                        <Alert kind="warn">"Your Caddy build doesn't include the " <code>{plugin}</code>
                            " plugin. Add it under " <a href="#/plugins">"Plugins"</a> " before using this provider."</Alert>
                    })}
                    <Alert kind="info">{privilege}" "{(!docs.is_empty()).then(|| view! { <a href=docs target="_blank" rel="noreferrer">"How to create a token"</a> })}</Alert>
                    <Text state=state path="name" label="Name" placeholder=pname/>
                    {fields}
                    <p class="muted small">"Tip: instead of the value you can enter " <code>"env:VAR_NAME"</code> " or "
                        <code>"file:/run/secrets/name"</code> " to read it from the Caddy container."</p>
                }
                .into_any()
            };
            view! {
                <div class="stack">
                    {if id == 0 {
                        view! { <Select state=state path="provider" label="Provider" options=opts.clone()/> }.into_any()
                    } else {
                        view! { <Text state=state path="provider" label="Provider" disabled=true/> }.into_any()
                    }}
                    {body}
                    {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
                </div>
            }
        },
        move |c| {
            let c2 = c.clone();
            let save = move |_| {
                let (c, d) = (c.clone(), done.clone());
                let body = state.get_untracked();
                rt::spawn(async move {
                    let r = if id == 0 {
                        api::post("/dns-credentials", body).await
                    } else {
                        api::put(&format!("/dns-credentials/{id}"), body).await
                    };
                    match r {
                        Ok(_) => {
                            c.close();
                            toast("DNS account saved", "ok");
                            d();
                            app::refresh_status();
                        }
                        Err(e) => error.set(Some(e.message)),
                    }
                });
            };
            view! {
                <button class="btn" on:click=move |_| c2.close()>"Cancel"</button>
                <button class="btn primary" on:click=save>"Save"</button>
            }
        },
    );
}

/// `#/domains`.
#[component]
pub fn Domains() -> impl IntoView {
    let domains = load("/domains");
    let creds = load("/dns-credentials");
    let providers = RwSignal::new(Vec::<Value>::new());
    rt::spawn(async move {
        match app::meta().await {
            Ok(m) => providers.set(a(&m, "dns_providers")),
            Err(e) => toast_err(e),
        }
    });
    let refresh = move || {
        reload(domains, "/domains");
        reload(creds, "/dns-credentials");
    };
    let cred_list = move || creds.get_untracked().and_then(|r| r.ok()).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let editable = app::is_admin();
    let domain_rows = show(domains, move |v| {
        let list = v.as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            return view! { <p class="muted">"No domains configured. Every site uses Caddy's automatic HTTPS."</p> }.into_any();
        }
        let cl = creds.get().and_then(|r| r.ok()).and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let rows = list
            .into_iter()
            .map(|d| {
                let (dd, id, name) = (d.clone(), i(&d, "id"), s(&d, "name"));
                let cid = i(&d, "credential_id");
                let cred = if cid == 0 { "—".to_string() } else { cl.iter().find(|c| i(c, "id") == cid).map(|c| s(c, "name")).unwrap_or("?".into()) };
                let remove = move |_| {
                    let name = name.clone();
                    rt::spawn(async move {
                        if !sure(&format!("Remove {name}?"), "Sites under it go back to default certificate handling.", "Remove", true).await {
                            return;
                        }
                        match api::del(&format!("/domains/{id}")).await {
                            Ok(_) => {
                                refresh();
                                app::refresh_status();
                            }
                            Err(e) => toast_err(e),
                        }
                    });
                };
                view! {
                    <tr>
                        <td><strong>{s(&d, "name")}</strong>" "{b(&d, "wildcard").then(|| view! { <span class="badge accent">{format!("*.{}", s(&d, "name"))}</span> })}</td>
                        <td>{challenge_label(&s(&d, "challenge")).to_string()}</td>
                        <td class="hide-sm">{cred}</td>
                        <td class="right">{editable.then(|| view! {
                            <button class="btn sm" on:click=move |_| edit_domain(dd.clone(), cred_list(), refresh) inner_html=ib("pencil", "Edit")></button>" "
                            <button class="btn sm danger" on:click=remove>"Remove"</button>
                        })}</td>
                    </tr>
                }
            })
            .collect_view();
        view! {
            <div class="table-wrap"><table class="table">
                <thead><tr><th>"Domain"</th><th>"Validation"</th><th class="hide-sm">"DNS account"</th><th></th></tr></thead>
                <tbody>{rows}</tbody>
            </table></div>
        }
        .into_any()
    });
    let cred_rows = show(creds, move |v| {
        let list = v.as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            return view! { <p class="muted">"API credentials for your DNS provider, used for DNS validation. Stored encrypted."</p> }
                .into_any();
        }
        let rows = list
            .into_iter()
            .map(|c| {
                let (cc, id, name) = (c.clone(), i(&c, "id"), s(&c, "name"));
                let pid = s(&c, "provider");
                let p = providers.get().into_iter().find(|p| s(p, "id") == pid);
                let missing = p.as_ref().is_some_and(|p| !b(p, "available"));
                let pname = p.map(|p| s(&p, "name")).unwrap_or(pid);
                let remove = move |_| {
                    let name = name.clone();
                    rt::spawn(async move {
                        if !sure(&format!("Delete DNS account {name}?"), "Domains using it can't renew certificates over DNS any more.", "Delete", true).await {
                            return;
                        }
                        match api::del(&format!("/dns-credentials/{id}")).await {
                            Ok(_) => refresh(),
                            Err(e) => toast_err(e),
                        }
                    });
                };
                view! {
                    <tr>
                        <td><strong>{s(&c, "name")}</strong></td>
                        <td>{pname}" "{missing.then(|| view! { <span class="badge warn">"plugin missing"</span> })}</td>
                        <td class="right">{editable.then(|| view! {
                            <button class="btn sm" on:click=move |_| edit_cred(cc.clone(), providers.get_untracked(), refresh) inner_html=ib("pencil", "Edit")></button>" "
                            <button class="btn sm danger" on:click=remove>"Delete"</button>
                        })}</td>
                    </tr>
                }
            })
            .collect_view();
        view! {
            <div class="table-wrap"><table class="table">
                <thead><tr><th>"Name"</th><th>"Provider"</th><th></th></tr></thead>
                <tbody>{rows}</tbody>
            </table></div>
        }
        .into_any()
    });
    view! {
        <PageHead title="Domains & DNS" text="Optional. By default Caddy gets a certificate for each hostname over HTTP. Add a domain here to use DNS validation (works behind firewalls, needed for wildcard certificates)."/>
        <div class="stack">
            <div class="card stack">
                <div class="row wrap">
                    <div class="grow"><h2 style="margin:0">"Domains"</h2><div class="muted small">"Certificate validation per domain"</div></div>
                    {editable.then(|| view! { <button class="btn sm" on:click=move |_| edit_domain(Value::Null, cred_list(), refresh) inner_html=ib("plus", "Add domain")></button> })}
                </div>
                {domain_rows}
            </div>
            <div class="card stack">
                <div class="row wrap">
                    <div class="grow"><h2 style="margin:0">"DNS provider accounts"</h2><div class="muted small">"API tokens, encrypted at rest"</div></div>
                    {editable.then(|| view! { <button class="btn sm" on:click=move |_| edit_cred(Value::Null, providers.get_untracked(), refresh) inner_html=ib("key", "Add account")></button> })}
                </div>
                {cred_rows}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_from_selects() {
        assert_eq!(id_or_null(&json!("3")), json!(3));
        assert_eq!(id_or_null(&json!("")), Value::Null);
        assert_eq!(id_or_null(&json!(0)), Value::Null);
        assert_eq!(id_or_null(&json!(7)), json!(7));
        assert_eq!(cred_options(&[json!({"id": 2, "name": "cf"})])[1], ("2".to_string(), "cf".to_string()));
        assert_eq!(challenge_label("dns-01"), "DNS");
    }
}
