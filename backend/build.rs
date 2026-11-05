//! Makes sure the directory the web UI is embedded from exists, so the
//! backend compiles (and its tests run) on a fresh checkout without a
//! web build (cd web && trunk build); web.rs then serves a hint instead of the UI. Also makes
//! cargo rebuild the binary when the UI build changes, and embeds the
//! release notes (../docs/releases/<version>.md) for What's new, and turns settings.toml into the Settings
//! pages' table (settings_gen.rs).

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dir = std::path::Path::new(&manifest).join("ui/dist");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        println!("cargo:warning=could not create {}: {e}", dir.display());
    }
    println!("cargo:rerun-if-changed=ui/dist");
    let notes = std::path::Path::new(&manifest).join("../docs/releases");
    kit_server_changelog::build::embed(&notes, "changelog_gen.rs");

    println!("cargo:rerun-if-changed=settings.toml");
    let text = std::fs::read_to_string(std::path::Path::new(&manifest).join("settings.toml"))
        .expect("settings.toml");
    let codegen = kit_server_settings::file::Codegen {
        accessors_on: None,
        methods: &[],
    };
    let code = kit_server_settings::file::generate(&text, &codegen)
        .unwrap_or_else(|e| panic!("settings.toml: {e}"));
    let out =
        std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("settings_gen.rs");
    std::fs::write(out, code).expect("write settings_gen.rs");
}
