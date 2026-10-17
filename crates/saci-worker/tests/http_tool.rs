//! Behavior of the HTTP tool against controlled endpoints.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use saci_core::evidence::HttpEvidence;
use saci_core::spec::{EnvironmentSpec, LlmSettings, SecretRef};
use saci_worker::credentials::Credentials;
use saci_worker::http::{HttpRequest, HttpTool};
use tokio_util::sync::CancellationToken;

const SECRET: &str = "target-secret-value";

async fn echo(headers: HeaderMap, body: String) -> String {
    let auth = headers
        .get("authorization")
        .map(|v| v.to_str().unwrap().to_owned());
    format!("auth={} body={body}", auth.unwrap_or_default())
}

async fn slow() -> &'static str {
    tokio::time::sleep(Duration::from_secs(10)).await;
    "late"
}

fn app() -> Router {
    Router::new()
        .route(
            "/ok",
            get(|| async { ([("set-cookie", "session=abc")], "{\"ok\":true}") }),
        )
        .route(
            "/missing",
            get(|| async { (StatusCode::NOT_FOUND, "nope") }),
        )
        .route("/echo", post(echo))
        .route("/slow", get(slow))
        .route("/big", get(|| async { "x".repeat(10_000) }))
        .route(
            "/redirect",
            get(|| async { Redirect::temporary("http://elsewhere.invalid/") }),
        )
        .route(
            "/leak",
            get(|| async { format!("token is {SECRET}").into_response() }),
        )
}

async fn serve() -> String {
    let app = app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}")
}

fn tool(base: &str, output_limit: u64) -> HttpTool {
    let environment = EnvironmentSpec {
        name: "test".into(),
        parameters: BTreeMap::from([("BASE".into(), base.to_owned())]),
        secrets: BTreeMap::from([("TOKEN".into(), SecretRef::Env { env: "X".into() })]),
        ..EnvironmentSpec::default()
    };
    let llm = LlmSettings {
        base_url: "http://model/v1".into(),
        model: "m".into(),
        api_key_env: None,
        api_key_secret: None,
    };
    let process = [("SACI_SECRET_TOKEN".to_owned(), SECRET.to_owned())];
    let credentials = Credentials::project(&environment, &llm, process).unwrap();
    HttpTool::new(credentials, output_limit).unwrap()
}

fn request(method: &str, url: &str) -> HttpRequest {
    HttpRequest {
        method: method.into(),
        url: url.into(),
        headers: BTreeMap::new(),
        body: None,
    }
}

async fn send(tool: &HttpTool, request: &HttpRequest, deadline: Duration) -> HttpEvidence {
    let cancel = CancellationToken::new();
    tool.send(request, deadline, &cancel)
        .await
        .unwrap()
        .evidence
}

const LONG: Duration = Duration::from_secs(5);

#[tokio::test]
async fn responses_of_any_status_are_evidence() {
    let base = serve().await;
    let tool = tool(&base, 1024);
    let ok = send(&tool, &request("get", "${BASE}/ok"), LONG).await;
    assert_eq!(ok.method, "GET");
    assert_eq!(ok.url, format!("{base}/ok"));
    let response = ok.response.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body.text, "{\"ok\":true}");
    assert_eq!(response.headers["set-cookie"], "[REDACTED]");
    let missing = send(&tool, &request("GET", &format!("{base}/missing")), LONG).await;
    assert_eq!(missing.response.unwrap().status, 404);
    assert!(missing.transport_error.is_none());
}

#[tokio::test]
async fn secrets_are_expanded_for_the_target_and_redacted_in_evidence() {
    let base = serve().await;
    let tool = tool(&base, 1024);
    let mut echo = request("POST", "${BASE}/echo");
    echo.headers
        .insert("Authorization".into(), "Bearer ${TOKEN}".into());
    echo.body = Some("token=${TOKEN}".into());
    let evidence = send(&tool, &echo, LONG).await;
    let text = serde_json::to_string(&evidence).unwrap();
    assert!(!text.contains(SECRET), "{text}");
    let body = evidence.response.unwrap().body.text;
    assert_eq!(
        body,
        "auth=Bearer [REDACTED:TOKEN] body=token=[REDACTED:TOKEN]"
    );
    assert_eq!(evidence.request_headers["Authorization"], "[REDACTED]");
    assert_eq!(
        evidence.request_body.unwrap().text,
        "token=[REDACTED:TOKEN]"
    );
    let leak = send(&tool, &request("GET", "${BASE}/leak"), LONG).await;
    assert_eq!(
        leak.response.unwrap().body.text,
        "token is [REDACTED:TOKEN]"
    );
}

