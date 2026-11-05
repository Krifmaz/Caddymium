//! The kit's own pages: bug reports, Diagnostics and What's new.

use crate::app::{a, i, s};
use crate::icons::icon;
use crate::ui::{ErrorBox, Loading, PageHead};
use crate::APP;
use kit_leptos::api;
use kit_leptos::bug_report::{self, SheetOpts};
use kit_leptos::diagnostics::DiagnosticsPage;
use kit_leptos::whats_new::ChangelogPage;
use leptos::prelude::*;
use serde_json::Value;
use std::rc::Rc;

/// Opens the New report sheet for the page `from`, with the server's version.
pub fn new_report(from: &'static str, after: Option<Rc<dyn Fn()>>) {
    kit_leptos::rt::spawn(async move {
        let version = api::get("/health").await.ok().map(|h| s(&h, "version")).unwrap_or_default();
        let mut o = SheetOpts::new(APP, from, &version);
        o.on_sent = Some(Rc::new(move |_| {
            if let Some(f) = &after {
                f();
            }
        }));
        bug_report::open(o);
    });
}

/// One report of the list (title, author, status and replies).
fn report_row(r: &Value) -> impl IntoView {
    let n = i(r, "message_count");
    let count = match n {
        0 => "No replies".to_string(),
        1 => "1 reply".to_string(),
        n => format!("{n} replies"),
    };
    view! {
        <div class="list-item bug-row" style="flex-wrap:wrap;gap:10px">
            <div class="grow" style="min-width:200px">
                <span class="bug-title" style="font-weight:650">{s(r, "title")}</span>
                <div class="small faint">{format!("by {} · {}", s(r, "username"), s(r, "kind"))}</div>
            </div>
            <span class="badge">{s(r, "status")}</span>
            <span class="small muted bug-count">{count}</span>
        </div>
    }
}

/// `#/bug-reports`: the account's reports (an admin's: everyone's) and New report.
#[component]
pub fn BugReports() -> impl IntoView {
    let gen = RwSignal::new(0u32);
    let list = LocalResource::new(move || {
        gen.get();
        async { api::get("/bug-reports").await.map_err(|e| e.message) }
    });
    let rows = move || match list.get() {
        None => view! { <Loading/> }.into_any(),
        Some(Err(e)) => view! { <ErrorBox message=e/> }.into_any(),
        Some(Ok(v)) => {
            let reports = a(&v, "reports");
            if reports.is_empty() {
                view! { <div class="empty" inner_html=format!(r#"{}<p class="muted">No reports yet. New report writes the first one.</p>"#, icon("bug"))></div> }.into_any()
            } else {
                view! { <div class="list">{reports.iter().map(report_row).collect_view()}</div> }.into_any()
            }
        }
    };
    let open = move |_| new_report("#/bug-reports", Some(Rc::new(move || gen.update(|g| *g += 1))));
    view! {
        <PageHead title="Bug reports" text="Something not working, or not clear? Tell the admin, with screenshots.">
            <button class="btn primary" on:click=open inner_html=format!("{}New report", icon("plus"))></button>
        </PageHead>
        <div class="card flat mt">{rows}</div>
    }
}

/// `#/diagnostics`: the panel's checks (Caddy, listeners, data folder, database, logs).
#[component]
pub fn Diagnostics() -> impl IntoView {
    view! { <DiagnosticsPage app=APP/> }
}

/// `#/whats-new`: every release's notes and the popup switch.
#[component]
pub fn WhatsNew() -> impl IntoView {
    view! { <ChangelogPage app=APP/> }
}
