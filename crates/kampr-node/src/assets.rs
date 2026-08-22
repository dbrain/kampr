use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

/// The Compose Multiplatform wasm bundle, staged here by the build before `cargo build`. An empty
/// directory is a valid state — the node is useful before the client ships, so it serves a
/// placeholder rather than a 404.
#[derive(Embed)]
#[folder = "dist/"]
struct Bundle;

pub fn has_bundle() -> bool {
    Bundle::get("index.html").is_some()
}

pub fn serve(path: &str) -> Response {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Bundle::get(path) {
        Some(file) => file_response(path, file.data.into_owned()),
        // Anything the bundle does not have is the SPA's own route, so it gets the shell — the
        // single exception being a request that is obviously for an asset.
        None if !path.contains('.') => match Bundle::get("index.html") {
            Some(file) => file_response("index.html", file.data.into_owned()),
            None => placeholder(),
        },
        None if path == "index.html" => placeholder(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Only a name that carries its own content hash may be cached forever, and the rule is stated
/// this way round on purpose. An allow-list of *mutable* names has to be right about every file
/// the bundler emits; miss one and a browser that has visited once keeps that file for a year,
/// across every node upgrade. `kamprWeb.js` is the file that made this concrete — it is served
/// under a stable name and it is what names the hashed `.wasm`, so caching it pinned a returning
/// browser to a build the node no longer had on disk.
fn hashed(path: &str) -> bool {
    let stem = path.rsplit('/').next().unwrap_or(path);
    stem.split('.')
        .next()
        .is_some_and(|s| s.len() >= 16 && s.chars().all(|c| c.is_ascii_hexdigit()))
}

fn file_response(path: &str, body: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if hashed(path) {
        "public, max-age=31536000, immutable"
    } else {
        "no-store"
    };
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(body))
        .expect("static response is well formed")
}

fn placeholder() -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(PLACEHOLDER))
        .expect("placeholder response is well formed")
}

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Kampr</title>
<style>
:root { color-scheme: light dark; }
body { margin:0; min-height:100dvh; display:grid; place-items:center;
       font:16px/1.6 ui-sans-serif,system-ui,sans-serif; padding:2rem; }
main { max-width:34rem; }
h1 { font-size:1.5rem; margin:0 0 .5rem; letter-spacing:-.01em; }
p { margin:.5rem 0; opacity:.8; }
code { font-family:ui-monospace,monospace; font-size:.9em;
       background:color-mix(in srgb, currentColor 10%, transparent);
       padding:.15em .4em; border-radius:.3em; }
</style></head><body><main>
<h1>Kampr is running</h1>
<p>The node is up and speaking the wire protocol at <code>/ws</code>. No client bundle has been
built into this binary yet.</p>
<p>Build one with the Gradle wasm target and stage it into <code>crates/kampr-node/dist/</code>
before <code>cargo build</code>, or point an existing client at this node.</p>
<p>Pair a device with <code>kampr setup</code>.</p>
</main></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn cache_of(path: &str) -> String {
        let r = file_response(path, b"x".to_vec());
        r.headers()[header::CACHE_CONTROL].to_str().unwrap().to_string()
    }

    /// The shell names `boot.js`, `kampr.css` and `kamprWeb.js` by stable name, and that last one
    /// names the content-hashed wasm. Serve any of them `immutable` and a browser that has visited
    /// once keeps its old client for a year — across every node upgrade — asking for a `.wasm` the
    /// node no longer has on disk.
    #[test]
    fn only_a_content_hashed_name_may_be_cached_forever() {
        for stable in [
            "index.html",
            "sw.js",
            "manifest.webmanifest",
            "boot.js",
            "kampr.css",
            "kamprWeb.js",
            "offline.html",
        ] {
            assert_eq!(
                cache_of(stable),
                "no-store",
                "{stable} is served under a stable name"
            );
        }
        for hashed in ["6e23e5428398b92da386.wasm", "cd08fc40208bccbb0d73.wasm"] {
            assert!(
                cache_of(hashed).contains("immutable"),
                "{hashed} carries its own hash"
            );
        }
    }

    async fn body_of(response: Response) -> String {
        String::from_utf8(to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn a_node_with_no_bundle_still_serves_a_page() {
        if has_bundle() {
            return;
        }
        let response = serve("/");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_of(response).await.contains("Kampr is running"));
    }

    #[tokio::test]
    async fn an_unknown_asset_is_a_404_rather_than_the_shell() {
        assert_eq!(serve("/nope.js").status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_client_route_falls_through_to_the_shell() {
        assert_eq!(serve("/herd/01J/w3:p2").status(), StatusCode::OK);
    }
}
