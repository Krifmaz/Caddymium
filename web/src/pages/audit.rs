//! `#/audit`: who changed what, and when.

use crate::app::s;
use crate::ui::{ago, when, ErrorBox, PageHead};
use kit_leptos::{api, rt};
use leptos::prelude::*;
use serde_json::Value;

const LIMIT: usize = 100;

/// `q` as a URL query value.
fn enc(q: &str) -> String {
    String::from(js_sys::encode_uri_component(q))
}

/// The audit log with search and paging.
#[component]
pub fn Audit() -> impl IntoView {
    let entries = RwSignal::new(Vec::<Value>::new());
    let error = RwSignal::new(None::<String>);
    let q = RwSignal::new(String::new());
    let offset = RwSignal::new(0usize);
    let seq = StoredValue::new(0u32);
    let load = move || {
        let (q, off) = (q.get_untracked(), offset.get_untracked());
        rt::spawn(async move {
            match api::get(&format!("/audit?limit={LIMIT}&offset={off}&q={}", enc(&q))).await {
                Ok(v) => {
                    entries.set(v.as_array().cloned().unwrap_or_default());
                    error.set(None);
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    load();
    let search = move |e: leptos::ev::Event| {
        q.set(event_target_value(&e));
        let n = seq.get_value() + 1;
        seq.set_value(n);
        rt::after(250, move || {
            if seq.get_value() == n {
                offset.set(0);
                load();
            }
        });
    };
    let rows = move || {
        entries
            .get()
            .into_iter()
            .map(|e| {
                view! {
                    <tr>
                        <td class="nowrap" title=when(&s(&e, "ts"))>{ago(&s(&e, "ts"))}</td>
                        <td>{s(&e, "user")}</td>
                        <td class="mono small">{s(&e, "action")}</td>
                        <td class="hide-sm">{s(&e, "target")}</td>
                        <td class="hide-sm small">{s(&e, "details")}</td>
                        <td class="hide-sm mono small">{s(&e, "ip")}</td>
                    </tr>
                }
            })
            .collect_view()
    };
    view! {
        <PageHead title="Audit log" text="Who changed what, and when."/>
        <input class="input" placeholder="Search user, action, target…" style="max-width:360px;margin-bottom:12px" on:input=search/>
        {move || error.get().map(|e| view! { <ErrorBox message=e/> })}
        <div class="card flat table-wrap">
            <table class="table">
                <thead><tr><th>"When"</th><th>"User"</th><th>"Action"</th><th class="hide-sm">"Target"</th><th class="hide-sm">"Details"</th><th class="hide-sm">"IP"</th></tr></thead>
                <tbody>{rows}</tbody>
            </table>
            {move || entries.get().is_empty().then(|| view! { <div class="empty">"No entries."</div> })}
        </div>
        <div class="row mt">
            <button class="btn sm" disabled=move || offset.get() == 0 on:click=move |_| { offset.update(|o| *o = o.saturating_sub(LIMIT)); load(); }>"← Newer"</button>
            <button class="btn sm" disabled=move || entries.get().len() < LIMIT on:click=move |_| { offset.update(|o| *o += LIMIT); load(); }>"Older →"</button>
        </div>
    }
}
