//! Build script: embeds the release notes (../docs/releases/<version>.md) for What's new.

fn main() {
    let notes = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/releases");
    kit_server_changelog::build::embed(&notes, "changelog_gen.rs");
}