#[tokio::test]
async fn bodies_are_bounded_with_visible_truncation() {
    let base = serve().await;
    let evidence = send(&tool(&base, 100), &request("GET", "${BASE}/big"), LONG).await;
    let body = evidence.response.unwrap().body;
    assert!(body.truncated);
    assert_eq!(body.text.len(), 100);
    assert_eq!(body.total_bytes, 10_000);
}

#[tokio::test]
async fn redirects_are_returned_not_followed() {
    let base = serve().await;
    let evidence = send(&tool(&base, 100), &request("GET", "${BASE}/redirect"), LONG).await;
    let response = evidence.response.unwrap();
    assert_eq!(response.status, 307);
    assert_eq!(response.headers["location"], "http://elsewhere.invalid/");
}

#[tokio::test]
async fn transport_failures_are_distinct_from_responses() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let tool = tool("unused", 100);
    let refused = send(&tool, &request("GET", &format!("http://{address}/")), LONG).await;
    assert!(refused.response.is_none());
    let error = refused.transport_error.unwrap();
    assert!(error.starts_with("connection failed"), "{error}");
}

#[tokio::test]
async fn deadlines_and_cancellation_stop_requests() {
    let base = serve().await;
    let tool = tool(&base, 100);
    let started = Instant::now();
    let slow = request("GET", "${BASE}/slow");
    let timed_out = send(&tool, &slow, Duration::from_millis(300)).await;
    assert!(started.elapsed() < Duration::from_secs(3));
    let error = timed_out.transport_error.unwrap();
    assert!(error.starts_with("the request timed out"), "{error}");
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });
    let cancelled = tool.send(&slow, LONG, &cancel).await.unwrap().evidence;
    assert_eq!(
        cancelled.transport_error.as_deref(),
        Some("the request was cancelled")
    );
}

#[tokio::test]
async fn invalid_arguments_are_rejected_before_sending() {
    let tool = tool("http://unused", 100);
    let cancel = CancellationToken::new();
    for (method, url) in [
        ("GE T", "http://h/"),
        ("GET", "not a url"),
        ("GET", "ftp://h/"),
    ] {
        let rejected = tool.send(&request(method, url), LONG, &cancel).await;
        assert!(rejected.is_err(), "{method} {url}");
    }
    let mut bad_header = request("GET", "http://h/");
    bad_header.headers.insert("bad header".into(), "x".into());
    assert!(tool.send(&bad_header, LONG, &cancel).await.is_err());
    let parsed: Result<HttpRequest, _> =
        serde_json::from_str(r#"{"method":"GET","url":"http://h","extra":1}"#);
    assert!(parsed.is_err(), "unknown arguments are rejected");
}

/// A server that sends a status, headers and part of a 100-byte body for
/// `/{SECRET}`, then closes the connection (`close`) or stalls.
async fn partial(close: bool) -> String {
    use tokio::io::AsyncWriteExt as _;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let head = "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 100\r\nretry-after: 7\r\n\r\n";
                let _ = socket
                    .write_all(format!("{head}partial {SECRET}").as_bytes())
                    .await;
                if !close {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
        }
    });
    format!("http://{address}")
}

#[tokio::test]
async fn a_response_whose_body_fails_keeps_its_status_and_headers() {
    for close in [true, false] {
        let base = partial(close).await;
        let tool = tool(&base, 1024);
        let evidence = send(
            &tool,
            &request("GET", "${BASE}/"),
            Duration::from_millis(500),
        )
        .await;
        assert!(evidence.transport_error.is_none(), "{evidence:?}");
        let response = evidence.response.unwrap();
        assert_eq!(response.status, 503);
        assert_eq!(response.headers["retry-after"], "7");
        assert_eq!(response.body.text, "partial [REDACTED:TOKEN]");
        assert_eq!(response.body.total_bytes, 100);
        let error = response.body_error.unwrap();
        assert!(
            error.starts_with("reading the response body failed"),
            "{error}"
        );
        if !close {
            assert!(error.contains("timed out"), "{error}");
        }
    }
}

#[tokio::test]
async fn cancelling_while_the_body_is_read_keeps_the_response() {
    let tool = tool(&partial(false).await, 1024);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });
    let get = request("GET", "${BASE}/");
    let evidence = tool.send(&get, LONG, &cancel).await.unwrap().evidence;
    let response = evidence.response.unwrap();
    assert_eq!(response.status, 503);
    assert_eq!(
        response.body_error.as_deref(),
        Some("the request was cancelled while the body was read")
    );
}
