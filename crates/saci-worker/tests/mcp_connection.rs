//! MCP connections against the controlled `saci-fake-mcp` server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParams;
use saci_core::spec::{
    EnvironmentSpec, HttpServer, LlmSettings, McpServerSpec, McpTransport, SecretRef, StdioServer,
};
use saci_testkit::fake_mcp;
use saci_worker::credentials::Credentials;
use saci_worker::mcp::{McpServer, connect, connect_all};

const TIMEOUT: Duration = Duration::from_secs(10);

fn credentials() -> Credentials {
    let binary = fake_mcp::binary().unwrap();
    let dir = binary.parent().unwrap().display().to_string();
    let path = format!("{dir}:{}", std::env::var("PATH").unwrap());
    let environment = EnvironmentSpec {
        name: "test".into(),
        secrets: BTreeMap::from([("MCP_TOKEN".into(), SecretRef::Env { env: "X".into() })]),
        ..EnvironmentSpec::default()
    };
    let llm = LlmSettings {
        base_url: "http://model/v1".into(),
        model: "m".into(),
        api_key_env: Some("SACI_LLM_API_KEY".into()),
        api_key_secret: None,
    };
    let process = [
        ("PATH", path.as_str()),
        ("SACI_SECRET_MCP_TOKEN", "mcp-token-value"),
        ("SACI_LLM_API_KEY", "llm-key-value"),
    ]
    .map(|(k, v)| (k.to_owned(), v.to_owned()));
    Credentials::project(&environment, &llm, process).unwrap()
}

fn stdio(command: &str, tools: Option<&[&str]>) -> McpServerSpec {
    McpServerSpec {
        transport: McpTransport::Stdio(StdioServer {
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::from([("TOKEN".into(), "${MCP_TOKEN}".into())]),
        }),
        tools: tools.map(|t| t.iter().map(|s| (*s).to_owned()).collect()),
    }
}

fn http(url: &str, token: &str) -> McpServerSpec {
    McpServerSpec {
        transport: McpTransport::Http(HttpServer {
            url: url.into(),
            headers: BTreeMap::from([("Authorization".into(), format!("Bearer {token}"))]),
        }),
        tools: None,
    }
}

async fn call_text(server: &McpServer, tool: &'static str, args: serde_json::Value) -> String {
    let serde_json::Value::Object(args) = args else {
        panic!("arguments must be an object")
    };
    let request = CallToolRequestParams::new(tool).with_arguments(args);
    let result = Box::pin(server.peer().call_tool(request)).await.unwrap();
    result.content[0].as_text().unwrap().text.clone()
}

#[tokio::test]
async fn stdio_servers_resolve_through_path_and_get_only_tool_credentials() {
    let credentials = credentials();
    let server = connect("fake", &stdio("saci-fake-mcp", None), &credentials, TIMEOUT)
        .await
        .unwrap();
    let names: Vec<_> = server.tools().iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names, ["echo", "fail", "sleep", "env", "record"]);
    let token = Box::pin(call_text(
        &server,
        "env",
        serde_json::json!({"name": "TOKEN"}),
    ))
    .await;
    assert_eq!(token, "mcp-token-value");
    let llm = Box::pin(call_text(
        &server,
        "env",
        serde_json::json!({"name": "SACI_LLM_API_KEY"}),
    ))
    .await;
    assert_eq!(llm, "", "the LLM key never reaches MCP servers");
    server.shutdown().await;
}

#[tokio::test]
async fn tool_selection_is_enforced() {
    let credentials = credentials();
    let selected = stdio("saci-fake-mcp", Some(&["echo"]));
    let server = connect("fake", &selected, &credentials, TIMEOUT)
        .await
        .unwrap();
    assert_eq!(server.tools().len(), 1);
    server.shutdown().await;
    let missing = stdio("saci-fake-mcp", Some(&["echo", "deploy"]));
    let error = connect("fake", &missing, &credentials, TIMEOUT)
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .contains("does not offer deploy; it offers echo, fail"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn startup_failures_are_bounded_and_explained() {
    let credentials = credentials();
    let absent = stdio("no-such-mcp-server", None);
    let error = connect("ghost", &absent, &credentials, TIMEOUT)
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .contains("`no-such-mcp-server` was not found in the worker PATH"),
        "{}",
        error.message
    );
    let started = Instant::now();
    let silent = stdio("sleep", None);
    let mut silent = silent;
    if let McpTransport::Stdio(stdio) = &mut silent.transport {
        stdio.args = vec!["30".into()];
    }
    let error = connect("mute", &silent, &credentials, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        error.message.contains("did not initialize within 1s"),
        "{}",
        error.message
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn http_servers_receive_expanded_headers() {
    let credentials = credentials();
    let (url, stop) = fake_mcp::serve_http(Some("mcp-token-value".into()))
        .await
        .unwrap();
    let server = connect("web", &http(&url, "${MCP_TOKEN}"), &credentials, TIMEOUT)
        .await
        .unwrap();
    let echoed = Box::pin(call_text(
        &server,
        "echo",
        serde_json::json!({"text": "hi"}),
    ))
    .await;
    assert_eq!(echoed, "echo: hi");
    server.shutdown().await;
    let error = connect("web", &http(&url, "wrong"), &credentials, TIMEOUT)
        .await
        .unwrap_err();
    assert!(
        error.message.contains("initialization failed"),
        "{}",
        error.message
    );
    stop.cancel();
}

#[tokio::test]
async fn one_failing_server_fails_the_whole_set() {
    let credentials = credentials();
    let servers = BTreeMap::from([
        ("a-good".to_owned(), stdio("saci-fake-mcp", None)),
        ("b-bad".to_owned(), stdio("no-such-mcp-server", None)),
    ]);
    let error = connect_all(&servers, &credentials, TIMEOUT)
        .await
        .unwrap_err();
    assert!(
        error.message.starts_with("MCP server `b-bad`"),
        "{}",
        error.message
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let unreachable = http(&format!("http://{address}/mcp"), "x");
    let error = connect("down", &unreachable, &credentials, TIMEOUT)
        .await
        .unwrap_err();
    assert!(
        error.message.starts_with("MCP server `down`"),
        "{}",
        error.message
    );
}

/// Captures what the worker logs while a test runs.
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    /// The server task may log from any runtime thread, so the capture is
    /// global; no other test in this file sets a subscriber.
    fn global() -> Self {
        let logs = Self::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_global_default(subscriber).unwrap();
        logs
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[tokio::test]
async fn server_stderr_is_logged_with_credentials_redacted() {
    let logs = Captured::global();
    let mut chatty = stdio("sh", None);
    if let McpTransport::Stdio(stdio) = &mut chatty.transport {
        stdio.args = vec![
            "-c".into(),
            "echo \"starting with $TOKEN\" >&2; sleep 30".into(),
        ];
    }
    let credentials = credentials();
    let error = connect("chatty", &chatty, &credentials, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        error.message.contains("did not initialize"),
        "{}",
        error.message
    );
    let started = Instant::now();
    while !logs.text().contains("starting with") && started.elapsed() < TIMEOUT {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let text = logs.text();
    assert!(text.contains("mcp server: starting with"), "{text}");
    assert!(text.contains("chatty"), "{text}");
    assert!(!text.contains("mcp-token-value"), "{text}");
}
