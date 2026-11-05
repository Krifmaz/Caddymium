//! The web app (web/, Leptos on the Krifmaz kit), built by trunk into ui/dist
//! and embedded into the binary. Without a build the server still runs and
//! shows a hint instead of the UI.

use axum::body::Body;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "ui/dist"]
struct Ui;

/// The script name's stem of the web app: `caddymium-web-<hash>.js`.
const SCRIPT: &str = "caddymium-web-";

/// The build of the web app (the hash in its script's name), for
/// `/api/health`: an open tab reloads when it changes.
pub fn web_build() -> Option<String> {
    Ui::iter().find_map(|f| kit_server_health::web_build_of(&f, SCRIPT).map(str::to_string))
}

/// Reports whether a UI build is embedded.
pub fn available() -> bool {
    Ui::get("index.html").is_some()
}

/// `caddymium-web-f92ab5b84b305a59_bg.wasm`, `app-6ecd585ddbbbd1c2.css`: a
/// file whose name ends in a hash of its content (trunk's `filehash`), so it
/// never changes under that name.
fn hashed_asset(name: &str) -> bool {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let stem = stem.strip_suffix("_bg").unwrap_or(stem);
    !name.contains('/')
        && stem
            .rsplit_once('-')
            .is_some_and(|(_, h)| h.len() >= 12 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Serves the web app: its files (the hashed ones cached for good),
/// index.html for everything else (its routes live in the URL's hash).
pub async fn spa(uri: Uri) -> Response {
    let p = uri.path().trim_start_matches('/');
    if !p.is_empty() {
        if let Some(f) = Ui::get(p) {
            let mime = mime_guess::from_path(p).first_or_octet_stream();
            let cache = if hashed_asset(p) {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            return Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache)
                .body(Body::from(f.data.into_owned()))
                .unwrap();
        }
    }
    match Ui::get("index.html") {
        Some(f) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(f.data.into_owned()))
            .unwrap(),
        None => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "Caddymium API is running, but the web UI was not built into this binary.\nBuild it with: cd web && trunk build --release\n",
        )
            .into_response(),
    }
}

/// `/next/…`, where the web app lived while it replaced the classic UI: a
/// permanent redirect to `/`. Browsers keep the hash (`/next/#/sites` →
/// `/#/sites`), so old bookmarks land on the same page.
pub async fn old_next() -> Redirect {
    Redirect::permanent("/")
}

#[cfg(test)]
mod tests {
    #[test]
    fn hashed_assets_are_cached_for_good() {
        assert!(super::hashed_asset(
            "caddymium-web-e1f801b77699cd55_bg.wasm"
        ));
        assert!(super::hashed_asset("caddymium-web-34921df7360661b.js"));
        assert!(super::hashed_asset("app-6ecd585ddbbbd1c2.css"));
        assert!(!super::hashed_asset("favicon.svg"));
        assert!(!super::hashed_asset("index.html"));
    }
}
