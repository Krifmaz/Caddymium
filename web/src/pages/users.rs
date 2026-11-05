//! `#/users`: who can use Caddymium, and API keys for scripts.

use crate::app::{b, i, me, s};
use crate::form::{Check, Select, Text};
use crate::icons::icon;
use crate::pages::deploy::ib;
use crate::ui::{ago, dialog, load, reload, show, sure, toast_err, when, Alert, ErrorBox, PageHead};
use kit_leptos::details::CopyButton;
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

/// What each role may do.
pub const ROLES: &[(&str, &str)] = &[
    ("admin", "everything, including users, domains and settings"),
    ("editor", "edit sites and apply changes"),
    ("viewer", "read-only"),
    ("member", "sign in to your protected sites only, no panel"),
];

fn role_options(keys: &[&str], with_help: bool) -> Vec<(String, String)> {
    ROLES
        .iter()
        .filter(|(r, _)| keys.contains(r))
        .map(|(r, h)| (r.to_string(), if with_help { format!("{r}: {h}") } else { r.to_string() }))
        .collect()
}

/// The Add/Edit user dialog.
fn edit_user(u: Value, done: impl Fn() + Clone + 'static) {
    let id = i(&u, "id");
    let state = RwSignal::new(json!({
        "username": s(&u, "username"),
        "role": if id == 0 { "editor".to_string() } else { s(&u, "role") },
        "password": "",
        "disabled": b(&u, "disabled"),
        "reset_2fa": false,
    }));
    let error = RwSignal::new(None::<String>);
    let totp = b(&u, "totp_enabled");
    let me_id = me().get_untracked().get("user").map(|x| i(x, "id")).unwrap_or(-1);
    let title = if id == 0 { "Add user" } else { "Edit user" };
    dialog(
        title,
        false,
        move |_| {
            view! {
                <div class="stack">
                    <Text state=state path="username" label="Username"/>
                    <Select state=state path="role" label="Role" options=role_options(&["admin", "editor", "viewer", "member"], true)/>
                    <Text state=state path="password" kind="password"
                        label=if id == 0 { "Password" } else { "New password (leave empty to keep)" }/>
                    {(id != 0).then(|| view! {
                        <Check state=state path="disabled" label="Disabled (can't sign in)"/>
                        {totp.then(|| view! { <Check state=state path="reset_2fa" label="Reset two-factor (lost phone)"/> })}
                    })}
                    {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
                </div>
            }
        },
        move |c| {
            let (c1, c2, c3) = (c.clone(), c.clone(), c);
            let (d1, d2) = (done.clone(), done);
            let save = move |_| {
                let (c, d) = (c1.clone(), d1.clone());
                let body = state.get_untracked();
                rt::spawn(async move {
                    let r = if id == 0 { api::post("/users", body).await } else { api::put(&format!("/users/{id}"), body).await };
                    match r {
                        Ok(_) => {
                            c.close();
                            toast("User saved", "ok");
                            d();
                        }
                        Err(e) => error.set(Some(e.message)),
                    }
                });
            };
            let delete = move |_| {
                let (c, d) = (c2.clone(), d2.clone());
                let name = s(&state.get_untracked(), "username");
                rt::spawn(async move {
                    if !sure(
                        &format!("Delete {name}?"),
                        "They can't sign in any more. Their past changes stay in the audit log.",
                        "Delete",
                        true,
                    )
                    .await
                    {
                        return;
                    }
                    match api::del(&format!("/users/{id}")).await {
                        Ok(_) => {
                            c.close();
                            d();
                        }
                        Err(e) => error.set(Some(e.message)),
                    }
                });
            };
            view! {
                {(id != 0 && id != me_id).then(|| view! { <button class="btn danger" style="margin-right:auto" on:click=delete>"Delete"</button> })}
                <button class="btn" on:click=move |_| c3.close()>"Cancel"</button>
                <button class="btn primary" on:click=save>"Save"</button>
            }
        },
    );
}

/// The Create API key dialog, then the new key (shown once).
fn create_key(done: impl Fn() + Clone + 'static) {
    let state = RwSignal::new(json!({ "name": "", "role": "viewer", "valid_days": 0 }));
    let error = RwSignal::new(None::<String>);
    dialog(
        "Create API key",
        false,
        move |_| {
            view! {
                <div class="stack">
                    <Text state=state path="name" label="Name" placeholder="backup script"/>
                    <Select state=state path="role" label="Role" options=role_options(&["viewer", "editor", "admin"], false)/>
                    <Text state=state path="valid_days" kind="number" label="Expires after (days, 0 = never)"/>
                    {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
                </div>
            }
        },
        move |c| {
            let c2 = c.clone();
            let create = move |_| {
                let (c, done) = (c.clone(), done.clone());
                let body = state.get_untracked();
                rt::spawn(async move {
                    match api::post("/api-keys", body).await {
                        Ok(k) => {
                            c.close();
                            done();
                            show_key(s(&k, "token"));
                        }
                        Err(e) => error.set(Some(e.message)),
                    }
                });
            };
            view! {
                <button class="btn" on:click=move |_| c2.close()>"Cancel"</button>
                <button class="btn primary" on:click=create>"Create"</button>
            }
        },
    );
}

fn show_key(token: String) {
    let t = token.clone();
    dialog(
        "Your new API key",
        false,
        move |_| {
            view! {
                <div class="stack">
                    <Alert kind="warn">"Copy it now. It won't be shown again."</Alert>
                    <div class="row code"><span class="grow">{token.clone()}</span><CopyButton text=token/></div>
                </div>
            }
        },
        move |c| {
            view! {
                <button class="btn" on:click=move |_| kit_leptos::clipboard::copy_then(&t, "Copied")>"Copy"</button>
                <button class="btn primary" on:click=move |_| c.close()>"Done"</button>
            }
        },
    );
}

/// `#/users`.
#[component]
pub fn Users() -> impl IntoView {
    let tab = RwSignal::new("users");
    let users = load("/users");
    let keys = load("/api-keys");
    let refresh = move || {
        reload(users, "/users");
        reload(keys, "/api-keys");
    };
    let me_id = me().get_untracked().get("user").map(|x| i(x, "id")).unwrap_or(-1);
    let user_rows = show(users, move |v| {
        let rows = v
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|u| {
                let uu = u.clone();
                let last = s(&u, "last_login");
                view! {
                    <tr class="clickable" on:click=move |_| edit_user(uu.clone(), refresh)>
                        <td>
                            <strong>{s(&u, "username")}</strong>" "
                            {(i(&u, "id") == me_id).then(|| view! { <span class="badge">"you"</span> })}" "
                            {b(&u, "disabled").then(|| view! { <span class="badge err">"disabled"</span> })}
                        </td>
                        <td>{s(&u, "role")}</td>
                        <td class="hide-sm">{if b(&u, "totp_enabled") { "on" } else { "—" }}</td>
                        <td class="hide-sm" title=when(&last)>{if last.is_empty() { "never".to_string() } else { ago(&last) }}</td>
                    </tr>
                }
            })
            .collect_view();
        view! {
            <div class="card flat table-wrap"><table class="table">
                <thead><tr><th>"User"</th><th>"Role"</th><th class="hide-sm">"2FA"</th><th class="hide-sm">"Last sign-in"</th></tr></thead>
                <tbody>{rows}</tbody>
            </table></div>
        }
    });
    let key_rows = show(keys, move |v| {
        let list = v.as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            return view! { <div class="card empty">"No API keys yet."</div> }.into_any();
        }
        let rows = list
            .into_iter()
            .map(|k| {
                let (id, name) = (i(&k, "id"), s(&k, "name"));
                let revoke = move |_| {
                    let name = name.clone();
                    rt::spawn(async move {
                        if !sure(&format!("Revoke key {name}?"), "Scripts using it stop working.", "Revoke", true).await {
                            return;
                        }
                        match api::del(&format!("/api-keys/{id}")).await {
                            Ok(_) => refresh(),
                            Err(e) => toast_err(e),
                        }
                    });
                };
                let (used, exp) = (s(&k, "last_used"), s(&k, "expires_at"));
                view! {
                    <tr>
                        <td>{if s(&k, "name").is_empty() { "—".to_string() } else { s(&k, "name") }}</td>
                        <td class="mono small">{format!("{}…", s(&k, "prefix"))}</td>
                        <td>{s(&k, "role")}</td>
                        <td class="hide-sm">{if used.is_empty() { "never".to_string() } else { ago(&used) }}</td>
                        <td class="hide-sm">{if exp.is_empty() { "never".to_string() } else { when(&exp) }}</td>
                        <td><button class="btn sm danger" on:click=revoke>"Revoke"</button></td>
                    </tr>
                }
            })
            .collect_view();
        view! {
            <div class="card flat table-wrap"><table class="table">
                <thead><tr><th>"Name"</th><th>"Key"</th><th>"Role"</th><th class="hide-sm">"Last used"</th><th class="hide-sm">"Expires"</th><th></th></tr></thead>
                <tbody>{rows}</tbody>
            </table></div>
        }
        .into_any()
    });
    view! {
        <PageHead title="Users & API keys" text="Who can use Caddymium, and keys for scripts."/>
        <div class="tabs" style="margin-bottom:16px">
            <button class:active=move || tab.get() == "users" on:click=move |_| tab.set("users")>"Users"</button>
            <button class:active=move || tab.get() == "keys" on:click=move |_| tab.set("keys")>"API keys"</button>
        </div>
        <div class="stack" class:hidden=move || tab.get() != "users">
            <div><button class="btn primary" on:click=move |_| edit_user(Value::Null, refresh) inner_html=ib("plus", "Add user")></button></div>
            {user_rows}
        </div>
        <div class="stack" class:hidden=move || tab.get() != "keys">
            <p class="muted" style="margin:0">
                "Use a key with " <code class="mono">"Authorization: Bearer <key>"</code> " against " <code class="mono">"/api/…"</code>
                ", for example " <code class="mono">"curl -H \"Authorization: Bearer chk_…\" https://panel/api/sites"</code>
                ". A key never has more rights than its owner."
            </p>
            <div><button class="btn primary" on:click=move |_| create_key(refresh) inner_html=format!("{}Create key", icon("key"))></button></div>
            {key_rows}
        </div>
    }
}
