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

// ---------------------------------------------------------------- times, files

/// "just now", "5 min ago", "3 h ago", else the date.
pub fn ago(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    let t = js_sys::Date::parse(ts);
    if t.is_nan() {
        return ts.to_string();
    }
    let s = (js_sys::Date::now() - t) / 1000.0;
    if s < 60.0 {
        "just now".into()
    } else if s < 3600.0 {
        format!("{} min ago", (s / 60.0) as i64)
    } else if s < 86400.0 {
        format!("{} h ago", (s / 3600.0) as i64)
    } else {
        let d = js_sys::Date::new(&ts.into());
        let opts = js_sys::Object::new();
        for (k, v) in [("month", "short"), ("day", "numeric"), ("year", "numeric")] {
            let _ = js_sys::Reflect::set(&opts, &k.into(), &v.into());
        }
        d.to_locale_date_string("default", &opts).into()
    }
}

/// The local date and time of `ts`.
pub fn when(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    js_sys::Date::new(&ts.into()).to_locale_string("default", &js_sys::Object::new()).into()
}

/// Saves `text` as the file `name` (the browser's download).
pub fn download(name: &str, text: &str) {
    use wasm_bindgen::JsCast;
    let parts = js_sys::Array::of1(&text.into());
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/plain");
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
    if let Some(a) =
        kit_leptos::rt::document().and_then(|d| d.create_element("a").ok()).and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok())
    {
        a.set_href(&url);
        a.set_download(name);
        a.click();
    }
    kit_leptos::rt::after(1000, move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
}

/// Asks "are you sure?" with the kit's dialog.
pub async fn sure(title: &str, text: &str, ok: &str, danger: bool) -> bool {
    kit_leptos::modal::confirm_dialog(title, text, ok, danger).await
}

/// Shows a failed call as an error toast (it stays, with Copy).
pub fn toast_err(e: kit_leptos::api::ApiError) {
    kit_leptos::toasts::toast(&e.message, "err");
}

// ---------------------------------------------------------------- alerts, diffs

/// A coloured box: `ok`, `info`, `warn` or `err`.
#[component]
pub fn Alert(#[prop(into)] kind: String, children: Children) -> impl IntoView {
    let ic = match kind.as_str() {
        "ok" => "check",
        "err" => "x",
        _ => "alert",
    };
    view! {
        <div class=format!("alert {kind}") role=if kind == "err" { "alert" } else { "status" }>
            <span class="alert-ic" inner_html=icon(ic)></span>
            <div class="grow">{children()}</div>
        </div>
    }
}

/// Caddy's adapt/validate messages of a plan that failed, with Copy.
#[component]
pub fn PlanErrors(plan: Value) -> impl IntoView {
    (!plan.is_null() && !b(&plan, "valid")).then(|| {
        let stage = s(&plan, "stage");
        let lines: Vec<String> = app::a(&plan, "errors").iter().map(|e| s(e, "text")).collect();
        let head = format!("Caddy rejected this config at the {stage} step. Nothing was changed.");
        let copy = format!("{head}\n{}", lines.join("\n"));
        view! {
            <div class="alert err" role="alert">
                <span class="alert-ic" inner_html=icon("x")></span>
                <div class="grow">
                    <strong>{head}</strong>
                    <ul class="plan-errors">{lines.into_iter().map(|l| view! { <li class="mono">{l}</li> }).collect_view()}</ul>
                </div>
                <CopyButton text=copy/>
            </div>
        }
    })
}

/// One line of a unified diff.
#[derive(Debug, PartialEq)]
pub enum DiffRow {
    Hunk(String),
    Add(i64, String),
    Del(i64, String),
    Same(i64, i64, String),
}

/// The rows of a unified diff, with old and new line numbers.
pub fn diff_rows(diff: &str) -> Vec<DiffRow> {
    let (mut a, mut b) = (0i64, 0i64);
    let mut out = Vec::new();
    for l in diff.split('\n') {
        if l.starts_with("+++") || l.starts_with("---") || l.is_empty() {
            continue;
        }
        if let Some(rest) = l.strip_prefix("@@ -") {
            let nums: Vec<i64> =
                rest.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect();
            // -a[,n] +b[,m]
            let plus = rest.split_once('+').map(|(_, p)| p).unwrap_or("");
            a = nums.first().copied().unwrap_or(0);
            b = plus.split(|c: char| !c.is_ascii_digit()).next().and_then(|x| x.parse().ok()).unwrap_or(0);
            out.push(DiffRow::Hunk(l.to_string()));
            continue;
        }
        let text = l.get(1..).unwrap_or("").to_string();
        match l.as_bytes()[0] {
            b'+' => {
                out.push(DiffRow::Add(b, text));
                b += 1;
            }
            b'-' => {
                out.push(DiffRow::Del(a, text));
                a += 1;
            }
            _ => {
                out.push(DiffRow::Same(a, b, text));
                a += 1;
                b += 1;
            }
        }
    }
    out
}

