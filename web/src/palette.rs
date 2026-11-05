//! The command palette (Ctrl+K / ⌘K, or the sidebar's search button): the kit's [`kit_leptos::palette`] with the
//! panel's actions, its pages and, loaded each time it opens, its sites.

use crate::app::{self, a, s};
use crate::NAV;
use kit_leptos::api;
use kit_leptos::palette::{self, Command, Palette, ACTIONS, GO_TO};

/// The group the sites are listed under.
pub const SITES: &str = "Sites";

/// The fixed commands for someone who can (`edit`) or can't change things, and is or isn't an admin.
pub fn commands(edit: bool, admin: bool) -> Vec<Command> {
    let mut list = Vec::new();
    if edit {
        list.push(Command::new(ACTIONS, "New site", || app::navigate("/sites/new")).icon("plus"));
        list.push(Command::new(ACTIONS, "Review & apply changes", || app::navigate("/changes")));
    }
    for item in NAV.iter().flat_map(|(_, items)| items.iter()).filter(|i| admin || !i.admin) {
        let path = format!("/{}", item.path);
        let cmd = Command::new(GO_TO, item.label, move || app::navigate(&path));
        list.push(if item.path == "settings" { cmd.icon("settings") } else { cmd });
    }
    list.push(Command::new(GO_TO, "Account", || app::navigate("/account")));
    list.push(Command::new(GO_TO, "What's new", || app::navigate("/whats-new")));
    list.push(Command::new(GO_TO, "Bug reports", || app::navigate("/bug-reports")).icon("bug"));
    list
}

/// One site as a command: its name, its hostnames as the hint.
pub fn site(v: &serde_json::Value) -> Command {
    let id = v.get("id").map(|x| x.to_string()).unwrap_or_default();
    let hosts: Vec<String> = a(v, "hosts").iter().filter_map(|h| h.as_str().map(str::to_string)).collect();
    Command::new(SITES, s(v, "name"), move || app::navigate(&format!("/sites/{id}"))).hint(hosts.join(", "))
}

/// Installs the palette for the signed-in person.
pub fn install() {
    let p = Palette::new(commands(app::can_edit(), app::is_admin())).source(|| async {
        api::get("/sites").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().iter().map(site).collect()
    });
    palette::install(p);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn labels(l: &[Command]) -> Vec<String> {
        l.iter().map(|c| c.item.label.clone()).collect()
    }

    #[test]
    fn commands_follow_the_role() {
        let viewer = labels(&commands(false, false));
        assert!(!viewer.contains(&"New site".to_string()));
        assert!(viewer.contains(&"Sites".to_string()));
        assert!(!viewer.contains(&"Users & API keys".to_string()));
        let admin = labels(&commands(true, true));
        assert_eq!(admin[0], "New site");
        assert!(admin.contains(&"Users & API keys".to_string()));
        assert!(admin.contains(&"Bug reports".to_string()));
    }

    #[test]
    fn a_site_shows_its_hosts() {
        let c = site(&json!({"id": 3, "name": "Jellyfin", "hosts": ["tv.example.com", "jf.example.com"]}));
        assert_eq!(c.item.label, "Jellyfin");
        assert_eq!(c.item.hint, "tv.example.com, jf.example.com");
        assert_eq!(c.item.group, SITES);
    }
}
