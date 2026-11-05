//! `#/import`: bring in an existing Caddy config. Also embedded in the first-run setup (`#/welcome`).

use crate::app::{self, a, b, i, s};
use crate::icons::icon;
use crate::pages::deploy::ib;
use crate::ui::{sure, toast_err, Alert, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};
use wasm_bindgen::JsCast;

/// Badge class and label for one report row's result.
pub fn status_view(status: &str) -> (&'static str, String) {
    match status {
        "structured" => ("ok", "editable".into()),
        "partial" => ("warn", "partly editable".into()),
        "raw" => ("accent", "kept for review".into()),
        "skipped" => ("err", "skipped".into()),
        other => ("", other.to_string()),
    }
}

/// The request body: a detected source, or the pasted text.
pub fn body(picked: &Option<String>, text: &str) -> Value {
    match picked {
        Some(id) => json!({ "source": id }),
        None => json!({ "caddyfile": text }),
    }
}

/// The one-line result after an import commit.
pub fn done_text(d: &Value) -> String {
    let mut t = format!("Imported {} site(s)", i(d, "sites"));
    if i(d, "credentials") > 0 {
        t += &format!(", {} DNS account(s)", i(d, "credentials"));
    }
    if i(d, "raw_blocks") > 0 {
        t += &format!(", {} block(s) kept for review", i(d, "raw_blocks"));
    }
    t.push('.');
    let skipped: Vec<String> = a(d, "skipped").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
    if !skipped.is_empty() {
        t += &format!(" Skipped (already exist): {}.", skipped.join(", "));
    }
    t
}

const PLACEHOLDER: &str = "example.com {\n\treverse_proxy localhost:8080\n}\n\n(or paste Caddy JSON config)";

