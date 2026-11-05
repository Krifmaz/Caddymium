//! `#/sites`: everything Caddy serves, as cards or a list, with search, tags, on/off, bulk actions and order.

use crate::app::{self, a, b, i, s};
use crate::icons::icon;
use crate::pages::deploy::ib;
use crate::sites::{found, kind_label, protected, target, url};
use crate::ui::{ago, load, reload, show, sure, toast_err, Avatar, PageHead};
use kit_leptos::toasts::toast;
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

const VIEW_KEY: &str = "caddymium.sites-view";

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "proxy" => "server",
        "static" => "folder",
        "redirect" => "corner-up-right",
        "respond" => "message",
        "forward_proxy" => "arrow-left-right",
        _ => "code",
    }
}

/// Upstream reachability: `upstream → {ok, error}` from `/upstreams/reach`.
type Reach = HashMap<String, Value>;

/// ("ok" | "err", first error) for a proxy site, None when nothing was checked.
pub fn health(site: &Value, reach: &Reach) -> Option<(&'static str, String)> {
    if s(site, "kind") != "proxy" || !b(site, "enabled") {
        return None;
    }
    let ups: Vec<&Value> =
        a(&site["config"]["proxy"], "upstreams").iter().filter_map(|u| u.as_str()).filter_map(|u| reach.get(u.trim())).collect();
    if ups.is_empty() {
        return None;
    }
    match ups.iter().find(|u| !b(u, "ok")) {
        None => Some(("ok", String::new())),
        Some(bad) => Some(("err", s(bad, "error"))),
    }
}

fn storage() -> Option<web_sys::Storage> {
    rt::window()?.local_storage().ok().flatten()
}

async fn bulk(ids: Vec<i64>, action: &str) -> Result<(), kit_leptos::api::ApiError> {
    api::post("/sites/bulk", json!({ "ids": ids, "action": action })).await.map(|_| ())
}

