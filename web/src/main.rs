//! Caddymium's web app on the Krifmaz kit: the kit's shell (sidebar, notice slot, view), its Encore theme and
//! every kit feature, with the panel's pages in it.
//!
//! | Where | What |
//! | --- | --- |
//! | everywhere | toasts with Copy, the details dialog, the server's notices with Check again, the offline and connection banners, crash logs, self-update after a new server build |
//! | after signing in | the What's new popup, once per release |
//! | `#/<page>` | the panel's pages ([`pages`]) |
//!
//! Until every page is here, the app is served at `/next/` next to the classic UI at `/`; a page that is not here
//! yet links to its classic version.

mod app;
mod form;
mod icons;
mod pages;
mod ui;

use app::{me, navigate, Route};
use kit_leptos::api::{self, HttpBackend};
use kit_leptos::{crash, details, dropzone, notice, offline, rt, toasts, update, whats_new};
use leptos::prelude::*;
use serde_json::Value;
use std::rc::Rc;
use wasm_bindgen::JsCast;

/// The package name: trunk names the script `caddymium-web-<hash>.js`, whose hash the server's `/api/health`
/// names as the web build.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The app's id: the `X-Requested-With` value, the `x-caddymium` mark on the server's answers and the prefix of
/// the app's events (`caddymium:server-down`).
pub const ID: &str = "caddymium";

/// The name people see.
pub const APP: &str = "Caddymium";

/// This page's web build: the hash in its script's name.
fn this_build() -> Option<String> {
    let head = rt::document()?.head()?.inner_html();
    kit_rules::update::build_in(&head, NAME)
}

/// The connection banner: "Not connected" while offline, "The Caddymium server isn't answering" while it is
/// down (a restart or an update).
fn watch_connection() {
    use kit_rules::offline::mode::{server_down, NOT_CONNECTED};
    let down = server_down(APP);
    rt::on_window("offline", |_| offline::conn_banner(true, NOT_CONNECTED));
    rt::on_window("online", |_| offline::conn_banner(false, ""));
    rt::on_window(&format!("{ID}:server-down"), move |_| offline::conn_banner(true, &down));
    rt::on_window(&format!("{ID}:server-up"), |_| offline::conn_banner(false, ""));
}

/// The kit's configuration: the app's id and name, its script and where What's new lives.
fn kit_config() -> kit_leptos::config::AppConfig {
    let mut cfg = kit_leptos::config::AppConfig::new(kit_rules::app::App::new(ID, APP));
    cfg.script = NAME.to_string();
    cfg.logo = Some("favicon.svg".into());
    cfg.routes.changelog = "/whats-new".into();
    // the panel's /api/settings is its whole settings document; the schema pages live beside it
    cfg.paths.settings = "/settings/form".into();
    cfg.paths.settings_schema = "/settings/form/schema".into();
    cfg.paths.settings_secret = "/settings/form/secret/{key}".into();
    cfg
}

/// Starts the kit's parts that live outside the page, then draws the sign-in page or the shell.
fn main() {
    std::panic::set_hook(Box::new(|p| web_sys::console::error_1(&p.to_string().into())));
    app::apply_theme();
    kit_leptos::config::set(kit_config());
    api::set_backend(HttpBackend::new(ID).with_headers(|method, _| {
        if method == "GET" {
            Vec::new()
        } else {
            vec![("X-CSRF-Token".into(), app::csrf())]
        }
    }));
    toasts::mount();
    crash::install();
    details::init();
    dropzone::init();
    watch_connection();
    let mut cfg = update::UpdateConfig::new(APP);
    cfg.build = this_build();
    update::start(cfg);
    // a call that finds the session gone shows the sign-in page again
    rt::on_window(&format!("{ID}:unauthorized"), |_| {
        if me().get_untracked().get("user").is_some_and(|u| !u.is_null()) {
            rt::spawn(boot());
        }
    });
    rt::spawn(boot());
}

/// Reads who is signed in and draws the page for it.
async fn boot() {
    let m = api::get("/me").await.unwrap_or_else(|e| serde_json::json!({ "error": e.message }));
    app::set_me(m.clone());
    let Some(root) = rt::by_id("app").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) else { return };
    root.set_inner_html("");
    let signed_in = m.get("user").is_some_and(|u| !u.is_null()) && !app::b(&m, "pending_2fa");
    if !signed_in {
        leptos::mount::mount_to(root, pages::login::SignIn).forget();
        return;
    }
    leptos::mount::mount_to(root, Shell).forget();
    // the notice slot is part of the shell: start once it is drawn
    notice::start(notice::NoticeSource::default(), app::is_admin());
    notice::mount();
    whats_new::start(Rc::new(navigate));
    app::start_status();
}

/// One page of the sidebar.
#[derive(Clone, Copy)]
pub struct NavItem {
    /// `#/<path>`
    pub path: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    /// admins only
    pub admin: bool,
}

