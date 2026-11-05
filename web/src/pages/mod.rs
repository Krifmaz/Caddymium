//! The panel's pages, one module each, and which one the address shows.

pub mod account;
pub mod audit;
pub mod ddns;
pub mod deploy;
pub mod domains;
pub mod kit;
pub mod login;
pub mod notifications;
pub mod plugins;
pub mod snippets;
pub mod users;

use crate::app::Route;
use crate::ui::Classic;
use crate::NAV;
use leptos::prelude::*;

/// The page for `route`.
pub fn page(route: &Route) -> AnyView {
    match route.page() {
        "account" => view! { <account::Account/> }.into_any(),
        "domains" => view! { <domains::Domains/> }.into_any(),
        "ddns" => view! { <ddns::Ddns/> }.into_any(),
        "plugins" => view! { <plugins::Plugins/> }.into_any(),
        "audit" => view! { <audit::Audit/> }.into_any(),
        "changes" => view! { <deploy::Changes/> }.into_any(),
        "history" => view! { <deploy::History/> }.into_any(),
        "caddyfile" => view! { <deploy::Caddyfile/> }.into_any(),
        "users" => view! { <users::Users/> }.into_any(),
        "notifications" => view! { <notifications::Notifications/> }.into_any(),
        "snippets" => view! { <snippets::Snippets/> }.into_any(),
        "bug-reports" => view! { <kit::BugReports/> }.into_any(),
        "diagnostics" => view! { <kit::Diagnostics/> }.into_any(),
        "whats-new" => view! { <kit::WhatsNew/> }.into_any(),
        p => {
            let item = NAV.iter().flat_map(|(_, items)| items.iter()).find(|i| i.path == p);
            match item {
                Some(i) => view! { <Classic title=i.label path=route.path.trim_start_matches('/').to_string()/> }.into_any(),
                None => view! {
                    <div class="empty"><h2>"Page not found"</h2><p><a class="btn" href="#/sites">"Go to sites"</a></p></div>
                }
                .into_any(),
            }
        }
    }
}