/// `#/sites`.
#[component]
pub fn Sites() -> impl IntoView {
    let sites = load("/sites");
    let reach = RwSignal::new(Reach::new());
    rt::spawn(async move {
        if let Ok(v) = api::get("/upstreams/reach").await {
            reach.set(v.as_array().into_iter().flatten().map(|u| (s(u, "upstream"), u.clone())).collect());
        }
    });
    let q = RwSignal::new(String::new());
    let tag = RwSignal::new(String::new());
    let selected = RwSignal::new(BTreeSet::<i64>::new());
    let start = storage().and_then(|st| st.get_item(VIEW_KEY).ok().flatten()).unwrap_or_else(|| "grid".into());
    let view_mode = RwSignal::new(start);
    let set_view = move |v: &str| {
        view_mode.set(v.to_string());
        if let Some(st) = storage() {
            let _ = st.set_item(VIEW_KEY, v);
        }
    };
    let editor = app::can_edit();
    let refresh = move || reload(sites, "/sites");

    let toggle = move |id: i64, on: bool| {
        rt::spawn(async move {
            match bulk(vec![id], if on { "enable" } else { "disable" }).await {
                Ok(()) => app::refresh_status(),
                Err(e) => {
                    toast_err(e);
                    refresh();
                }
            }
        });
    };
    let bulk_action = move |action: &'static str| {
        let ids: Vec<i64> = selected.get_untracked().into_iter().collect();
        if ids.is_empty() {
            return;
        }
        rt::spawn(async move {
            if action == "delete"
                && !sure(&format!("Delete {} site(s)?", ids.len()), "History keeps the old Caddyfiles.", "Delete", true).await
            {
                return;
            }
            let n = ids.len();
            match bulk(ids, action).await {
                Ok(()) => {
                    selected.set(BTreeSet::new());
                    toast(&format!("Done: {action} {n} site(s)"), "ok");
                    refresh();
                    app::refresh_status();
                }
                Err(e) => toast_err(e),
            }
        });
    };
    let move_site = move |list: Vec<Value>, at: usize, up: bool| {
        let j = if up { at.checked_sub(1) } else { Some(at + 1) };
        let Some(j) = j.filter(|j| *j < list.len()) else { return };
        let mut list = list;
        list.swap(at, j);
        let ids: Vec<i64> = list.iter().map(|x| i(x, "id")).collect();
        sites.set(Some(Ok(Value::Array(list))));
        rt::spawn(async move {
            match bulk(ids, "reorder").await {
                Ok(()) => app::refresh_status(),
                Err(e) => {
                    toast_err(e);
                    refresh();
                }
            }
        });
    };

    let stats = move || {
        let st = app::status().get();
        let r = reach.get();
        let (up_total, up_ok) = (r.len(), r.values().filter(|u| b(u, "ok")).count());
        let caddy = if st.is_null() {
            view! { "–" }.into_any()
        } else if b(&st, "caddy_reachable") {
            let v = s(&st, "caddy_version");
            view! { <span class="dot ok"></span>{if v.is_empty() { "running".to_string() } else { v }} }.into_any()
        } else {
            view! { <span class="dot err"></span>"offline" }.into_any()
        };
        let last = &st["last_apply"];
        let last_view = if last.is_object() {
            let cls = if b(last, "good") { "dot ok" } else { "dot err" };
            view! { <span class=cls></span>{ago(&s(last, "created_at"))} }.into_any()
        } else {
            view! { "never" }.into_any()
        };
        let num = |k: &str| st.get(k).and_then(Value::as_i64).map(|n| n.to_string()).unwrap_or("–".into());
        view! {
            <div class="stats">
                <div class="stat"><span class="s-ic" inner_html=icon("layers")></span>
                    <div><div class="s-label">"Active sites"</div><div class="s-val">{num("sites_enabled")}<span class="muted">{format!(" / {}", num("sites_total"))}</span></div></div></div>
                <div class="stat"><span class="s-ic" inner_html=icon("activity")></span>
                    <div><div class="s-label">"Apps reachable"</div><div class="s-val">{if up_total > 0 { up_ok.to_string() } else { "–".into() }}
                        <span class="muted">{format!(" / {}", if up_total > 0 { up_total.to_string() } else { "–".into() })}</span></div></div></div>
                <div class="stat"><span class="s-ic" inner_html=icon("server")></span>
                    <div><div class="s-label">"Caddy"</div><div class="s-val row" style="gap:6px">{caddy}</div></div></div>
                <a class="stat" href="#/history"><span class="s-ic" inner_html=icon("rocket")></span>
                    <div><div class="s-label">"Last apply"</div><div class="s-val row" style="gap:6px">{last_view}</div></div></a>
            </div>
        }
    };

    let body = show(sites, move |v| {
        let all = v.as_array().cloned().unwrap_or_default();
        if all.is_empty() {
            return view! {
                <div class="card empty stack" style="align-items:center">
                    <span class="s-ic" inner_html=icon("globe")></span>
                    <h2 style="margin:0">"No sites yet"</h2>
                    <p class="muted" style="margin:0">"Add your first site, or bring in the Caddyfile you already have."</p>
                    <div class="row wrap" style="justify-content:center">
                        {editor.then(|| view! { <a class="btn primary" href="#/sites/new" inner_html=ib("plus", "New site")></a> })}
                        {app::is_admin().then(|| view! { <a class="btn" href="#/import" inner_html=ib("file-input", "Import a Caddyfile")></a> })}
                    </div>
                </div>
            }
            .into_any();
        }
        let tags: BTreeSet<String> = all.iter().flat_map(|x| a(x, "tags")).filter_map(|t| t.as_str().map(str::to_string)).collect();
        let tag_opts = tags.into_iter().map(|t| view! { <option value=t.clone()>{t.clone()}</option> }).collect_view();
        let has_tags = !all.iter().all(|x| a(x, "tags").is_empty());
        let all2 = all.clone();
        let list = move || {
            let (qq, tg) = (q.get().to_lowercase(), tag.get());
            let shown: Vec<Value> = all2
                .iter()
                .filter(|x| tg.is_empty() || a(x, "tags").iter().any(|t| t.as_str() == Some(tg.as_str())))
                .filter(|x| found(x, &qq))
                .cloned()
                .collect();
            let filtered = !qq.is_empty() || !tg.is_empty();
            if shown.is_empty() {
                return view! { <div class="card empty">"No sites match your search."</div> }.into_any();
            }
            let r = reach.get();
            if view_mode.get() == "grid" {
                grid(shown, &r, editor && !filtered, editor, toggle).into_any()
            } else {
                table(shown, all2.clone(), &r, editor, !filtered, selected, toggle, move_site).into_any()
            }
        };
        view! {
            <div class="row wrap" style="margin-bottom:14px">
                <input class="input" type="search" placeholder="Search hosts, upstreams, tags…" style="flex:1;max-width:400px;min-width:200px"
                    prop:value=move || q.get() on:input=move |e| q.set(event_target_value(&e))/>
                {has_tags.then(|| view! {
                    <select class="input" style="width:auto;min-width:130px" on:change=move |e| tag.set(event_target_value(&e))>
                        <option value="">"All tags"</option>{tag_opts}
                    </select>
                })}
                {move || (editor && view_mode.get() == "list" && !selected.get().is_empty()).then(|| view! {
                    <span class="muted small">{format!("{} selected", selected.get().len())}</span>
                    <button class="btn sm" on:click=move |_| bulk_action("enable")>"Enable"</button>
                    <button class="btn sm" on:click=move |_| bulk_action("disable")>"Disable"</button>
                    <button class="btn sm danger" on:click=move |_| bulk_action("delete")>"Delete"</button>
                })}
                <div class="tabs" style="margin-left:auto" role="group" aria-label="View">
                    <button class:active=move || view_mode.get() == "grid" on:click=move |_| set_view("grid") aria-label="Grid view" title="Grid view" inner_html=icon("grid")></button>
                    <button class:active=move || view_mode.get() == "list" on:click=move |_| set_view("list") aria-label="List view" title="List view" inner_html=icon("list")></button>
                </div>
            </div>
            {list}
        }
        .into_any()
    });

    view! {
        <PageHead title="Sites" text="Everything Caddy serves. Edit freely: nothing goes live until you review and apply.">
            {editor.then(|| view! { <a class="btn primary" href="#/sites/new" inner_html=ib("plus", "New site")></a> })}
        </PageHead>
        {stats}
        {body}
    }
}

