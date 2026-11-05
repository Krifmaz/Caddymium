//! The panel's pages, one module each, and which one the address shows.

pub mod account;
pub mod audit;
pub mod ddns;
pub mod deploy;
pub mod domains;
pub mod import;
pub mod kit;
pub mod login;
pub mod notifications;
pub mod plugins;
pub mod security;
pub mod settings;
pub mod site_editor;
pub mod sites;
pub mod snippets;
pub mod users;
pub mod welcome;

use crate::app::Route;
use leptos::prelude::*;

/// The page for `route`.
pub fn page(route: &Route) -> AnyView {
    if welcome::due() && route.page() != "import" {
        return view! { <welcome::Welcome/> }.into_any();
    }
    match route.page() {
        "account" => view! { <account::Account/> }.into_any(),
        "domains" => view! { <domains::Domains/> }.into_any(),
        "ddns" => view! { <ddns::Ddns/> }.into_any(),
        "plugins" => view! { <plugins::Plugins/> }.into_any(),
        "settings" => view! { <settings::Settings/> }.into_any(),
        "import" => view! { <import::Import/> }.into_any(),
        "welcome" => view! { <welcome::Welcome/> }.into_any(),
        "security" => view! { <security::Security/> }.into_any(),
        "sites" => match route.parts().get(1) {
            Some(id) => view! { <site_editor::SiteEditor id=id.to_string()/> }.into_any(),
            None => view! { <sites::Sites/> }.into_any(),
        },
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
        _ => view! {
            <div class="empty"><h2>"Page not found"</h2><p><a class="btn" href="#/sites">"Go to sites"</a></p></div>
        }
        .into_any(),
    }
}
