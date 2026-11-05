//! Small parts every page uses: errors with Copy, loading states, page headings, the sidebar's Caddy status and
//! the "unapplied changes" bar.

use crate::app::{self, b, s, Route};
use crate::icons::icon;
use kit_leptos::details::CopyButton;
use leptos::prelude::*;
use serde_json::Value;

/// An error box: the whole sentence, selectable, with Copy.
#[component]
pub fn ErrorBox(#[prop(into)] message: String) -> impl IntoView {
    view! {
        <div class="error-box row" role="alert" style="align-items:flex-start">
            <span class="grow" style="white-space:pre-wrap;user-select:text">{message.clone()}</span>
            <CopyButton text=message/>
        </div>
    }
}

/// The spinner a page shows while it loads.
#[component]
pub fn Loading() -> impl IntoView {
    view! { <div class="empty"><span class="spinner"></span></div> }
}

/// A page's heading with its sentence and its buttons on the right.
#[component]
pub fn PageHead(
    #[prop(into)] title: String,
    #[prop(optional, into)] text: String,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="row wrap between page-head">
            <div class="grow" style="min-width:220px">
                <h1>{title}</h1>
                {(!text.is_empty()).then(|| view! { <p class="muted" style="margin:6px 0 0">{text}</p> })}
            </div>
            <div class="row wrap">{children.map(|c| c())}</div>
        </div>
    }
}

/// When the watchdog's last restart shows: "14:05" today, else a short date.
pub fn short_time(at: &str) -> String {
    let d = js_sys::Date::new(&at.into());
    if d.get_time().is_nan() {
        return at.to_string();
    }
    let today = js_sys::Date::new_0();
    let same_day = d.to_date_string() == today.to_date_string();
    let opts = js_sys::Object::new();
    if same_day {
        let _ = js_sys::Reflect::set(&opts, &"hour".into(), &"2-digit".into());
        let _ = js_sys::Reflect::set(&opts, &"minute".into(), &"2-digit".into());
        d.to_locale_time_string_with_options("default", &opts).into()
    } else {
        let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"short".into());
        let _ = js_sys::Reflect::set(&opts, &"timeStyle".into(), &"short".into());
        d.to_locale_string("default", &opts).into()
    }
}

/// Whether `at` (an RFC 3339 time) is less than a day ago.
pub fn within_a_day(at: &str) -> bool {
    let t = js_sys::Date::parse(at);
    !t.is_nan() && js_sys::Date::now() - t < 86_400_000.0
}

/// The sidebar's Caddy line ("Caddy 2.10.2 · connected" or "Caddy unreachable") and the watchdog's last restart
/// for a day.
#[component]
pub fn CaddyStatus() -> impl IntoView {
    let status = app::status();
    let line = {
        let status = status.clone();
        move || {
            let st = status.get();
            if st.is_null() {
                view! { <span class="dot"></span><span class="faint grow">"Checking Caddy…"</span> }.into_any()
            } else if b(&st, "caddy_reachable") {
                let v = format!("Caddy {}", s(&st, "caddy_version"));
                view! { <span class="dot ok"></span><span class="grow ellipsis">{v}</span><span class="faint small">"connected"</span> }
                    .into_any()
            } else {
                let why = s(&st, "caddy_error");
                view! { <span class="dot err"></span><span class="grow" title=why>"Caddy unreachable"</span> }.into_any()
            }
        }
    };
    let watchdog = move || {
        let st = status.get();
        let w = st.get("watchdog").cloned().unwrap_or(Value::Null);
        let at = s(&w, "at");
        (!at.is_empty() && within_a_day(&at)).then(|| {
            let ok = b(&w, "ok");
            let text = if ok { "Caddy restarted by watchdog" } else { "Watchdog could not restart Caddy" };
            let title = format!("{}\n{}", s(&w, "reason"), s(&w, "outcome"));
            view! {
                <a class="watchdog small" class:err=!ok href="#/plugins" title=title>{format!("{text} at {}", short_time(&at))}</a>
            }
        })
    };
    view! {
        <div class="caddy-status">
            <div class="row small">{line}</div>
            {watchdog}
        </div>
    }
}

/// The bar at the bottom while there are unapplied changes (not on the pages that apply them).
#[component]
pub fn PendingBar(route: ReadSignal<Route>) -> impl IntoView {
    let status = app::status();
    move || {
        let st = status.get();
        let r = route.get();
        let editing = r.page() == "sites" && r.parts().len() > 1;
        let show = b(&st, "pending_changes") && app::can_edit() && r.page() != "changes" && !editing;
        show.then(|| {
            view! {
                <a class="pending-bar" href="#/changes">
                    <span class="dot warn"></span>
                    <span class="pending-text">"You have unapplied changes"</span>
                    <span class="go">"Review & apply" <span inner_html=icon("arrow-right")></span></span>
                </a>
            }
        })
    }
}

/// A page that is still in the classic panel: what it is, and a link to it.
#[component]
pub fn Classic(#[prop(into)] title: String, #[prop(into)] path: String) -> impl IntoView {
    let href = format!("/#/{path}");
    view! {
        <PageHead title=title.clone()/>
        <div class="card mt">
            <p style="margin-top:0">{format!("{title} hasn't moved to the new look yet.")}</p>
            <a class="btn primary" href=href inner_html=format!("{}Open it in the classic panel", icon("external"))></a>
        </div>
    }
}