/// The sidebar's groups.
pub const NAV: &[(&str, &[NavItem])] = &[
    (
        "Configure",
        &[
            NavItem { path: "sites", label: "Sites", icon: "globe", admin: false },
            NavItem { path: "domains", label: "Domains & DNS", icon: "shield-check", admin: true },
            NavItem { path: "ddns", label: "Dynamic DNS", icon: "signal", admin: true },
            NavItem { path: "security", label: "Security", icon: "shield", admin: true },
            NavItem { path: "snippets", label: "Snippets & raw", icon: "braces", admin: false },
            NavItem { path: "import", label: "Import", icon: "file-input", admin: true },
        ],
    ),
    (
        "Deploy",
        &[
            NavItem { path: "changes", label: "Review & apply", icon: "rocket", admin: false },
            NavItem { path: "history", label: "History", icon: "history", admin: false },
            NavItem { path: "caddyfile", label: "Caddyfile", icon: "file-code", admin: false },
        ],
    ),
    (
        "Manage",
        &[
            NavItem { path: "settings", label: "Settings", icon: "settings", admin: false },
            NavItem { path: "plugins", label: "Plugins", icon: "puzzle", admin: true },
            NavItem { path: "users", label: "Users & API keys", icon: "users", admin: true },
            NavItem { path: "notifications", label: "Notifications", icon: "bell", admin: true },
            NavItem { path: "audit", label: "Audit log", icon: "scroll", admin: true },
            NavItem { path: "diagnostics", label: "Diagnostics", icon: "activity", admin: true },
        ],
    ),
];

/// One link of the sidebar, with its dot (unapplied changes, a Caddy update).
#[component]
fn NavLink(item: NavItem, route: ReadSignal<Route>, menu: RwSignal<bool>) -> impl IntoView {
    let active = move || route.get().page() == item.path;
    let status = app::status();
    let dot = move || {
        let st = status.get();
        let (on, title) = match item.path {
            "changes" => (app::b(&st, "pending_changes"), "Unapplied changes".to_string()),
            "plugins" => (app::b(&st, "update_available"), format!("Caddy {} is available", app::s(&st, "latest_caddy"))),
            _ => (false, String::new()),
        };
        on.then(|| view! { <span class="dot warn" title=title></span> })
    };
    view! {
        <a href=format!("#/{}", item.path) class:active=active on:click=move |_| menu.set(false)>
            <span inner_html=icons::icon(item.icon)></span>
            <span class="grow">{item.label}</span>
            {dot}
        </a>
    }
}

/// The question Sign out asks first.
pub fn sign_out_question() -> kit_rules::dialogs::Confirm {
    kit_rules::dialogs::Confirm::new("Sign out?")
        .text("This browser forgets its sign-in. You'll need your password to sign in again.")
        .ok("Sign out")
        .danger(true)
}

/// The shell: the sidebar with the panel's pages, the notice slot and the page.
#[component]
fn Shell() -> impl IntoView {
    let (route, set_route) = signal(Route::current());
    rt::on_window("hashchange", move |_| set_route.set(Route::current()));
    let menu = RwSignal::new(false);
    let m = me().get_untracked();
    let user = m.get("user").cloned().unwrap_or(Value::Null);
    let username = app::s(&user, "username");
    let role = app::s(&user, "role");
    let initial = username.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let admin = app::is_admin();
    let sign_out = move |_| {
        rt::spawn(async {
            if kit_leptos::modal::confirm(sign_out_question()).await {
                let _ = api::post_empty("/logout").await;
                navigate("/");
                rt::reload();
            }
        })
    };
    let groups = NAV
        .iter()
        .map(|(label, items)| {
            let links =
                items.iter().filter(|i| admin || !i.admin).map(|i| view! { <NavLink item=*i route=route menu=menu/> }).collect_view();
            view! { <div class="nav-label">{*label}</div>{links} }
        })
        .collect_view();
    let page = move || pages::page(&route.get());
    view! {
        <div class="shell">
            <aside class="sidebar" id="sidebar" class:open=move || menu.get()>
                <a class="brand" href="#/sites"><img src="favicon.svg" alt=""/>{APP}</a>
                <nav class="nav">{groups}</nav>
                <ui::CaddyStatus/>
                <div class="sidebar-foot">
                    <a class="row grow me-link" href="#/account" on:click=move |_| menu.set(false)>
                        <span class="avatar">{initial}</span>
                        <span class="grow" style="min-width:0">
                            <div class="ellipsis" style="font-weight:650">{username}</div>
                            <div class="small faint">{role}</div>
                        </span>
                    </a>
                    <a class="btn ghost icon" href="#/bug-reports" title="Bug reports" aria-label="Bug reports" inner_html=icons::icon("bug")></a>
                    <button class="btn ghost icon" id="logout-btn" title="Sign out" aria-label="Sign out" on:click=sign_out inner_html=icons::icon("log-out")></button>
                </div>
            </aside>
            <div class="scrim" class:hidden=move || !menu.get() on:click=move |_| menu.set(false)></div>
            <main class="main">
                <header class="mobile-top">
                    <button class="btn ghost icon" aria-label="Menu" on:click=move |_| menu.set(true) inner_html=icons::icon("menu")></button>
                    <img src="favicon.svg" alt="" width="24" height="24"/>
                    <strong>{APP}</strong>
                </header>
                <div id="server-notice"></div>
                <div id="view">{page}</div>
            </main>
        </div>
        <ui::PendingBar route=route/>
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sign_out_asks_first() {
        let q = super::sign_out_question();
        assert_eq!((q.title.as_str(), q.ok.as_str(), q.danger), ("Sign out?", "Sign out", true));
    }

    #[test]
    fn every_page_is_in_one_group() {
        let mut seen = std::collections::HashSet::new();
        for (_, items) in super::NAV {
            for i in *items {
                assert!(seen.insert(i.path), "{} twice", i.path);
                assert!(!super::icons::icon(i.icon).is_empty(), "icon {}", i.icon);
            }
        }
    }
}
