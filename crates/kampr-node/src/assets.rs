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

pub fn serve(path: &str, if_none_match: Option<&str>) -> Response {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Bundle::get(path) {
        Some(file) => file_response(path, file, if_none_match),
        // Anything the bundle does not have is the SPA's own route, so it gets the shell — the
        // single exception being a request that is obviously for an asset.
        None if !path.contains('.') => match Bundle::get("index.html") {
            Some(file) => file_response("index.html", file, if_none_match),
            None => placeholder(),
        },
        None if path == "index.html" => placeholder(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// The file's own content hash, as an entity tag.
///
/// `rust-embed` computes it at compile time, so this costs nothing per request — which matters
/// because the thing it exists to save is the largest file in the bundle being re-sent on every
/// page load.
fn etag(file: &rust_embed::EmbeddedFile) -> String {
    let hash = file.metadata.sha256_hash();
    let mut tag = String::with_capacity(2 + hash.len() * 2);
    tag.push('"');
    for byte in hash.iter().take(16) {
        tag.push_str(&format!("{byte:02x}"));
    }
    tag.push('"');
    tag
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

/// **`no-store` is stronger than the rule needs, and the fonts are what made that expensive.**
///
/// The rule above is that only a content-hashed name may be cached *forever*, and it is right: a
/// stable name served `immutable` pinned a returning browser to a build the node no longer had
/// (#157). But `no-store` says something else — *never keep this at all* — so every page load
/// re-fetched every unhashed file whole. The four terminal faces went from 342 KB to 1.01 MB each
/// when the emoji were cut in (#417), which turned that into **+2.7 MB on every visit**, not just
/// the first, on the surface the operator reads from a phone.
///
/// `no-cache` with an entity tag is the honest middle: the browser keeps a copy and **asks every
/// time**, and gets 304 and no body when nothing changed. It cannot pin a stale file — that is the
/// whole difference from `immutable` — and it cannot serve one either, because it never answers
/// from its own copy without asking first.
fn file_response(path: &str, file: rust_embed::EmbeddedFile, if_none_match: Option<&str>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let tag = etag(&file);
    if if_none_match.is_some_and(|held| held.split(',').any(|one| one.trim() == tag)) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::CACHE_CONTROL, cache_for(path))
            .header(header::ETAG, &tag)
            .body(Body::empty())
            .expect("a 304 is well formed");
    }
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache_for(path))
        .header(header::ETAG, &tag)
        .body(Body::from(file.data.into_owned()))
        .expect("static response is well formed")
}

fn cache_for(path: &str) -> &'static str {
    if hashed(path) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
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
        cache_for(path).to_string()
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
                "no-cache",
                "{stable} is served under a stable name, so it must be revalidated every time"
            );
        }
        for hashed in ["6e23e5428398b92da386.wasm", "cd08fc40208bccbb0d73.wasm"] {
            assert!(
                cache_of(hashed).contains("immutable"),
                "{hashed} carries its own hash"
            );
        }
    }

    /// **The half `no-store` was paying for.** A stable name must be re-checked on every load —
    /// that is the whole of #157 — but re-checking is a conditional request, not a re-download. The
    /// four terminal faces are 1.01 MB each since the emoji were cut in, and serving them
    /// `no-store` meant +2.7 MB on every visit rather than on the first.
    #[test]
    fn a_stable_name_is_revalidated_rather_than_re_sent() {
        let file = Bundle::get("index.html");
        let Some(file) = file else {
            return; // no bundle staged into this build; `has_bundle` covers that case
        };
        let tag = etag(&file);
        assert!(
            tag.starts_with('"') && tag.ends_with('"'),
            "{tag} is not an entity tag"
        );

        let fresh = serve("index.html", None);
        assert_eq!(fresh.status(), StatusCode::OK);
        assert_eq!(fresh.headers()[header::ETAG].to_str().unwrap(), tag);
        assert_eq!(
            fresh.headers()[header::CACHE_CONTROL].to_str().unwrap(),
            "no-cache"
        );

        let held = serve("index.html", Some(&tag));
        assert_eq!(
            held.status(),
            StatusCode::NOT_MODIFIED,
            "a browser holding the current file was sent the whole thing again",
        );
        assert_eq!(held.headers()[header::ETAG].to_str().unwrap(), tag);

        let stale = serve("index.html", Some("\"0123456789abcdef\""));
        assert_eq!(
            stale.status(),
            StatusCode::OK,
            "a browser holding an old file was told it was still current",
        );
    }

    /// The tag is the file's own content, so two different files never share one and the same file
    /// keeps its across a rebuild that did not change it.
    #[test]
    fn the_entity_tag_is_the_files_own_content() {
        let (Some(a), Some(b)) = (Bundle::get("index.html"), Bundle::get("sw.js")) else {
            return;
        };
        assert_ne!(etag(&a), etag(&b));
        assert_eq!(etag(&a), etag(&Bundle::get("index.html").unwrap()));
    }

    async fn body_of(response: Response) -> String {
        String::from_utf8(to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn a_node_with_no_bundle_still_serves_a_page() {
        if has_bundle() {
            return;
        }
        let response = serve("/", None);
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_of(response).await.contains("Kampr is running"));
    }

    #[tokio::test]
    async fn an_unknown_asset_is_a_404_rather_than_the_shell() {
        assert_eq!(serve("/nope.js", None).status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_client_route_falls_through_to_the_shell() {
        assert_eq!(serve("/herd/01J/w3:p2", None).status(), StatusCode::OK);
    }
}
