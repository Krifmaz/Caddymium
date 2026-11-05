//! `#/notifications`: where the panel sends alerts (ntfy, Gotify, Discord, Slack, email, webhooks).

use crate::app::{self, a, b, i, s};
use crate::form::{Check, Select, Text};
use crate::icons::icon;
use crate::pages::deploy::ib;
use crate::ui::{dialog, load, reload, show, sure, toast_err, ErrorBox, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

/// A channel type as people know it.
pub fn type_name(t: &str) -> String {
    match t {
        "webhook" => "Webhook",
        "ntfy" => "ntfy",
        "gotify" => "Gotify",
        "discord" => "Discord",
        "slack" => "Slack",
        "email" => "Email (SMTP)",
        other => other,
    }
    .to_string()
}

/// An event as people read it.
pub fn event_name(e: &str) -> String {
    match e {
        "apply_failed" => "Config apply failed",
        "cert_expiring" => "Certificate expiring",
        "cert_error" => "Certificate error",
        "upstream_down" => "Upstream down",
        "upstream_up" => "Upstream back up",
        "update_available" => "Caddy update available",
        "update_installed" => "Caddy updated automatically",
        "update_failed" => "Automatic Caddy update failed",
        "backup_failed" => "Backup failed",
        "discovery" => "New container discovered",
        "caddy_down" => "Caddy is down",
        "caddy_recovered" => "Caddy rolled back or restarted",
        "bug_report" => "Bug report filed",
        other => other,
    }
    .to_string()
}

fn field_label(f: &str) -> String {
    match f {
        "url" => "URL",
        "token" => "Token",
        "priority" => "Priority",
        "header" => "Extra header (Name: value)",
        "host" => "SMTP host",
        "port" => "Port",
        "username" => "Username",
        "password" => "Password",
        "from" => "From address",
        "to" => "To address",
        "tls" => "TLS (starttls / tls / none)",
        other => other,
    }
    .to_string()
}

fn is_secret(f: &str) -> bool {
    matches!(f, "token" | "password" | "header")
}

/// The Add/Edit channel dialog.
fn edit_channel(c: Value, meta: Value, done: impl Fn() + Clone + 'static) {
    let id = i(&c, "id");
    let state = RwSignal::new(if id == 0 { json!({ "name": "", "type": "ntfy", "config": {}, "events": [], "enabled": true }) } else { c });
    let error = RwSignal::new(None::<String>);
    let testing = RwSignal::new(false);
    let types = meta.get("notification_types").cloned().unwrap_or(Value::Null);
    let events = a(&meta, "notification_events");
    let type_opts: Vec<(String, String)> =
        types.as_object().map(|o| o.keys().map(|k| (k.clone(), type_name(k))).collect()).unwrap_or_default();
    dialog(
        if id == 0 { "Add channel" } else { "Edit channel" },
        false,
        move |_| {
            let fields = move || {
                let t = s(&state.get(), "type");
                a(&types, &t)
                    .into_iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .map(|f| {
                        let kind = if is_secret(&f) { "password" } else { "text" };
                        view! { <Text state=state path=format!("config.{f}") label=field_label(&f) kind=kind/> }
                    })
                    .collect_view()
            };
            let evs = events
                .iter()
                .filter_map(|e| e.as_str().map(str::to_string))
                .map(|e| {
                    let (e1, e2) = (e.clone(), e.clone());
                    let on = move || a(&state.get(), "events").iter().any(|x| x.as_str() == Some(e1.as_str()));
                    let toggle = move |ev: leptos::ev::Event| {
                        let checked = event_target_checked(&ev);
                        let e = e2.clone();
                        state.update(|st| {
                            let mut list: Vec<Value> = a(st, "events").into_iter().filter(|x| x.as_str() != Some(e.as_str())).collect();
                            if checked {
                                list.push(Value::String(e));
                            }
                            st["events"] = Value::Array(list);
                        });
                    };
                    view! { <label class="row small"><input type="checkbox" prop:checked=on on:change=toggle/>{event_name(&e)}</label> }
                })
                .collect_view();
            view! {
                <div class="stack">
                    <div class="form-grid">
                        <Text state=state path="name" label="Name" placeholder="My phone"/>
                        <Select state=state path="type" label="Type" options=type_opts.clone()/>
                    </div>
                    {fields}
                    <div class="field"><span class="small muted" style="font-weight:600">"Events (none ticked = all)"</span>
                        <div class="ev-grid">{evs}</div>
                    </div>
                    <Check state=state path="enabled" label="Enabled"/>
                    {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
                </div>
            }
        },
        move |c| {
            let (c1, c2) = (c.clone(), c);
            let (d1, d2) = (done.clone(), done);
            let save = move |_| {
                let (c, d) = (c1.clone(), d1.clone());
                let body = state.get_untracked();
                rt::spawn(async move {
                    let r = if id == 0 {
                        api::post("/notifications", body).await
                    } else {
                        api::put(&format!("/notifications/{id}"), body).await
                    };
                    match r {
                        Ok(_) => {
                            c.close();
                            toast("Saved", "ok");
                            d();
                        }
                        Err(e) => error.set(Some(e.message)),
                    }
                });
            };
            let test = move |_| {
                let body = state.get_untracked();
                testing.set(true);
                error.set(None);
                rt::spawn(async move {
                    match api::post("/notifications/test", body).await {
                        Ok(_) => toast("Test message sent", "ok"),
                        Err(e) => error.set(Some(format!("The test failed: {}", e.message))),
                    }
                    testing.set(false);
                });
            };
            let delete = move |_| {
                let (c, d) = (c2.clone(), d2.clone());
                let name = s(&state.get_untracked(), "name");
                rt::spawn(async move {
                    if !sure(&format!("Delete {name}?"), "Alerts stop going to this channel.", "Delete", true).await {
                        return;
                    }
                    match api::del(&format!("/notifications/{id}")).await {
                        Ok(_) => {
                            c.close();
                            d();
                        }
                        Err(e) => toast_err(e),
                    }
                });
            };
            view! {
                {(id != 0).then(|| view! { <button class="btn danger" style="margin-right:auto" on:click=delete>"Delete"</button> })}
                <button class="btn" disabled=move || testing.get() on:click=test
                    inner_html=move || ib("bell", if testing.get() { "Sending…" } else { "Send test" })></button>
                <button class="btn primary" on:click=save>"Save"</button>
            }
        },
    );
}

/// `#/notifications`.
#[component]
pub fn Notifications() -> impl IntoView {
    let list = load("/notifications");
    let refresh = move || reload(list, "/notifications");
    let open = move |c: Value| {
        rt::spawn(async move {
            match app::meta().await {
                Ok(m) => edit_channel(c, m, refresh),
                Err(e) => toast_err(e),
            }
        });
    };
    let body = show(list, move |v| {
        let chans = v.as_array().cloned().unwrap_or_default();
        if chans.is_empty() {
            return view! {
                <div class="card empty" inner_html=format!("{}<p>No channels yet. ntfy, Gotify, Discord, Slack, email and plain webhooks work.</p>", icon("bell"))></div>
            }
            .into_any();
        }
        let rows = chans
            .into_iter()
            .map(|c| {
                let evs = a(&c, "events");
                let evs = if evs.is_empty() {
                    "all events".to_string()
                } else {
                    evs.iter().filter_map(|e| e.as_str()).map(event_name).collect::<Vec<_>>().join(", ")
                };
                let cc = c.clone();
                view! {
                    <tr class="clickable" on:click=move |_| open(cc.clone())>
                        <td><strong>{s(&c, "name")}</strong></td>
                        <td>{type_name(&s(&c, "type"))}</td>
                        <td class="hide-sm small">{evs}</td>
                        <td>{if b(&c, "enabled") { view! { <span class="badge ok">"on"</span> } } else { view! { <span class="badge">"off"</span> } }}</td>
                    </tr>
                }
            })
            .collect_view();
        view! {
            <div class="card flat table-wrap"><table class="table">
                <thead><tr><th>"Name"</th><th>"Type"</th><th class="hide-sm">"Events"</th><th>"Status"</th></tr></thead>
                <tbody>{rows}</tbody>
            </table></div>
        }
        .into_any()
    });
    view! {
        <PageHead title="Notifications" text="Get told when an apply fails or something needs a look.">
            <button class="btn primary" on:click=move |_| open(Value::Null) inner_html=ib("plus", "Add channel")></button>
        </PageHead>
        {body}
    }
}
