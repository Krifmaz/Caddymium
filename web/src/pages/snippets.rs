//! `#/snippets`: snippets (reusable pieces attached to sites) and raw blocks (copied into the Caddyfile as-is).

use crate::app::{self, b, i, s};
use crate::form::{Area, Text};
use crate::icons::icon;
use crate::pages::deploy::ib;
use crate::ui::{dialog, load, reload, show, sure, toast_err, ErrorBox, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

const SNIPPET_PH: &str = "header {\n\tStrict-Transport-Security \"max-age=31536000\"\n\tX-Frame-Options DENY\n}";
const BLOCK_PH: &str = "old.example.com {\n\treverse_proxy 10.0.0.9:80\n}";

/// The Edit dialog of a snippet (`snippet`) or a raw block (`block`).
fn edit(kind: &'static str, item: Value, done: impl Fn() + Clone + 'static) {
    let id = i(&item, "id");
    let snippet = kind == "snippet";
    let base = if snippet { "/snippets" } else { "/raw-blocks" };
    let state = RwSignal::new(if id != 0 {
        item
    } else if snippet {
        json!({ "name": "", "description": "", "body": "" })
    } else {
        json!({ "name": "", "body": "", "enabled": true })
    });
    let error = RwSignal::new(None::<String>);
    let check = RwSignal::new(None::<Value>);
    let title = match (snippet, id != 0) {
        (true, true) => "Edit snippet",
        (true, false) => "New snippet",
        (false, true) => "Edit raw block",
        (false, false) => "New raw block",
    };
    dialog(
        title,
        true,
        move |_| {
            let validate = move |_| {
                check.set(None);
                let body = json!({ "text": s(&state.get_untracked(), "body"), "context": if snippet { "site" } else { "top" } });
                rt::spawn(async move {
                    let r = api::post("/config/validate", body).await.unwrap_or_else(|e| json!({ "valid": false, "error": e.message }));
                    check.set(Some(r));
                });
            };
            view! {
                <div class="stack">
                    <div class="form-grid">
                        <Text state=state path="name" label="Name" placeholder=if snippet { "security-headers" } else { "legacy sites" }/>
                        {snippet.then(|| view! { <Text state=state path="description" label="Description"/> })}
                    </div>
                    <Area state=state path="body" rows=14 label=if snippet { "Directives" } else { "Caddyfile text" }
                        placeholder=if snippet { SNIPPET_PH } else { BLOCK_PH }/>
                    <div class="row wrap">
                        <button class="btn sm" on:click=validate inner_html=ib("check", "Check syntax")></button>
                        {move || check.get().map(|c| if b(&c, "valid") {
                            view! { <span class="badge ok">"looks valid"</span> }.into_any()
                        } else {
                            view! { <ErrorBox message=s(&c, "error")/> }.into_any()
                        })}
                    </div>
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
                    let r = if id == 0 { api::post(base, body).await } else { api::put(&format!("{base}/{id}"), body).await };
                    match r {
                        Ok(_) => {
                            c.close();
                            toast("Saved. Apply to make it live.", "ok");
                            d();
                            app::refresh_status();
                        }
                        Err(e) => error.set(Some(e.message)),
                    }
                });
            };
            let delete = move |_| {
                let (c, d) = (c2.clone(), d2.clone());
                let name = s(&state.get_untracked(), "name");
                rt::spawn(async move {
                    let what = if name.is_empty() { "this block".to_string() } else { name };
                    if !sure(&format!("Delete {what}?"), "It leaves the Caddyfile with the next apply.", "Delete", true).await {
                        return;
                    }
                    match api::del(&format!("{base}/{id}")).await {
                        Ok(_) => {
                            c.close();
                            d();
                            app::refresh_status();
                        }
                        Err(e) => toast_err(e),
                    }
                });
            };
            view! {
                {(id != 0).then(|| view! { <button class="btn danger" style="margin-right:auto" on:click=delete>"Delete"</button> })}
                <button class="btn" on:click=move |_| c3.close()>"Cancel"</button>
                <button class="btn primary" on:click=save>"Save"</button>
            }
        },
    );
}

