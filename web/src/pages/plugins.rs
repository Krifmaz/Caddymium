//! `#/plugins`: the Caddy build (plugins, updates, restart, roll back) and the plugin catalog.

use crate::app::{self, a, b, i, s};
use crate::form::{Check, Select, Text};
use crate::icons::icon;
use crate::pages::deploy::ib;
use crate::ui::{sure, toast_err, when, Alert, ErrorBox, Loading, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;

const CATEGORIES: &[(&str, &str)] = &[
    ("featured", "Featured"),
    ("dns", "DNS providers"),
    ("security", "Security"),
    ("proxy", "Proxying"),
    ("network", "Network"),
    ("other", "Other"),
    ("all", "All"),
];

const INTERVALS: &[(i64, &str)] = &[(1, "hour"), (6, "6 hours"), (12, "12 hours"), (24, "day"), (168, "week")];

/// "1.2M", "34k", "12".
pub fn downloads(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

/// Whether catalog package `p` shows under category `cat` and search `q`.
pub fn matches(p: &Value, cat: &str, q: &str) -> bool {
    if cat == "featured" && !b(p, "featured") {
        return false;
    }
    if cat != "featured" && cat != "all" && s(p, "category") != cat {
        return false;
    }
    if q.is_empty() {
        return true;
    }
    let mods: Vec<String> = a(p, "modules").iter().filter_map(|m| m.as_str().map(str::to_string)).collect();
    format!("{} {} {} {}", s(p, "path"), s(p, "name"), s(p, "description"), mods.join(" ")).to_lowercase().contains(&q.to_lowercase())
}

fn job_title(j: &Value) -> String {
    let what = match s(j, "kind").as_str() {
        "install" => "Installing plugins",
        "upgrade" => "Updating Caddy",
        "reset" => "Resetting to the image build",
        "restart" => "Restarting Caddy",
        _ => "Rolling back",
    };
    match s(j, "status").as_str() {
        "done" => format!("{what}: done"),
        "failed" => format!("{what}: failed"),
        _ => what.to_string(),
    }
}

fn short(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// `#/plugins`.
#[component]
pub fn Plugins() -> impl IntoView {
    let st = RwSignal::new(Value::Null);
    let upd = RwSignal::new(Value::Null);
    let us = RwSignal::new(Value::Null);
    let q = RwSignal::new(String::new());
    let cat = RwSignal::new("featured".to_string());
    let add = RwSignal::new(Vec::<String>::new());
    let del = RwSignal::new(Vec::<String>::new());
    let busy = RwSignal::new(false);
    let show_log = RwSignal::new(true);
    let admin = app::is_admin();
    let alive = Arc::new(AtomicBool::new(true));
    let polling = Arc::new(AtomicBool::new(false));
    {
        let alive = alive.clone();
        on_cleanup(move || alive.store(false, Relaxed));
    }

    let load = move || {
        rt::spawn(async move {
            match api::get("/plugins").await {
                Ok(v) => st.set(v),
                Err(e) => toast_err(e),
            }
            match api::get("/updates").await {
                Ok(v) => upd.set(v),
                Err(e) => toast_err(e),
            }
            if admin {
                if let Ok(v) = api::get("/settings").await {
                    us.set(v);
                }
            }
        });
    };

    // Polls the running job every 2s until it ends (or the page goes away).
    fn poll(
        st: RwSignal<Value>,
        add: RwSignal<Vec<String>>,
        del: RwSignal<Vec<String>>,
        alive: Arc<AtomicBool>,
        polling: Arc<AtomicBool>,
        load: impl Fn() + Copy + 'static,
    ) {
        if !alive.load(Relaxed) {
            polling.store(false, Relaxed);
            return;
        }
        rt::after(2000, move || {
            rt::spawn(async move {
                if !alive.load(Relaxed) {
                    polling.store(false, Relaxed);
                    return;
                }
                let Ok(j) = api::get("/plugins/job").await else {
                    polling.store(false, Relaxed);
                    return;
                };
                st.update(|v| v["job"] = j.clone());
                if s(&j, "status") == "running" {
                    poll(st, add, del, alive, polling, load);
                    return;
                }
                polling.store(false, Relaxed);
                if s(&j, "status") == "done" {
                    toast("Done. Caddy is running the new build.", "ok");
                    add.set(vec![]);
                    del.set(vec![]);
                } else {
                    toast("The plugin operation failed; see the log.", "err");
                }
                load();
                app::refresh_status();
            });
        });
    }
    let watch = {
        let (alive, polling) = (alive.clone(), polling.clone());
        move || {
            if !polling.swap(true, Relaxed) {
                poll(st, add, del, alive.clone(), polling.clone(), load);
            }
        }
    };
    {
        let watch = watch.clone();
        rt::spawn(async move {
            if let Ok(v) = api::get("/plugins").await {
                let running = s(&v["job"], "status") == "running";
                st.set(v);
                if running {
                    watch();
                }
            }
        });
        rt::spawn(async move {
            if let Ok(v) = api::get("/updates").await {
                upd.set(v);
            }
            if admin {
                if let Ok(v) = api::get("/settings").await {
                    us.set(v);
                }
            }
        });
    }

    let run = {
        let watch = watch.clone();
        move |title: String, text: String, ok: &'static str, danger: bool, path: &'static str, body: Value| {
            let watch = watch.clone();
            rt::spawn(async move {
                if !sure(&title, &text, ok, danger).await {
                    return;
                }
                busy.set(true);
                match api::post(path, body).await {
                    Ok(j) => {
                        st.update(|v| v["job"] = j);
                        show_log.set(true);
                        watch();
                    }
                    Err(e) => toast_err(e),
                }
                busy.set(false);
            });
        }
    };
    let running = move || s(&st.get()["job"], "status") == "running";
    let supported = move || b(&st.get(), "supported");

    let check_now = move |_| {
        busy.set(true);
        rt::spawn(async move {
            match api::post_empty("/updates/check").await {
                Ok(v) => {
                    let err = s(&v, "last_error");
                    if !err.is_empty() {
                        toast(&err, "err");
                    } else if b(&v, "update_available") {
                        toast(&format!("Caddy {} is available", s(&v, "latest")), "ok");
                    } else {
                        toast(&format!("Caddy {} is up to date", s(&v, "running")), "ok");
                    }
                    upd.set(v);
                    if let Ok(p) = api::get("/plugins").await {
                        st.set(p);
                    }
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };
    let save_updates = move |_| {
        busy.set(true);
        let mut body = us.get_untracked();
        if let Some(n) = body["updates"]["check_every_hours"].as_str().and_then(|x| x.parse::<i64>().ok()) {
            body["updates"]["check_every_hours"] = json!(n);
        }
        if let Some(o) = body.as_object_mut() {
            o.remove("acme_eab_mac_set");
            o.remove("crowdsec_api_key_set");
        }
        rt::spawn(async move {
            match api::put("/settings", body).await {
                Ok(v) => {
                    us.set(v);
                    if let Ok(u) = api::get("/updates").await {
                        upd.set(u);
                    }
                    toast("Update settings saved", "ok");
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };
    let refresh_catalog = move |_| {
        busy.set(true);
        rt::spawn(async move {
            match api::post_empty("/plugins/catalog/refresh").await {
                Ok(c) => {
                    st.update(|v| {
                        v["catalog"] = c["packages"].clone();
                        v["catalog_fetched_at"] = c["fetched_at"].clone();
                    });
                    toast("Catalog refreshed", "ok");
                }
                Err(e) => toast_err(e),
            }
            busy.set(false);
        });
    };

    let build_card = {
        let run = run.clone();
        move || {
            let v = st.get();
            let run = run.clone();
            let (r1, r2, r3, r4) = (run.clone(), run.clone(), run.clone(), run);
            let latest = s(&v, "latest_caddy");
            let reason = s(&v, "reason");
            let latest2 = latest.clone();
            let actions = admin.then(|| {
                view! {
                    <div class="row wrap" style="margin-left:auto">
                        <button class="btn sm" disabled=move || busy.get() || running() || !supported()
                            title="Rebuild with the current plugins against the latest Caddy release"
                            on:click=move |_| r1(
                                "Rebuild Caddy?".into(),
                                format!("Rebuild Caddy {} with the current plugins and restart it? Sites are unavailable for a few seconds.", if latest2.is_empty() { "(latest release)".into() } else { latest2.clone() }),
                                "Rebuild", false, "/plugins/install", json!({ "add": [], "remove": [] }))
                            inner_html=ib("download", if b(&v, "update_available") { "Update Caddy" } else { "Rebuild latest" })></button>
                        <button class="btn sm" disabled=move || busy.get() || running()
                            on:click=move |_| r2("Restart Caddy?".into(),
                                "Sites are unavailable for a few seconds. If Caddy does not answer it is stopped forcefully, and if it does not come back the image's built-in Caddy takes over.".into(),
                                "Restart", false, "/plugins/restart", json!({}))
                            inner_html=ib("refresh", "Restart Caddy")></button>
                        {b(&v, "previous_binary").then(|| view! {
                            <button class="btn sm" disabled=move || busy.get() || running()
                                on:click=move |_| r3("Roll back?".into(), "Switch back to the previous custom build and restart Caddy?".into(),
                                    "Roll back", false, "/plugins/rollback", json!({}))
                                inner_html=ib("undo", "Roll back")></button>
                        })}
                        {b(&v, "custom_binary").then(|| view! {
                            <button class="btn sm danger" disabled=move || busy.get() || running()
                                on:click=move |_| r4("Reset to the image build?".into(),
                                    "Remove the custom build and go back to the Caddy that ships in the image? Plugins that are not in the image stop working.".into(),
                                    "Reset", true, "/plugins/reset", json!({}))
                                inner_html=ib("trash", "Reset to image")></button>
                        })}
                    </div>
                }
            });
            view! {
                {(!b(&v, "supported")).then_some(reason).map(|reason| view! {
                    <Alert kind="warn"><strong>"Installing is not available: "</strong>{reason}
                        ". The panel needs write access to the shared " <code>"caddy-bin"</code> " volume (see the compose file) and a running Caddy."</Alert>
                })}
                <div class="card row wrap" style="gap:28px">
                    <div><div class="muted small">"Caddy build"</div>
                        <strong>{if s(&v, "caddy_version").is_empty() { "unknown".into() } else { s(&v, "caddy_version") }}</strong>
                        " "<span class="muted small">{format!("{}/{}", s(&v, "os"), s(&v, "arch"))}</span></div>
                    <div><div class="muted small">"Source"</div>
                        {if b(&v, "custom_binary") { view! { <span class="badge accent">"custom build"</span> } } else { view! { <span class="badge">"image build"</span> } }}</div>
                    <div><div class="muted small">"Latest release"</div>
                        <strong>{if latest.is_empty() { "—".into() } else { latest.clone() }}</strong>" "
                        {if b(&v, "update_available") {
                            Some(view! { <span class="badge warn">"update available"</span> }.into_any())
                        } else if !latest.is_empty() {
                            Some(view! { <span class="badge ok">"up to date"</span> }.into_any())
                        } else { None }}</div>
                    {actions}
                </div>
            }
        }
    };

    let updates_card = move || {
        let u = upd.get();
        let checked = s(&u, "last_check");
        let next = s(&u, "next_check");
        let err = s(&u, "last_error");
        let line = if checked.is_empty() {
            "Not checked yet".to_string()
        } else if next.is_empty() {
            format!("Last checked {}", when(&checked))
        } else {
            format!("Last checked {} · next {}", when(&checked), when(&next))
        };
        let auto = u.get("last_auto").filter(|x| x.is_object()).cloned().map(|x| {
            let from = s(&x, "from");
            let from = if from.is_empty() { String::new() } else { format!("from {from} ") };
            if b(&x, "ok") {
                view! { <Alert kind="ok">{format!("Automatic update {from}to {} installed {}", s(&x, "to"), when(&s(&x, "at")))}</Alert> }.into_any()
            } else {
                view! { <ErrorBox title=format!("Automatic update {from}to {} failed {}", s(&x, "to"), when(&s(&x, "at"))) message=s(&x, "error")/> }.into_any()
            }
        });
        view! {
            <div class="card stack">
                <div class="row wrap">
                    <div class="grow"><h2 style="margin:0">"Updates"</h2>
                        <div class="muted small">{line}{(!err.is_empty()).then(|| view! { " · " <span class="warn-text">{err.clone()}</span> })}</div></div>
                    {admin.then(|| view! { <button class="btn sm" on:click=check_now disabled=move || busy.get() inner_html=ib("refresh", "Check now")></button> })}
                </div>
                {auto}
                {move || (!us.get().is_null()).then(|| update_form(us, upd, busy, admin, save_updates))}
            </div>
        }
    };

    let job_card = move || {
        let j = st.get()["job"].clone();
        if !j.is_object() {
            return None;
        }
        let status = s(&j, "status");
        let lines: Vec<String> = a(&j, "log").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        let n = a(&j, "packages").len();
        let started = format!("Started {}{}", when(&s(&j, "started")), if n > 0 { format!(" · {n} plugin(s)") } else { String::new() });
        let ic = match status.as_str() {
            "running" => "<span class=\"spinner\"></span>".to_string(),
            "done" => icon("circle-check"),
            _ => icon("x"),
        };
        Some(view! {
            <div class="card stack" class:job-running=status == "running">
                <div class="row wrap">
                    <span inner_html=ic></span>
                    <div class="grow"><h2 style="margin:0">{job_title(&j)}</h2><div class="muted small">{started}</div></div>
                    <button class="btn sm" on:click=move |_| show_log.update(|x| *x = !*x)>{move || if show_log.get() { "Hide log" } else { "Show log" }}</button>
                </div>
                {(status == "failed").then(|| view! { <ErrorBox title="The plugin operation failed" message=s(&j, "error")/> })}
                {move || show_log.get().then(|| view! { <pre class="code-block scroll-55">{lines.join("\n")}</pre> })}
            </div>
        })
    };

    let installed_card = move || {
        let list = a(&st.get(), "installed");
        let body = if list.is_empty() {
            view! { <p class="muted">"Only standard modules. Pick plugins below to build a custom Caddy."</p> }.into_any()
        } else {
            let rows = list
                .into_iter()
                .map(|p| {
                    let path = s(&p, "path");
                    let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
                    let mods: Vec<String> = a(&p, "modules").iter().filter_map(|m| m.as_str().map(str::to_string)).collect();
                    let off = Memo::new(move |_| del.get().contains(&p1));
                    let toggle = move |_| {
                        let p = p2.clone();
                        del.update(|l| if l.contains(&p) { l.retain(|x| x != &p) } else { l.push(p) });
                    };
                    view! {
                        <tr class:off=move || off.get()>
                            <td><strong>{s(&p, "name")}</strong><div class="muted small truncate">{mods.join(", ")}</div></td>
                            <td class="hide-sm mono small">{path}</td>
                            <td class="mono small">{s(&p, "version")}</td>
                            <td class="right">{admin.then(|| view! {
                                <button class="btn sm" class:danger=move || !del.get().contains(&p3) on:click=toggle disabled=move || running() || !supported()>
                                    {move || if off.get() { "Keep" } else { "Remove" }}</button>
                            })}</td>
                        </tr>
                    }
                })
                .collect_view();
            view! {
                <div class="table-wrap"><table class="table">
                    <thead><tr><th>"Plugin"</th><th class="hide-sm">"Package"</th><th>"Version"</th><th></th></tr></thead>
                    <tbody>{rows}</tbody>
                </table></div>
            }
            .into_any()
        };
        view! {
            <div class="card stack">
                <div><h2 style="margin:0">"Installed"</h2><div class="muted small">"Plugins compiled into the running Caddy"</div></div>
                {body}
            </div>
        }
    };

    let catalog = move || {
        let v = st.get();
        let installed: Vec<String> = a(&v, "installed").iter().map(|p| s(p, "path")).collect();
        let all = a(&v, "catalog");
        let (c, qq) = (cat.get(), q.get());
        let shown: Vec<Value> = all.iter().filter(|p| matches(p, &c, &qq)).cloned().collect();
        let total = shown.len();
        let rows = shown
            .into_iter()
            .take(120)
            .map(|p| {
                let path = s(&p, "path");
                let inst = installed.contains(&path);
                let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
                let queued = Memo::new(move |_| add.get().contains(&p1));
                let toggle = move |_| {
                    let p = p2.clone();
                    add.update(|l| if l.contains(&p) { l.retain(|x| x != &p) } else { l.push(p) });
                };
                let dl = i(&p, "downloads");
                let repo = s(&p, "repo");
                view! {
                    <div class="prow" class:queued=move || queued.get()>
                        <div class="grow" style="min-width:0">
                            <div class="row wrap" style="gap:6px"><strong>{s(&p, "name")}</strong>
                                {inst.then(|| view! { <span class="badge ok">"installed"</span> })}
                                {b(&p, "featured").then(|| view! { <span class="badge accent">"featured"</span> })}</div>
                            <div class="muted small">{s(&p, "description")}</div>
                            <div class="mono small muted truncate">{path}{(dl > 0).then(|| format!(" · {} downloads", downloads(dl)))}</div>
                        </div>
                        {(!repo.is_empty()).then(|| view! { <a class="btn sm" href=repo target="_blank" rel="noreferrer" title="Repository" inner_html=icon("external")></a> })}
                        {(admin && !inst).then(|| view! {
                            <button class="btn sm" class:primary=move || !add.get().contains(&p3) on:click=toggle disabled=move || running() || !supported()>
                                {move || if queued.get() { "Undo" } else { "Add" }}</button>
                        })}
                    </div>
                }
            })
            .collect_view();
        let fetched = s(&v, "catalog_fetched_at");
        view! {
            <div class="card stack">
                <div class="row wrap">
                    <div class="grow"><h2 style="margin:0">"Catalog"</h2>
                        <div class="muted small">{format!("{} packages{}", all.len(), if fetched.is_empty() { String::new() } else { format!(" · updated {}", when(&fetched)) })}</div></div>
                    <button class="btn sm" on:click=refresh_catalog disabled=move || busy.get() inner_html=ib("refresh", "Refresh catalog")></button>
                </div>
                {if total == 0 {
                    view! { <p class="muted">"Nothing matches."</p> }.into_any()
                } else {
                    view! {
                        <div class="stack" style="gap:6px">{rows}</div>
                        {(total > 120).then(|| view! { <p class="muted small">{format!("Showing 120 of {total}; narrow the search.")}</p> })}
                    }
                    .into_any()
                }}
            </div>
        }
    };
    let chips = CATEGORIES
        .iter()
        .map(|(k, l)| {
            let k = k.to_string();
            let k2 = k.clone();
            view! { <button class="chip" class:active=move || cat.get() == k on:click=move |_| cat.set(k2.clone())>{*l}</button> }
        })
        .collect_view();

    let action_bar = {
        let run = run.clone();
        move || {
            let (ad, rm) = (add.get(), del.get());
            if !admin || ad.len() + rm.len() == 0 {
                return None;
            }
            let run = run.clone();
            let n = ad.len() + rm.len();
            let names = |l: &[String]| l.iter().map(|p| short(p)).collect::<Vec<_>>().join(", ");
            let (an, rn) = (names(&ad), names(&rm));
            let body = json!({ "add": ad, "remove": rm });
            Some(view! {
                <div class="actionbar">
                    <div class="grow small">
                        {(!an.is_empty()).then(|| view! { <span>"Add: "<span class="mono">{an.clone()}</span></span> })}
                        {(!rn.is_empty()).then(|| view! { <span>" Remove: "<span class="mono">{rn.clone()}</span></span> })}
                    </div>
                    <button class="btn" on:click=move |_| { add.set(vec![]); del.set(vec![]); }>"Discard"</button>
                    <button class="btn primary" disabled=move || busy.get() || running()
                        on:click=move |_| run("Build and restart Caddy?".into(),
                            format!("Download a new Caddy build with {n} change(s) and restart Caddy? Sites are unavailable for a few seconds during the restart."),
                            "Build & restart", false, "/plugins/install", body.clone())
                        inner_html=ib("download", "Build & restart Caddy")></button>
                </div>
            })
        }
    };

    let page = move || {
        if st.get().is_null() {
            return view! { <Loading/> }.into_any();
        }
        view! {
            <div class="stack">
                {build_card.clone()}
                {updates_card}
                {job_card}
                {installed_card}
                <div class="row wrap">
                    <input class="input grow" type="search" placeholder="Search packages, modules…" style="min-width:200px"
                        prop:value=move || q.get() on:input=move |e| q.set(event_target_value(&e))/>
                    <div class="row wrap" style="gap:6px">{chips.clone()}</div>
                </div>
                {catalog}
            </div>
            {action_bar.clone()}
        }
        .into_any()
    };
    let loaded = Memo::new(move |_| !st.get().is_null());
    view! {
        <PageHead title="Plugins" text="Caddy plugins are compiled in. Caddymium fetches a build with your selection from the official download service (what caddy add-package uses), checks your config against it, then restarts Caddy. The previous build is kept so you can roll back.">
            <button class="btn" on:click=move |_| load() disabled=move || busy.get() inner_html=ib("refresh", "Reload")></button>
        </PageHead>
        {move || {
            loaded.get();
            untrack(page.clone())
        }}
    }
}

fn update_form(
    us: RwSignal<Value>,
    upd: RwSignal<Value>,
    busy: RwSignal<bool>,
    admin: bool,
    save: impl Fn(leptos::ev::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    let every = i(&us.get_untracked()["updates"], "check_every_hours");
    let mut opts: Vec<(String, String)> = INTERVALS.iter().map(|(h, l)| (h.to_string(), l.to_string())).collect();
    if every > 0 && !INTERVALS.iter().any(|(h, _)| *h == every) {
        opts.push((every.to_string(), format!("{every} hours")));
    }
    let check = Memo::new(move |_| b(&us.get()["updates"], "check"));
    let auto = Memo::new(move |_| b(&us.get()["updates"], "auto"));
    view! {
        <fieldset class="stack plain" disabled=!admin>
            <Check state=us path="updates.check" label="Check for new Caddy releases in the background"/>
            {move || check.get().then(|| {
                let opts = opts.clone();
                view! {
                    <div class="form-grid"><Select state=us path="updates.check_every_hours" label="Check every" options=opts/></div>
                    <Check state=us path="updates.auto" label="Install new releases automatically"
                        hint="Same path as Update Caddy: the new build is downloaded with your plugins, checked, and your configuration is validated against it before it replaces the running one. Caddy restarts for a few seconds. A build that fails any check is discarded, the old one keeps running, and you are notified."/>
                    {move || auto.get().then(|| {
                        let u = upd.get();
                        let outside = !u.is_null() && !b(&u, "in_window");
                        let blocked = s(&u, "auto_blocked");
                        view! {
                            <div class="form-grid">
                                <Text state=us path="updates.window_start" label="Only between" kind="time"/>
                                <Text state=us path="updates.window_end" label="and" kind="time"/>
                            </div>
                            <small class="muted">"Server local time (set " <code>"TZ"</code> " on the container to change it). Leave both empty to install as soon as a release is found."
                                {outside.then_some(" Outside the window right now.")}</small>
                            {(b(&u, "update_available") && !blocked.is_empty()).then(|| view! { <div class="muted small">{format!("Not installing yet: {blocked}.")}</div> })}
                        }
                    })}
                }
            })}
            {admin.then(|| view! { <div><button class="btn primary sm" on:click=save disabled=move || busy.get()>"Save"</button></div> })}
        </fieldset>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_filters() {
        let p = json!({ "path": "github.com/caddy-dns/cloudflare", "name": "cloudflare", "category": "dns", "featured": true, "modules": ["dns.providers.cloudflare"] });
        assert!(matches(&p, "featured", ""));
        assert!(matches(&p, "dns", "providers.cloud"));
        assert!(!matches(&p, "security", ""));
        assert!(matches(&p, "all", "CLOUD"));
        assert!(!matches(&p, "all", "route53"));
        assert_eq!(downloads(1_250_000), "1.2M");
        assert_eq!(downloads(34_567), "34k");
        assert_eq!(short("github.com/a/b"), "b");
    }
}
