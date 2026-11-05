//! `#/settings?s=<page>`: Caddy's global options, drawn by the kit from the server's settings schema
//! (backend/settings.toml), and Backup & restore.

use crate::app::{self, s};
use crate::pages::deploy::ib;
use crate::ui::{download, sure, toast_err, Alert, ErrorBox, Loading, PageHead};
use kit_leptos::schema_form::schema::{visible_cards, Schema};
use kit_leptos::schema_form::SettingsCards;
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::Value;
use wasm_bindgen::JsCast;

/// "3 sites, 1 snippets" from a backup's `{table: count}`.
pub fn counts(v: &Value) -> String {
    v.as_object().map(|o| o.iter().map(|(k, n)| format!("{n} {}", k.replace('_', " "))).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

fn backup() {
    rt::spawn(async move {
        match api::get("/backup").await {
            Ok(env) => {
                let stamp: String = js_sys::Date::new_0()
                    .to_iso_string()
                    .as_string()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .take(12)
                    .collect();
                download(&format!("caddymium-backup-{stamp}.json"), &serde_json::to_string_pretty(&env).unwrap_or_default());
                toast("Backup downloaded. It is encrypted with your CADDYMIUM_SECRET_KEY; keep the key with it.", "ok");
            }
            Err(e) => toast_err(e),
        }
    });
}

#[component]
fn Backup() -> impl IntoView {
    let restoring = RwSignal::new(false);
    let restored = RwSignal::new(None::<Value>);
    let on_file = move |e: leptos::ev::Event| {
        let Some(input) = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) else { return };
        let Some(file) = input.files().and_then(|f| f.get(0)) else { return };
        input.set_value("");
        rt::spawn(async move {
            let text = wasm_bindgen_futures::JsFuture::from(file.text()).await.ok().and_then(|t| t.as_string()).unwrap_or_default();
            let Ok(env) = serde_json::from_str::<Value>(&text) else {
                toast("That file is not a Caddymium backup.", "err");
                return;
            };
            let made = s(&env, "created_at");
            let holds = counts(&env["contents"]);
            let text = format!(
                "It was made {} and holds {}. This replaces all sites, domains, DNS accounts, snippets, raw blocks, templates, notification channels and settings. Users and API keys are kept. Nothing reaches Caddy until you review and apply.",
                if made.is_empty() { "on an unknown date".to_string() } else { crate::ui::when(&made) },
                if holds.is_empty() { "unknown contents".to_string() } else { holds }
            );
            if !sure("Restore this backup?", &text, "Restore", true).await {
                return;
            }
            restoring.set(true);
            restored.set(None);
            match api::post("/backup/restore", env).await {
                Ok(r) => {
                    restored.set(Some(r));
                    toast("Backup restored. Review & apply to put it into Caddy.", "ok");
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
            restoring.set(false);
        });
    };
    view! {
        <div class="card stack">
            <h2 style="margin:0">"Backup & restore"</h2>
            <p class="muted small" style="margin:0">"A backup holds every site, domain, DNS account, snippet, raw block, template, notification channel, named secret and the settings, encrypted with the panel's "
                <code>"CADDYMIUM_SECRET_KEY"</code>". Keep the key with the file: a restore needs the same key. Users, API keys and the history are not included; certificates live in Caddy's data volume."</p>
            <div class="row wrap">
                <button class="btn primary" on:click=move |_| backup() inner_html=ib("download", "Download backup")></button>
                <label class="btn" style="cursor:pointer">"Restore from file…"
                    <input type="file" accept=".json,application/json" style="display:none" on:change=on_file disabled=move || restoring.get()/></label>
            </div>
            {move || restored.get().map(|r| view! {
                <Alert kind="ok">{format!("Restored: {}. ", counts(&r["restored"]["tables"]))}<a href="#/changes">"Review & apply"</a>" to put it into Caddy."</Alert>
            })}
        </div>
    }
}

/// `#/settings`.
#[component]
pub fn Settings() -> impl IntoView {
    let data = LocalResource::new(|| async {
        let schema = api::get("/settings/form/schema").await.map_err(|e| e.message)?;
        let values = api::get("/settings/form").await.map_err(|e| e.message)?;
        Ok::<_, String>((Schema::from_json(&schema), values))
    });
    let route = app::Route::current();
    let page = route.param("s");
    let admin = app::is_admin();
    let current = {
        let page = page.clone();
        move || match data.get() {
            Some(Ok((schema, _))) if page.is_empty() => schema.sections.first().map(|s| s.id.clone()).unwrap_or_default(),
            _ => page.clone(),
        }
    };
    let tabs = {
        let current = current.clone();
        move || {
            let on = current();
            let mut list: Vec<(String, String)> = match data.get() {
                Some(Ok((schema, _))) => schema.sections.iter().map(|s| (s.id.clone(), s.title.clone())).collect(),
                _ => vec![],
            };
            if admin {
                list.push(("backup".into(), "Backup & restore".into()));
            }
            list.into_iter()
                .map(|(id, title)| {
                    let active = id == on;
                    view! { <a href=format!("#/settings?s={id}") class:active=active>{title}</a> }
                })
                .collect_view()
        }
    };
    let body = move || match current().as_str() {
        "backup" if admin => view! { <Backup/> }.into_any(),
        id => match data.get() {
            None => view! { <Loading/> }.into_any(),
            Some(Err(e)) => view! { <ErrorBox title="The settings can't be loaded" message=e/> }.into_any(),
            Some(Ok((schema, values))) => {
                let cards = schema.section(id).map(|s| visible_cards(&s.groups, &values)).unwrap_or_default();
                if cards.is_empty() {
                    view! { <div class="empty">"Nothing here."</div> }.into_any()
                } else {
                    view! {
                        {(!admin).then(|| view! { <Alert kind="info">"Only admins can change these settings."</Alert> })}
                        <SettingsCards cards=cards values=values/>
                        <p class="muted small">"Saved settings reach Caddy with the next apply."</p>
                    }
                    .into_any()
                }
            }
        },
    };
    view! {
        <PageHead title="Settings" text="Global options for the whole Caddy server."/>
        <div class="tabs" style="margin-bottom:16px">{tabs}</div>
        <div class="stack">{body}</div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn backup_counts() {
        assert_eq!(counts(&json!({"raw_blocks": 2})), "2 raw blocks");
        assert_eq!(counts(&Value::Null), "");
    }
}