/// `#/snippets`.
#[component]
pub fn Snippets() -> impl IntoView {
    let tab = RwSignal::new("snippets");
    let snippets = load("/snippets");
    let blocks = load("/raw-blocks");
    let refresh = move || {
        reload(snippets, "/snippets");
        reload(blocks, "/raw-blocks");
    };
    let count = |sig: crate::ui::Loaded| move || sig.get().and_then(|r| r.ok()).and_then(|v| v.as_array().map(|x| x.len())).unwrap_or(0);
    let (sn_count, bl_count) = (count(snippets), count(blocks));
    let snippet_list = show(snippets, move |v| {
        let list = v.as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            return view! {
                <div class="card empty" inner_html=format!("{}<p>Example: a “security-headers” snippet with HSTS and X-Frame-Options that you tick on every site.</p>", icon("braces"))></div>
            }
            .into_any();
        }
        list.into_iter()
            .map(|sn| {
                let item = sn.clone();
                view! {
                    <button class="card item" on:click=move |_| if app::can_edit() { edit("snippet", item.clone(), refresh) }>
                        <div class="row"><strong class="mono">{format!("({})", s(&sn, "name"))}</strong><span class="muted small">{s(&sn, "description")}</span></div>
                        <pre class="code-block peek">{s(&sn, "body")}</pre>
                    </button>
                }
            })
            .collect_view()
            .into_any()
    });
    let block_list = show(blocks, move |v| {
        let list = v.as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            return view! {
                <div class="card empty" inner_html=format!("{}<p>Raw blocks hold top-level Caddyfile text, for example sites using features Caddymium doesn't model. Imported blocks it couldn't understand also land here, untouched.</p>", icon("file-code"))></div>
            }
            .into_any();
        }
        list.into_iter()
            .map(|bl| {
                let item = bl.clone();
                let toggle_item = bl.clone();
                let toggle = move |e: leptos::ev::Event| {
                    let on = event_target_checked(&e);
                    let mut body = toggle_item.clone();
                    body["enabled"] = Value::Bool(on);
                    let id = i(&body, "id");
                    rt::spawn(async move {
                        match api::put(&format!("/raw-blocks/{id}"), body).await {
                            Ok(_) => app::refresh_status(),
                            Err(e) => toast_err(e),
                        }
                    });
                };
                let name = if s(&bl, "name").is_empty() { "Untitled block".to_string() } else { s(&bl, "name") };
                view! {
                    <div class="card stack">
                        <div class="row wrap">
                            <strong class="grow">{name}</strong>
                            {(s(&bl, "source") == "import").then(|| view! { <span class="badge">"imported"</span> })}
                            {app::is_admin().then(|| view! {
                                <label class="switch small"><input type="checkbox" prop:checked=b(&bl, "enabled") on:change=toggle/><i></i>"Enabled"</label>
                                <button class="btn sm" on:click=move |_| edit("block", item.clone(), refresh) inner_html=ib("pencil", "Edit")></button>
                            })}
                        </div>
                        <pre class="code-block peek">{s(&bl, "body")}</pre>
                    </div>
                }
            })
            .collect_view()
            .into_any()
    });
    view! {
        <PageHead title="Snippets & raw blocks" text="For anything the forms don't cover. Snippets are reusable pieces you attach to sites; raw blocks go into the Caddyfile as they are."/>
        <div class="tabs" style="margin-bottom:16px">
            <button class:active=move || tab.get() == "snippets" on:click=move |_| tab.set("snippets")>{move || format!("Snippets ({})", sn_count())}</button>
            <button class:active=move || tab.get() == "blocks" on:click=move |_| tab.set("blocks")>{move || format!("Raw blocks ({})", bl_count())}</button>
        </div>
        <div class="stack" class:hidden=move || tab.get() != "snippets">
            {app::can_edit().then(|| view! { <div><button class="btn primary" on:click=move |_| edit("snippet", Value::Null, refresh) inner_html=ib("plus", "New snippet")></button></div> })}
            {snippet_list}
        </div>
        <div class="stack" class:hidden=move || tab.get() != "blocks">
            {app::is_admin().then(|| view! { <div><button class="btn primary" on:click=move |_| edit("block", Value::Null, refresh) inner_html=ib("plus", "New raw block")></button></div> })}
            {block_list}
        </div>
    }
}
