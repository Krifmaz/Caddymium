//! Deploying: Review & apply (`#/changes`), History (`#/history`) and the generated Caddyfile (`#/caddyfile`).

use crate::app::{self, a, b, i, s};
use crate::icons::icon;
use crate::ui::{ago, download, sure, toast_err, when, Alert, Diff, ErrorBox, Loading, PageHead, PlanErrors};
use kit_leptos::modal::{self, ModalOpts};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};

/// A button's inner HTML: an icon and its label.
pub fn ib(name: &str, label: &str) -> String {
    format!("{}{}", icon(name), label)
}

/// What an apply ended as: (alert kind, title).
pub fn result_view(status: &str) -> (&'static str, String) {
    match status {
        "applied" => ("ok", "Applied".into()),
        "dry_run" => ("info", "Dry run passed. The config is valid; nothing was applied".into()),
        "rolled_back" => ("err", "Rolled back".into()),
        other => ("err", format!("Not applied ({other})")),
    }
}

/// `#/changes`: the plan (validation, diff), Dry run, Apply, Discard, Revert and Backup.
#[component]
pub fn Changes() -> impl IntoView {
    let plan = RwSignal::new(Value::Null);
    let goods = RwSignal::new(Vec::<Value>::new());
    let loading = RwSignal::new(true);
    let busy = RwSignal::new(false);
    let result = RwSignal::new(Value::Null);
    let error = RwSignal::new(None::<String>);
    let show_full = RwSignal::new(false);
    let note = NodeRef::<leptos::html::Input>::new();
    let load = move || {
        loading.set(true);
        result.set(Value::Null);
        rt::spawn(async move {
            match api::post_empty("/config/plan").await {
                Ok(p) => {
                    plan.set(p);
                    error.set(None);
                }
                Err(e) => error.set(Some(e.message)),
            }
            let vs = api::get("/versions?limit=50").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            goods.set(vs.into_iter().filter(|v| b(v, "good")).collect());
            loading.set(false);
        });
    };
    load();
    let current = move || goods.get().first().cloned();
    let previous = move || goods.get().get(1).filter(|v| b(v, "snapshot")).cloned();
    let run = move |dry: bool| {
        busy.set(true);
        result.set(Value::Null);
        let message = note.get().map(|n| n.value()).unwrap_or_default();
        rt::spawn(async move {
            match api::post("/config/apply", json!({ "dry_run": dry, "message": message })).await {
                Ok(r) => {
                    if s(&r, "status") == "applied" {
                        toast("Applied. Caddy is running the new config.", "ok");
                        if let Some(n) = note.get() {
                            n.set_value("");
                        }
                    }
                    if let Some(p) = r.get("plan").filter(|p| !p.is_null()) {
                        plan.set(p.clone());
                    }
                    result.set(r);
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };
    let discard = move |_| {
        rt::spawn(async move {
            let ok = sure(
                "Discard changes?",
                "Sites, domains, DNS accounts, snippets, raw blocks and settings go back to what Caddy is running now. This can't be undone.",
                "Discard",
                true,
            )
            .await;
            if !ok {
                return;
            }
            busy.set(true);
            match api::post_empty("/config/discard").await {
                Ok(r) => {
                    toast(&format!("Changes discarded; back to version #{}.", s(&r, "version_id")), "ok");
                    load();
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };
    let revert = move |_| {
        let Some(prev) = previous() else { return };
        rt::spawn(async move {
            let id = i(&prev, "id");
            let text = format!(
                "Caddy goes back to version #{id} ({}), and the sites and settings in Caddymium with it. Unapplied edits are lost.",
                when(&s(&prev, "created_at"))
            );
            if !sure("Revert the last apply?", &text, "Revert", true).await {
                return;
            }
            busy.set(true);
            match api::post_empty(&format!("/versions/{id}/rollback")).await {
                Ok(r) => {
                    if s(&r, "status") == "applied" {
                        toast(&format!("Reverted to version #{id}."), "ok");
                    }
                    result.set(r);
                    load();
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };
    let backup = move |_| {
        rt::spawn(async move {
            match api::get("/backup").await {
                Ok(env) => {
                    let stamp: String = String::from(js_sys::Date::new_0().to_iso_string())
                        .chars()
                        .take(16)
                        .filter(|c| !matches!(c, '-' | ':' | 'T'))
                        .collect();
                    download(&format!("caddymium-backup-{stamp}.json"), &serde_json::to_string_pretty(&env).unwrap_or_default());
                    toast("Backup downloaded. It is encrypted with your CADDYMIUM_SECRET_KEY; keep the key with it.", "ok");
                }
                Err(e) => toast_err(e),
            }
        });
    };
    let body = move || {
        let p = plan.get();
        if let Some(e) = error.get() {
            return view! { <ErrorBox message=e/> }.into_any();
        }
        if p.is_null() {
            return view! { <div class="card row muted"><span class="spinner"></span>"Generating and validating with Caddy…"</div> }
                .into_any();
        }
        let edit = app::can_edit();
        let valid = b(&p, "valid");
        let changed = b(&p, "changed");
        let res = result.get();
        let res_view = (!res.is_null()).then(|| {
            let st = s(&res, "status");
            let (kind, title) = result_view(&st);
            let msg = if st == "dry_run" { String::new() } else { s(&res, "message") };
            view! { <Alert kind=kind><strong>{format!("{title}.")}</strong>" "{msg}</Alert> }
        });
        let cur = current();
        let prev = previous();
        let can_discard = cur.as_ref().is_some_and(|c| b(c, "snapshot"));
        let no_snapshot = cur.as_ref().is_some_and(|c| !b(c, "snapshot"));
        let hint = if no_snapshot {
            format!(
                "Version #{} was applied before Caddymium kept a copy of the sites and settings with each version, so Discard and Revert are not available yet. They work from the next apply on. ",
                cur.as_ref().map(|c| i(c, "id")).unwrap_or(0)
            )
        } else {
            "Discard throws away unapplied edits. Revert re-applies the previous version and restores its sites and settings. ".into()
        };
        let compared = if i(&p, "previous_id") > 0 {
            format!("Compared with the last applied version (#{})", i(&p, "previous_id"))
        } else {
            "First apply: everything is new".into()
        };
        let warnings = a(&p, "warnings")
            .into_iter()
            .map(|w| view! { <Alert kind="warn">{w.as_str().unwrap_or("").to_string()}</Alert> })
            .collect_view();
        let caddyfile = s(&p, "caddyfile");
        view! {
            <div class="stack">
                {res_view}
                <div class="card summary">
                    <div class="sum-item">
                        <span class=if valid { "sum-ic ok" } else { "sum-ic err" } inner_html=icon(if valid { "shield-check" } else { "x" })></span>
                        <div><div class="sum-label">"Validation"</div><div class="sum-val">{if valid { "Passed" } else { "Failed" }}</div></div>
                    </div>
                    <div class="sum-item">
                        <span class="sum-ic" inner_html=icon("file-code")></span>
                        <div><div class="sum-label">"Changes"</div><div class="sum-val">
                            {if changed {
                                view! { <span class="plus">{format!("+{}", i(&p, "added"))}</span>" "<span class="minus">{format!("−{}", i(&p, "removed"))}</span>" lines" }.into_any()
                            } else { "None".into_any() }}
                        </div></div>
                    </div>
                    {edit.then(|| view! {
                        <div class="sum-actions">
                            <button class="btn" disabled=move || busy.get() on:click=move |_| run(true) inner_html=ib("flask", "Dry run")></button>
                            <button class="btn primary" disabled=move || busy.get() || !valid on:click=move |_| run(false)
                                inner_html=move || ib("rocket", if busy.get() { "Applying…" } else { "Apply to Caddy" })></button>
                        </div>
                    })}
                </div>
                {edit.then(|| view! {
                    <div class="row wrap">
                        {(changed && can_discard).then(|| view! { <button class="btn danger" disabled=move || busy.get() on:click=discard inner_html=ib("trash", "Discard changes")></button> })}
                        {prev.map(|pv| view! { <button class="btn" disabled=move || busy.get() on:click=revert inner_html=ib("history", &format!("Revert last apply (to #{})", i(&pv, "id")))></button> })}
                        {app::is_admin().then(|| view! { <button class="btn" on:click=backup inner_html=ib("download", "Backup")></button> })}
                        <span class="small muted grow" style="min-width:220px">{hint}"Backup downloads everything except users and API keys, encrypted with the panel's secret key; restore it from Settings."</span>
                    </div>
                })}
                <PlanErrors plan=p.clone()/>
                {warnings}
                {(edit && valid && changed).then(|| view! {
                    <input class="input" node_ref=note placeholder="Optional note for the history, e.g. “added jellyfin”"/>
                })}
                <div class="card stack">
                    <div><h2>"Diff"</h2><div class="small muted">{compared}</div></div>
                    {if changed {
                        view! { <Diff diff=s(&p, "diff")/> }.into_any()
                    } else {
                        view! { <div class="empty" inner_html=format!("{}<p>Caddy is already running this exact config.</p>", icon("circle-check"))></div> }.into_any()
                    }}
                </div>
                <div>
                    <button class="btn ghost sm" on:click=move |_| show_full.update(|v| *v = !*v)
                        inner_html=move || ib("file-code", if show_full.get() { "Hide the full generated Caddyfile" } else { "Show the full generated Caddyfile" })></button>
                    {move || show_full.get().then(|| view! { <pre class="code-block mt">{caddyfile.clone()}</pre> })}
                </div>
            </div>
        }
        .into_any()
    };
    view! {
        <PageHead title="Review & apply" text="Caddymium writes the Caddyfile, Caddy checks it, you see exactly what changes. If Caddy isn't healthy after applying, the previous config comes back by itself.">
            <button class="btn" disabled=move || loading.get() on:click=move |_| load() inner_html=ib("refresh", "Refresh")></button>
        </PageHead>
        {body}
    }
}

/// The badge class of a version's status.
fn status_badge(st: &str) -> &'static str {
    match st {
        "applied" => "badge ok",
        "dry_run" => "badge accent",
        "invalid" | "failed" => "badge err",
        "rolled_back" => "badge warn",
        _ => "badge",
    }
}

/// The modal of one version: its changes, a diff with now, its Caddyfile, Download and Restore.
fn open_version(v: Value, versions: Vec<Value>, reload: impl Fn() + Clone + 'static) {
    let id = i(&v, "id");
    rt::spawn(async move {
        let full = match api::get(&format!("/versions/{id}")).await {
            Ok(f) => f,
            Err(e) => return toast_err(e),
        };
        let tab = RwSignal::new("diff");
        let diff = RwSignal::new(None::<Result<Value, String>>);
        let prev = versions.iter().find(|x| i(x, "id") < id && b(x, "good")).map(|x| i(x, "id"));
        let load_diff = move |against_now: bool| {
            diff.set(None);
            rt::spawn(async move {
                let r = if against_now {
                    api::get(&format!("/versions/diff?a={id}&b=0")).await.map_err(|e| e.message)
                } else {
                    match prev {
                        Some(p) => api::get(&format!("/versions/diff?a={p}&b={id}")).await.map_err(|e| e.message),
                        None => Ok(json!({ "first": true })),
                    }
                };
                diff.set(Some(r));
            });
        };
        load_diff(false);
        let f = full.clone();
        let st = s(&f, "status");
        let meta = format!("{} · {} · Caddy {}", when(&s(&f, "created_at")), s(&f, "user"), {
            let v = s(&f, "caddy_version");
            if v.is_empty() {
                "?".to_string()
            } else {
                v
            }
        });
        let good = b(&f, "good");
        let snapshot = b(&f, "snapshot");
        let message = s(&f, "message");
        let caddyfile = s(&f, "caddyfile");
        let cf = caddyfile.clone();
        let body = move || {
            let tab_btn = move |key: &'static str, label: &'static str| {
                view! {
                    <button class:active=move || tab.get() == key on:click=move |_| {
                        tab.set(key);
                        match key { "diff" => load_diff(false), "pending" => load_diff(true), _ => {} }
                    }>{label}</button>
                }
            };
            view! {
                <div class="stack">
                    <div class="row wrap small">
                        <span class=status_badge(&st)>{st.replace('_', " ")}</span>
                        <span class="muted">{meta}</span>
                        {good.then(|| if snapshot {
                            view! { <span class="badge accent" title="Restoring this version also brings back its sites and settings">"sites & settings saved"</span> }.into_any()
                        } else {
                            view! { <span class="badge" title="Applied before Caddymium saved sites with versions; only the Caddyfile can be re-applied">"Caddyfile only"</span> }.into_any()
                        })}
                    </div>
                    {(!message.is_empty()).then(|| view! { <p class="small" style="margin:0">{message.clone()}</p> })}
                    <div class="tabs">
                        {tab_btn("diff", "Changes in this version")}
                        {tab_btn("pending", "Compare with now")}
                        {tab_btn("file", "Caddyfile")}
                    </div>
                    {move || {
                        if tab.get() == "file" {
                            return view! { <pre class="code-block scroll-55">{cf.clone()}</pre> }.into_any();
                        }
                        match diff.get() {
                            None => view! { <Loading/> }.into_any(),
                            Some(Err(e)) => view! { <ErrorBox message=e/> }.into_any(),
                            Some(Ok(d)) if b(&d, "first") => view! { <p class="muted">"This is the first applied version."</p> }.into_any(),
                            Some(Ok(d)) => view! { <div class="scroll-55"><Diff diff=s(&d, "diff")/></div> }.into_any(),
                        }
                    }}
                </div>
            }
        };
        let handle: StoredValue<Option<modal::Modal>, LocalStorage> = StoredValue::new_local(None);
        let busy = RwSignal::new(false);
        let restore = move |_| {
            let reload = reload.clone();
            rt::spawn(async move {
                let text = if snapshot {
                    format!("Caddy runs exactly that Caddyfile again (it's checked first), and the sites and settings in Caddymium go back to version {id} too. Unapplied edits are lost.")
                } else {
                    format!("Caddy runs exactly that Caddyfile again (it's checked first). Version {id} was applied before Caddymium saved sites with versions, so your sites and settings stay as they are; the next normal apply uses them again.")
                };
                if !sure(&format!("Restore version {id}?"), &text, "Restore", true).await {
                    return;
                }
                busy.set(true);
                match api::post_empty(&format!("/versions/{id}/rollback")).await {
                    Ok(r) => {
                        if s(&r, "status") == "applied" {
                            toast(&format!("Rolled back to version {id}"), "ok");
                        } else {
                            toast(&s(&r, "message"), "err");
                        }
                        if let Some(m) = handle.get_value() {
                            m.close();
                        }
                        reload();
                        app::refresh_status();
                    }
                    Err(e) => toast_err(e),
                }
                busy.set(false);
            });
        };
        let footer = move || {
            let cf = caddyfile.clone();
            view! {
                <button class="btn" on:click=move |_| download(&format!("Caddyfile.v{id}"), &cf) inner_html=ib("download", "Download")></button>
                {(app::can_edit() && good).then(|| view! {
                    <button class="btn primary" disabled=move || busy.get() on:click=restore inner_html=ib("undo", "Restore this version")></button>
                })}
            }
        };
        let m = modal::open(ModalOpts::with_view(&format!("Version {id}"), body).footer_view(footer).wide(true));
        handle.set_value(Some(m));
    });
}

/// `#/history`: every apply attempt with its Caddyfile; good ones can be restored.
#[component]
pub fn History() -> impl IntoView {
    let versions = RwSignal::new(None::<Result<Vec<Value>, String>>);
    let load = move || {
        rt::spawn(async move {
            let r = api::get("/versions?limit=100").await.map(|v| v.as_array().cloned().unwrap_or_default()).map_err(|e| e.message);
            versions.set(Some(r));
        });
    };
    load();
    let body = move || {
        match versions.get() {
        None => view! { <Loading/> }.into_any(),
        Some(Err(e)) => view! { <ErrorBox message=e/> }.into_any(),
        Some(Ok(vs)) if vs.is_empty() => view! {
            <div class="card empty" inner_html=format!("{}<h2>Nothing applied yet</h2><p>Versions show up here once you apply a config.</p>", icon("history"))></div>
        }
        .into_any(),
        Some(Ok(vs)) => {
            let all = vs.clone();
            let rows = vs
                .into_iter()
                .map(|v| {
                    let st = s(&v, "status");
                    let (vv, all) = (v.clone(), all.clone());
                    view! {
                        <tr class="clickable" on:click=move |_| open_version(vv.clone(), all.clone(), load)>
                            <td class="mono">{i(&v, "id")}</td>
                            <td class="nowrap" title=when(&s(&v, "created_at"))>{ago(&s(&v, "created_at"))}</td>
                            <td><span class=status_badge(&st)>{st.replace('_', " ")}</span></td>
                            <td class="hide-sm">{s(&v, "user")}</td>
                            <td class="hide-sm small ellipsis" style="max-width:420px">{s(&v, "message")}</td>
                        </tr>
                    }
                })
                .collect_view();
            view! {
                <div class="card flat table-wrap"><table class="table">
                    <thead><tr><th>"#"</th><th>"When"</th><th>"Result"</th><th class="hide-sm">"By"</th><th class="hide-sm">"Message"</th></tr></thead>
                    <tbody>{rows}</tbody>
                </table></div>
            }
            .into_any()
        }
    }
    };
    view! {
        <PageHead title="History" text="Every apply attempt is kept with its exact Caddyfile. Good ones can be restored with one click."/>
        {body}
    }
}

/// `#/caddyfile`: the whole generated config, with Copy and Download.
#[component]
pub fn Caddyfile() -> impl IntoView {
    let out = RwSignal::new(None::<Result<Value, String>>);
    rt::spawn(async move { out.set(Some(api::get("/config/preview").await.map_err(|e| e.message))) });
    let buttons = move || {
        out.get().and_then(|r| r.ok()).map(|o| {
            let (c1, c2) = (s(&o, "caddyfile"), s(&o, "caddyfile"));
            view! {
                <button class="btn" on:click=move |_| kit_leptos::clipboard::copy_then(&c1, "Copied") inner_html=ib("copy", "Copy")></button>
                <button class="btn primary" on:click=move |_| download("Caddyfile", &c2) inner_html=ib("download", "Download")></button>
            }
        })
    };
    let body = move || {
        match out.get() {
        None => view! { <Loading/> }.into_any(),
        Some(Err(e)) => view! { <ErrorBox message=e/> }.into_any(),
        Some(Ok(o)) => view! {
            <div class="stack">
                {a(&o, "warnings").into_iter().map(|w| view! { <Alert kind="warn">{w.as_str().unwrap_or("").to_string()}</Alert> }).collect_view()}
                <pre class="code-block">{s(&o, "caddyfile")}</pre>
                <p class="muted small" style="margin:0">"Want to change something Caddymium has no field for? Use a site's Custom Caddyfile box, a snippet or a raw block. Editing this file directly would be overwritten by the next apply."</p>
            </div>
        }
        .into_any(),
    }
    };
    view! {
        <PageHead title="Caddyfile" text="The whole config made from your sites and settings. You can always export it and run Caddy without Caddymium.">
            {buttons}
        </PageHead>
        {body}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn results_read_plainly() {
        assert_eq!(super::result_view("applied").0, "ok");
        assert_eq!(super::result_view("weird").1, "Not applied (weird)");
    }
}