#[component]
pub fn Import(#[prop(optional)] embedded: bool, #[prop(optional, into)] on_done: Option<Callback<Value>>) -> impl IntoView {
    let sources = RwSignal::new(None::<Vec<Value>>);
    let picked = RwSignal::new(None::<String>);
    let text = RwSignal::new(String::new());
    let preview = RwSignal::new(None::<Value>);
    let replace = RwSignal::new(false);
    let settings = RwSignal::new(true);
    let busy = RwSignal::new(false);
    let done = RwSignal::new(None::<Value>);
    let show_paste = RwSignal::new(false);

    rt::spawn(async move {
        let l = api::get("/import/sources").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        if l.is_empty() {
            show_paste.set(true);
        }
        sources.set(Some(l));
    });

    let analyze = move |src: Option<String>| {
        picked.set(src.clone());
        busy.set(true);
        preview.set(None);
        done.set(None);
        let req = body(&src, &text.get_untracked());
        rt::spawn(async move {
            match api::post("/import/preview", req).await {
                Ok(v) => preview.set(Some(v)),
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };
    let commit = move |_| {
        rt::spawn(async move {
            if replace.get_untracked()
                && !sure(
                    "Replace everything?",
                    "Replace ALL existing sites, snippets and raw blocks with the imported ones?",
                    "Replace",
                    true,
                )
                .await
            {
                return;
            }
            busy.set(true);
            let mut req = body(&picked.get_untracked(), &text.get_untracked());
            req["replace"] = json!(replace.get_untracked());
            req["settings"] = json!(settings.get_untracked());
            match api::post("/import/commit", req).await {
                Ok(d) => {
                    toast(&format!("Imported {} site(s). Nothing is live until you apply.", i(&d, "sites")), "ok");
                    app::refresh_status();
                    if let Some(cb) = on_done {
                        cb.run(d.clone());
                    }
                    done.set(Some(d));
                }
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };
    let on_file = move |e: leptos::ev::Event| {
        let Some(input) = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) else { return };
        let Some(file) = input.files().and_then(|f| f.get(0)) else { return };
        input.set_value("");
        rt::spawn(async move {
            let t = wasm_bindgen_futures::JsFuture::from(file.text()).await.ok().and_then(|t| t.as_string()).unwrap_or_default();
            text.set(t);
            picked.set(None);
            preview.set(None);
        });
    };

    let found = move || {
        let l = sources.get()?;
        let cards: Vec<_> = l
            .into_iter()
            .map(|src| {
                let id = s(&src, "id");
                let id2 = id.clone();
                let hosts: Vec<String> = a(&src, "hosts").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
                let more = hosts.len().saturating_sub(8);
                let chips: Vec<_> = hosts.into_iter().take(8).map(|h| view! { <span class="badge">{h}</span> }).collect();
                view! {
                    <div class="card found" class:active=move || picked.get().as_deref() == Some(id.as_str()) && preview.get().is_some()>
                        <span class="f-ic" inner_html=icon("radar")></span>
                        <div class="grow">
                            <div class="f-title">"Found an existing config in "{s(&src, "label")}</div>
                            <div class="muted small">{s(&src, "detail")}</div>
                            {(!chips.is_empty()).then(|| view! {
                                <div class="hosts">{chips}{(more > 0).then(|| view! { <span class="faint small">{format!("+{more} more")}</span> })}</div>
                            })}
                        </div>
                        <button class="btn primary" disabled=move || busy.get() on:click=move |_| analyze(Some(id2.clone()))
                            inner_html=ib("search", "Preview import")></button>
                    </div>
                }
            })
            .collect();
        Some(cards)
    };

    let paste = move || {
        let has = sources.get().is_some_and(|l| !l.is_empty());
        view! {
            <div class="card stack">
                <div class="row wrap between">
                    <div>
                        <h2>{if has { "Or paste a config" } else { "Paste your Caddyfile" }}</h2>
                        <div class="muted small">{if has { "" } else { "No running config was found automatically. " }}"A Caddyfile or Caddy's JSON config both work."</div>
                    </div>
                    <label class="btn sm" style="cursor:pointer"><span inner_html=icon("upload")></span>"Choose file…"
                        <input type="file" style="display:none" on:change=on_file/></label>
                </div>
                <textarea class="mono" rows="12" placeholder=PLACEHOLDER prop:value=move || text.get()
                    on:input=move |e| text.set(event_target_value(&e))></textarea>
                <div><button class="btn primary" disabled=move || busy.get() || text.get().trim().is_empty()
                    on:click=move |_| analyze(None) inner_html=ib("search", "Analyze")></button></div>
            </div>
        }
    };

    let report = move || {
        let p = preview.get()?;
        let v = &p["verify"];
        let diffs: Vec<String> = a(v, "differences").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        let verify = if b(v, "checked") && diffs.is_empty() {
            let n = i(v, "sites");
            view! { <Alert kind="ok"><strong>{format!("{n} site(s) imported, 0 differences.")}</strong>
            " Caddymium regenerated the config, Caddy checked it, and every host behaves the same as before."</Alert> }
            .into_any()
        } else if b(v, "checked") {
            let n = diffs.len();
            let items: Vec<_> = diffs.into_iter().map(|d| view! { <li class="mono small">{d}</li> }).collect();
            view! { <Alert kind="warn"><strong>{format!("{n} difference(s) after regeneration:")}</strong><ul class="tight-list">{items}</ul></Alert> }
                .into_any()
        } else if !s(v, "error").is_empty() {
            let err = format!("Round-trip check skipped: {}", s(v, "error"));
            view! { <Alert kind="info">{err}</Alert> }.into_any()
        } else {
            ().into_any()
        };
        let warnings: Vec<_> = a(&p, "warnings")
            .iter()
            .map(|w| w.as_str().unwrap_or_default().to_string())
            .map(|w| view! { <Alert kind="warn">{w}</Alert> })
            .collect();
        let count = |k: &str| a(&p, k).len();
        let sites = count("sites");
        let rows: Vec<_> = a(&p, "report")
            .iter()
            .map(|r| {
                let (cls, label) = status_view(&s(r, "status"));
                view! { <tr><td class="mono small">{s(r, "block")}</td><td><span class=format!("badge {cls}")>{label}</span></td>
                <td class="hide-sm small muted">{s(r, "detail")}</td></tr> }
            })
            .collect();
        let from = match picked.get() {
            Some(id) => {
                let label = sources.get().unwrap_or_default().iter().find(|x| s(x, "id") == id).map(|x| s(x, "label")).unwrap_or(id);
                format!("From {label}")
            }
            None => "From pasted config".to_string(),
        };
        let chip =
            |n: usize, what: &str, cls: &str| (n > 0).then(|| view! { <span class=format!("badge {cls}")>{format!("{n} {what}")}</span> });
        let raw = count("raw_blocks");
        Some(view! {
            <div class="card stack">
                <div><h2>"Import preview"</h2><div class="muted small">{from}</div></div>
                {verify}
                {warnings}
                <div class="row wrap small">
                    <span class="badge ok">{format!("{sites} sites")}</span>
                    {chip(count("snippets"), "snippets", "")}
                    {chip(count("credentials"), "DNS accounts", "")}
                    {chip(count("domains"), "domains", "")}
                    {chip(raw, "kept for review", "accent")}
                </div>
                <div class="table-wrap"><table class="table">
                    <thead><tr><th>"Block"</th><th>"Result"</th><th class="hide-sm">"Details"</th></tr></thead>
                    <tbody>{rows}</tbody>
                </table></div>
                {(raw > 0).then(|| view! { <p class="muted small">"Blocks “kept for review” land under "<strong>"Snippets & raw"</strong>
                    ", disabled, with the original config so you can convert them by hand."</p> })}
                <label class="switch"><input type="checkbox" prop:checked=move || settings.get() on:change=move |e| settings.set(event_target_checked(&e))/><i></i>
                    <span>"Also import global options (email, certificate authority)"</span></label>
                <label class="switch"><input type="checkbox" prop:checked=move || replace.get() on:change=move |e| replace.set(event_target_checked(&e))/><i></i>
                    <span>"Replace my existing sites (otherwise hostnames that already exist are skipped)"</span></label>
                <div><button class="btn primary" disabled=move || busy.get() on:click=commit inner_html=ib("file-input", &format!("Import {sites} site(s)"))></button></div>
            </div>
        })
    };

    view! {
        {(!embedded).then(|| view! {
            <PageHead title="Import" text="Bring in your existing Caddy config. What Caddymium understands becomes editable sites; everything else is kept, nothing is dropped silently. Nothing goes live until you apply."/>
        })}
        <div class="stack">
            {move || sources.get().is_none().then(|| view! { <div class="card row muted"><span inner_html=icon("radar")></span>"Looking for an existing Caddy config…"</div> })}
            {found}
            {move || (sources.get().is_some_and(|l| !l.is_empty()) && !show_paste.get()).then(|| view! {
                <div><button class="btn ghost sm" on:click=move |_| show_paste.set(true) inner_html=ib("file-input", "Paste a Caddyfile instead")></button></div>
            })}
            {move || show_paste.get().then(paste)}
            {move || (busy.get() && preview.get().is_none()).then(|| view! {
                <div class="card row muted"><span class="spinner"></span>"Analyzing and test-regenerating with Caddy…"</div>
            })}
            {report}
            {move || done.get().filter(|_| !embedded).map(|d| view! {
                <Alert kind="ok">{done_text(&d)}
                    <div class="row" style="margin-top:8px">
                        <button class="btn primary sm" on:click=|_| app::navigate("/changes")>"Review & apply"</button>
                        <button class="btn sm" on:click=|_| app::navigate("/sites")>"See sites"</button>
                    </div>
                </Alert>
            })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_helpers() {
        assert_eq!(body(&Some("docker".into()), "x"), json!({"source": "docker"}));
        assert_eq!(body(&None, "a {}"), json!({"caddyfile": "a {}"}));
        assert_eq!(status_view("raw").0, "accent");
        assert_eq!(status_view("odd").1, "odd");
        let t = done_text(&json!({"sites": 2, "credentials": 1, "raw_blocks": 0, "skipped": ["a.com"]}));
        assert_eq!(t, "Imported 2 site(s), 1 DNS account(s). Skipped (already exist): a.com.");
    }
}