fn switch(id: i64, on: bool, editor: bool, toggle: impl Fn(i64, bool) + Copy + 'static) -> impl IntoView {
    view! {
        <label class="switch" title="Enabled" on:click=|e| e.stop_propagation()>
            <input type="checkbox" prop:checked=on disabled=!editor on:change=move |e| toggle(id, event_target_checked(&e))/><i></i>
        </label>
    }
}

fn grid(shown: Vec<Value>, reach: &Reach, add: bool, editor: bool, toggle: impl Fn(i64, bool) + Copy + 'static) -> impl IntoView {
    let cards = shown
        .into_iter()
        .map(|x| {
            let id = i(&x, "id");
            let hosts: Vec<String> = a(&x, "hosts").iter().filter_map(|h| h.as_str().map(str::to_string)).collect();
            let first = hosts.first().cloned().unwrap_or_default();
            let name = s(&x, "name");
            let avatar_name = if name.is_empty() { first.clone() } else { name.clone() };
            let h = health(&x, reach);
            let open = url(&x).filter(|_| b(&x, "enabled"));
            let tags: Vec<String> = a(&x, "tags").iter().take(3).filter_map(|t| t.as_str().map(str::to_string)).collect();
            view! {
                <div class="card site" class:off=!b(&x, "enabled") role="link" tabindex="0"
                    on:click=move |_| app::navigate(&format!("/sites/{id}"))
                    on:keydown=move |e| if e.key() == "Enter" { app::navigate(&format!("/sites/{id}")) }>
                    <div class="row" style="gap:12px">
                        <Avatar name=avatar_name size=40/>
                        <div class="grow" style="min-width:0">
                            <div class="truncate" style="font-weight:600">{name}</div>
                            <div class="truncate muted small">{first}{(hosts.len() > 1).then(|| view! { <span class="muted">{format!(" +{}", hosts.len() - 1)}</span> })}</div>
                        </div>
                        {switch(id, b(&x, "enabled"), editor, toggle)}
                    </div>
                    <div class="site-target">
                        <span inner_html=icon(kind_icon(&s(&x, "kind")))></span>
                        <span class="muted small" style="flex:none">{kind_label(&s(&x, "kind"))}</span>
                        <span class="mono truncate">{target(&x)}</span>
                    </div>
                    {h.clone().filter(|(k, _)| *k == "err").map(|(_, e)| view! { <div class="reach-err">{e}</div> })}
                    <div class="row wrap" style="gap:6px;min-height:22px">
                        {h.map(|(k, e)| {
                            let title = if e.is_empty() { "Caddy can connect to this app".to_string() } else { e };
                            view! { <span class=format!("badge {k}") title=title><span class=format!("dot {k}")></span>{if k == "ok" { "reachable" } else { "can't reach app" }}</span> }
                        })}
                        {b(&x["config"], "maintenance").then(|| view! { <span class="badge warn">"maintenance"</span> })}
                        {protected(&x).then(|| view! { <span class="badge" inner_html=format!("{}protected", icon("lock"))></span> })}
                        {tags.into_iter().map(|t| view! { <span class="badge">{t}</span> }).collect_view()}
                        {open.map(|u| view! {
                            <a class="btn sm" style="margin-left:auto" href=u target="_blank" rel="noreferrer" title="Open site" aria-label="Open site"
                                on:click=|e| e.stop_propagation() inner_html=icon("external")></a>
                        })}
                    </div>
                </div>
            }
        })
        .collect_view();
    view! {
        <div class="site-grid">
            {cards}
            {add.then(|| view! { <a class="card site-add" href="#/sites/new" inner_html=format!("{}<span>Add a site</span>", icon("plus"))></a> })}
        </div>
    }
}

