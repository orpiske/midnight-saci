//! The web console under `/ui/`: deep links, caching, security headers,
//! and API routes that stay JSON. Tests of the built client's files run
//! when the backend was built with `SACI_WEB_DIST` (CI's web job).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use common::{TOKEN, TestBackend};
use saci_backend::api::ClientAuth;
use saci_backend::console;
use tower::ServiceExt;

struct Page {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
}

async fn get(api: &TestBackend, uri: &str) -> Page {
    let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let response = api.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Page {
        status,
        headers,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn header(page: &Page, name: header::HeaderName) -> &str {
    page.headers
        .get(name)
        .map_or("", |value| value.to_str().unwrap())
}

#[tokio::test]
async fn the_root_and_bare_base_redirect_to_the_console() {
    let api = TestBackend::start(ClientAuth::Open).await;
    for uri in ["/", "/ui"] {
        let page = get(&api, uri).await;
        assert!(page.status.is_redirection(), "{uri}: {}", page.status);
        assert_eq!(header(&page, header::LOCATION), "/ui/");
    }
}

#[tokio::test]
async fn deep_links_reload_to_the_client_without_a_token() {
    let api = TestBackend::start(ClientAuth::Token(TOKEN.into())).await;
    for uri in [
        "/ui/",
        "/ui/runs",
        "/ui/runs/run-3f1c9a2b7d4e4f0a9c1b2d3e4f5a6b7c?tab=activity",
        "/ui/actions?state=unknown",
        "/ui/connection",
    ] {
        let page = get(&api, uri).await;
        assert_eq!(page.status, StatusCode::OK, "{uri}");
        assert!(header(&page, header::CONTENT_TYPE).starts_with("text/html"));
        assert_eq!(header(&page, header::CACHE_CONTROL), "no-cache");
        assert!(
            page.text.contains("<!doctype html>"),
            "{uri}: {}",
            page.text
        );
    }
}

#[tokio::test]
async fn console_pages_carry_security_headers() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let page = get(&api, "/ui/runs").await;
    let csp = header(&page, header::CONTENT_SECURITY_POLICY);
    for directive in [
        "default-src 'self'",
        "script-src 'self'",
        "frame-ancestors 'none'",
    ] {
        assert!(csp.contains(directive), "{csp}");
    }
    assert!(!csp.contains("unsafe-inline") && !csp.contains("unsafe-eval"));
    assert_eq!(header(&page, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert_eq!(header(&page, header::REFERRER_POLICY), "no-referrer");
}

#[tokio::test]
async fn missing_files_are_not_answered_with_the_client() {
    let api = TestBackend::start(ClientAuth::Open).await;
    for uri in ["/ui/assets/missing-0000.js", "/ui/favicon.ico"] {
        let page = get(&api, uri).await;
        assert_eq!(page.status, StatusCode::NOT_FOUND, "{uri}");
        assert!(!page.text.contains("<html"));
    }
}

#[tokio::test]
async fn api_routes_keep_answering_json() {
    let api = TestBackend::start(ClientAuth::Token(TOKEN.into())).await;
    let unknown = api.get("/api/v1/ui/runs").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body["error"]["code"], "not_found");
    let guarded = api.get("/api/v1/runs").await;
    assert_eq!(guarded.status, StatusCode::UNAUTHORIZED);
    assert_eq!(guarded.body["error"]["code"], "unauthorized");
}

/// The `src` or `href` values in `html` that point into `/ui/assets/`.
fn asset_links(html: &str) -> Vec<&str> {
    html.split('"')
        .filter(|part| part.starts_with("/ui/assets/"))
        .collect()
}

#[tokio::test]
async fn the_built_client_serves_its_hashed_assets() {
    if !console::embedded() {
        eprintln!("skipped: the backend was built without SACI_WEB_DIST");
        return;
    }
    let api = TestBackend::start(ClientAuth::Open).await;
    let index = get(&api, "/ui/runs/run-x").await;
    assert!(index.text.contains("id=\"root\""), "{}", index.text);
    let links = asset_links(&index.text);
    assert!(links.iter().any(|link| link.ends_with(".js")), "{links:?}");
    for link in links {
        let asset = get(&api, link).await;
        assert_eq!(asset.status, StatusCode::OK, "{link}");
        assert_eq!(
            header(&asset, header::CACHE_CONTROL),
            "public, max-age=31536000, immutable"
        );
        let kind = header(&asset, header::CONTENT_TYPE);
        assert!(
            kind.starts_with("text/javascript") || kind.starts_with("text/css"),
            "{kind}"
        );
    }
    let licenses = get(&api, "/ui/licenses.txt").await;
    assert!(licenses.text.contains("SIL Open Font License"));
}