/// A unified diff with line numbers and the changes coloured.
#[component]
pub fn Diff(#[prop(into)] diff: String) -> impl IntoView {
    let rows = diff_rows(&diff);
    if rows.is_empty() {
        return view! { <p class="muted">"No differences."</p> }.into_any();
    }
    let rows = rows
        .into_iter()
        .map(|r| match r {
            DiffRow::Hunk(t) => view! { <tr class="hunk"><td colspan="4">{t}</td></tr> }.into_any(),
            DiffRow::Add(n, t) => {
                view! { <tr class="add"><td class="n"></td><td class="n">{n}</td><td class="sign">"+"</td><td class="code">{t}</td></tr> }
                    .into_any()
            }
            DiffRow::Del(n, t) => {
                view! { <tr class="del"><td class="n">{n}</td><td class="n"></td><td class="sign">"−"</td><td class="code">{t}</td></tr> }
                    .into_any()
            }
            DiffRow::Same(a, b, t) => {
                view! { <tr><td class="n">{a}</td><td class="n">{b}</td><td class="sign"></td><td class="code">{t}</td></tr> }.into_any()
            }
        })
        .collect_view();
    view! { <div class="diff"><table><tbody>{rows}</tbody></table></div> }.into_any()
}

#[cfg(test)]
mod tests {
    use super::{diff_rows, DiffRow};

    #[test]
    fn diffs_number_their_lines() {
        let d = "--- a\n+++ b\n@@ -3,2 +3,3 @@\n same\n-old\n+new\n+more\n";
        let r = diff_rows(d);
        assert_eq!(r[1], DiffRow::Same(3, 3, "same".into()));
        assert_eq!(r[2], DiffRow::Del(4, "old".into()));
        assert_eq!(r[3], DiffRow::Add(4, "new".into()));
        assert_eq!(r[4], DiffRow::Add(5, "more".into()));
    }
}

// ---------------------------------------------------------------- dialogs

/// Closes the dialog it was handed to (set once the dialog is open).
#[derive(Clone, Default)]
pub struct Closer(std::rc::Rc<std::cell::RefCell<Option<kit_leptos::modal::Modal>>>);

impl Closer {
    /// Closes the dialog.
    pub fn close(&self) {
        if let Some(m) = self.0.borrow_mut().take() {
            m.close();
        }
    }
}

/// Opens a dialog (the kit's modal) with `body` and `footer`, both handed a [`Closer`].
pub fn dialog<B, F>(title: &str, wide: bool, body: impl FnOnce(Closer) -> B + 'static, footer: impl FnOnce(Closer) -> F + 'static) -> Closer
where
    B: IntoView + 'static,
    F: IntoView + 'static,
{
    let c = Closer::default();
    let (c1, c2) = (c.clone(), c.clone());
    let m = kit_leptos::modal::open(
        kit_leptos::modal::ModalOpts::with_view(title, move || body(c1)).footer_view(move || footer(c2)).wide(wide),
    );
    *c.0.borrow_mut() = Some(m);
    c
}

/// A signal holding a call's answer: None while it loads.
pub type Loaded = RwSignal<Option<Result<Value, String>>>;

/// Loads `path` into a new [`Loaded`] signal; [`reload`] loads it again.
pub fn load(path: &str) -> Loaded {
    let sig: Loaded = RwSignal::new(None);
    reload(sig, path);
    sig
}

/// Loads `path` into `sig` again (the old answer stays until the new one is in).
pub fn reload(sig: Loaded, path: &str) {
    let path = path.to_string();
    kit_leptos::rt::spawn(async move {
        sig.set(Some(kit_leptos::api::get(&path).await.map_err(|e| e.message)));
    });
}

/// Draws a [`Loaded`] signal: the spinner, the error, or `f` of the answer.
pub fn show<V: IntoView + 'static>(sig: Loaded, f: impl Fn(Value) -> V + 'static) -> impl Fn() -> AnyView {
    move || match sig.get() {
        None => view! { <Loading/> }.into_any(),
        Some(Err(e)) => view! { <ErrorBox message=e/> }.into_any(),
        Some(Ok(v)) => f(v).into_any(),
    }
}