#[allow(clippy::too_many_arguments)]
fn table(
    shown: Vec<Value>,
    all: Vec<Value>,
    reach: &Reach,
    editor: bool,
    order: bool,
    selected: RwSignal<BTreeSet<i64>>,
    toggle: impl Fn(i64, bool) + Copy + 'static,
    move_site: impl Fn(Vec<Value>, usize, bool) + Copy + 'static,
) -> impl IntoView {
    let n = shown.len();
    let rows = shown
        .into_iter()
        .enumerate()
        .map(|(at, x)| {
            let id = i(&x, "id");
            let hosts: Vec<String> = a(&x, "hosts").iter().filter_map(|h| h.as_str().map(str::to_string)).collect();
            let name = s(&x, "name");
            let avatar_name = if name.is_empty() { hosts.first().cloned().unwrap_or_default() } else { name.clone() };
            let h = health(&x, reach).map(|(k, _)| k);
            let (l1, l2) = (all.clone(), all.clone());
            view! {
                <tr class="clickable" class:off=!b(&x, "enabled") on:click=move |_| app::navigate(&format!("/sites/{id}"))>
                    {editor.then(|| view! {
                        <td on:click=|e| e.stop_propagation()><input type="checkbox" aria-label="Select"
                            prop:checked=move || selected.get().contains(&id)
                            on:change=move |e| { let on = event_target_checked(&e); selected.update(|s| { if on { s.insert(id); } else { s.remove(&id); } }); }/></td>
                    })}
                    <td>
                        <div class="row" style="gap:8px;flex-wrap:nowrap">
                            <Avatar name=avatar_name size=28/>
                            <div style="min-width:0">
                                <div class="row" style="gap:6px"><strong>{name}</strong>{h.map(|k| view! { <span class=format!("dot {k}")></span> })}</div>
                                <div class="muted small truncate">{hosts.join(", ")}</div>
                            </div>
                        </div>
                    </td>
                    <td class="hide-sm muted">{kind_label(&s(&x, "kind"))}</td>
                    <td class="hide-sm mono small truncate" style="max-width:280px">{target(&x)}</td>
                    <td on:click=|e| e.stop_propagation()>{switch(id, b(&x, "enabled"), editor, toggle)}</td>
                    {(editor && order).then(|| view! {
                        <td class="hide-sm right" on:click=|e| e.stop_propagation()>
                            <button class="btn sm" aria-label="Move up" title="Move up" disabled=at == 0 on:click=move |_| move_site(l1.clone(), at, true) inner_html=icon("arrow-up")></button>" "
                            <button class="btn sm" aria-label="Move down" title="Move down" disabled=at + 1 == n on:click=move |_| move_site(l2.clone(), at, false) inner_html=icon("arrow-down")></button>
                        </td>
                    })}
                </tr>
            }
        })
        .collect_view();
    view! {
        <div class="card flat table-wrap"><table class="table">
            <thead><tr>
                {editor.then(|| view! { <th style="width:36px"></th> })}
                <th>"Site"</th><th class="hide-sm">"Type"</th><th class="hide-sm">"Target"</th><th style="width:64px">"On"</th>
                {(editor && order).then(|| view! { <th class="hide-sm" style="width:100px">"Order"</th> })}
            </tr></thead>
            <tbody>{rows}</tbody>
        </table></div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_of_proxies() {
        let site = json!({ "kind": "proxy", "enabled": true, "config": { "proxy": { "upstreams": ["a:1", "b:2"] } } });
        let mut r = Reach::new();
        assert_eq!(health(&site, &r), None);
        r.insert("a:1".into(), json!({ "ok": true }));
        assert_eq!(health(&site, &r), Some(("ok", String::new())));
        r.insert("b:2".into(), json!({ "ok": false, "error": "refused" }));
        assert_eq!(health(&site, &r), Some(("err", "refused".to_string())));
        assert_eq!(health(&json!({ "kind": "static", "enabled": true }), &r), None);
    }
}
