#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The runner over real Streamable HTTP, with the `rmcp` client.

mod common;

use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, ClientRequest,
    Implementation, Request,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use saci_script_runner::config::Config;
use saci_script_runner::server;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

type Client = RunningService<RoleClient, ClientInfo>;

struct Runner {
    base: String,
    shutdown: CancellationToken,
    task: JoinHandle<anyhow::Result<()>>,
}

/// The runner configuration; `DIR` and `MAX` are replaced.
const CONFIG: &str = r"
execution:
  max_concurrent: MAX
  working_directory: DIR
  environment: {PATH: /usr/bin:/bin}
tools:
  - name: echo_args
    description: Print each argument
    executable: DIR/args.sh
    read_only: true
    arguments:
      - name: mode
        type: string
        required: true
        enum: [fast, slow]
        binding: {kind: option, flag: --mode}
      - name: text
        type: string
        binding: {kind: positional}
  - name: fail
    description: Exit with code 3
    executable: DIR/exit3.sh
  - name: sleep
    description: Sleep
    executable: DIR/sleep.sh
    arguments:
      - name: seconds
        type: string
        pattern: '^[0-9]+$'
        binding: {kind: positional}
  - name: hold
    description: Start a background process and wait for it
    executable: DIR/orphan-wait.sh
    arguments:
      - name: pid_file
        type: string
        required: true
        binding: {kind: positional}
";

impl Runner {
    async fn start(max_concurrent: usize, token: Option<&str>) -> Self {
        let yaml = CONFIG
            .replace("DIR", &common::scripts().display().to_string())
            .replace("MAX", &max_concurrent.to_string());
        let config: Config = serde_yaml::from_str(&yaml).unwrap();
        config.validate().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let shutdown = CancellationToken::new();
        let token = token.map(str::to_owned);
        let stop = shutdown.clone();
        let task = tokio::spawn(async move { server::serve(&config, token, listener, stop).await });
        Self {
            base,
            shutdown,
            task,
        }
    }

    fn url(&self) -> String {
        format!("{}/mcp", self.base)
    }

    async fn client(&self, token: Option<&str>) -> Result<Client, String> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.url());
        if let Some(token) = token {
            config = config.auth_header(token);
        }
        let info = ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("saci-test", "1"),
        );
        Box::pin(info.serve(StreamableHttpClientTransport::from_config(config)))
            .await
            .map_err(|error| error.to_string())
    }
}

fn params(name: &'static str, arguments: &Value) -> CallToolRequestParams {
    let params = CallToolRequestParams::new(name);
    match arguments {
        Value::Object(arguments) => params.with_arguments(arguments.clone()),
        _ => params,
    }
}

async fn call(client: &Client, name: &'static str, arguments: Value) -> CallToolResult {
    Box::pin(client.call_tool(params(name, &arguments)))
        .await
        .unwrap()
}

const fn structured(result: &CallToolResult) -> &Value {
    result.structured_content.as_ref().unwrap()
}

#[tokio::test]
async fn tools_list_shows_one_tool_per_script() {
    let runner = Runner::start(2, None).await;
    let client = runner.client(None).await.unwrap();
    let tools = Box::pin(client.list_all_tools()).await.unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, ["echo_args", "fail", "sleep", "hold"]);
    let echo = &tools[0];
    assert_eq!(echo.description.as_deref(), Some("Print each argument"));
    let schema = Value::Object((*echo.input_schema).clone());
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["required"], json!(["mode"]));
    assert_eq!(
        schema["properties"]["mode"]["enum"],
        json!(["fast", "slow"])
    );
    let annotations = echo.annotations.as_ref().unwrap();
    assert_eq!(annotations.read_only_hint, Some(true));
    let fail = tools[1].annotations.as_ref().unwrap();
    assert_eq!(fail.read_only_hint, Some(false));
    assert_eq!(fail.destructive_hint, Some(true));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn calls_run_the_fixed_script_with_bound_arguments() {
    let runner = Runner::start(2, None).await;
    let client = runner.client(None).await.unwrap();
    let text = "$(touch /tmp/pwned); echo hi | cat";
    let result = call(&client, "echo_args", json!({"text": text, "mode": "fast"})).await;
    assert_eq!(result.is_error, Some(false));
    let value = structured(&result);
    assert_eq!(value["status"], "completed");
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["stdout"], format!("[--mode]\n[fast]\n[{text}]\n"));
    assert!(
        value["execution_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    // The text fallback carries the same result.
    let text_content = result.content[0].as_text().unwrap();
    assert!(text_content.text.contains("\"status\":\"completed\""));

    let result = call(&client, "fail", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(structured(&result)["status"], "failed");
    assert_eq!(structured(&result)["exit_code"], 3);
    assert_eq!(structured(&result)["stderr"], "err\n");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn invalid_calls_are_rejected_before_anything_runs() {
    let runner = Runner::start(2, None).await;
    let client = runner.client(None).await.unwrap();
    for arguments in [
        json!({}),
        json!({"mode": "rm"}),
        json!({"mode": "fast", "text": "--help"}),
        json!({"mode": "fast", "executable": "/bin/sh"}),
        json!({"mode": "fast", "text": 5}),
    ] {
        let result = call(&client, "echo_args", arguments.clone()).await;
        assert_eq!(result.is_error, Some(true), "{arguments}");
        assert_eq!(structured(&result)["status"], "rejected", "{arguments}");
        assert!(structured(&result).get("stdout").is_none());
    }
    let unknown = Box::pin(client.call_tool(params("sh", &json!({"command": "id"}))))
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("unknown tool"), "{unknown}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn malformed_requests_get_protocol_errors() {
    let runner = Runner::start(1, None).await;
    let http = reqwest::Client::new();
    let response = http
        .post(runner.url())
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_client_error(), "{}", response.status());
    let health: Value = http
        .get(format!("{}/health", runner.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health, json!({"status": "ok"}));
}

#[tokio::test]
async fn the_bearer_token_and_origin_are_enforced() {
    let runner = Runner::start(1, Some("s3cret")).await;
    assert!(runner.client(None).await.is_err());
    assert!(runner.client(Some("wrong")).await.is_err());
    let client = runner.client(Some("s3cret")).await.unwrap();
    let result = call(&client, "fail", json!({})).await;
    assert_eq!(structured(&result)["exit_code"], 3);
    client.cancel().await.unwrap();

    let response = reqwest::Client::new()
        .post(runner.url())
        .header("authorization", "Bearer s3cret")
        .header("origin", "https://evil.example")
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_full_runner_rejects_calls_and_frees_its_slot() {
    let runner = Runner::start(1, None).await;
    let client = runner.client(None).await.unwrap();
    let peer = client.peer().clone();
    let first = tokio::spawn(async move {
        Box::pin(peer.call_tool(params("sleep", &json!({"seconds": "2"})))).await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let busy = call(&client, "fail", json!({})).await;
    assert_eq!(structured(&busy)["status"], "rejected");
    assert!(
        structured(&busy)["error"]
            .as_str()
            .unwrap()
            .contains("busy")
    );
    let first = first.await.unwrap().unwrap();
    assert_eq!(structured(&first)["status"], "completed");
    let after = call(&client, "fail", json!({})).await;
    assert_eq!(structured(&after)["status"], "failed");
    client.cancel().await.unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_cancelled_call_stops_the_script() {
    let runner = Runner::start(1, None).await;
    let client = runner.client(None).await.unwrap();
    let pid_file = tempfile::NamedTempFile::new().unwrap();
    let arguments = json!({"pid_file": pid_file.path().to_str().unwrap()});
    let request = ClientRequest::CallToolRequest(Request::new(params("hold", &arguments)));
    let handle = Box::pin(
        client
            .peer()
            .send_cancellable_request(request, PeerRequestOptions::no_options()),
    )
    .await
    .unwrap();
    let pid = common::read_pid(pid_file.path()).await;
    handle.cancel(Some("test".to_owned())).await.unwrap();
    assert!(
        common::gone(pid).await,
        "process {pid} survived cancellation"
    );
    // The slot is free again once the script is reaped.
    for _ in 0..50 {
        let after = call(&client, "fail", json!({})).await;
        if structured(&after)["status"] == "failed" {
            client.cancel().await.unwrap();
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the execution slot was not released");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn shutdown_stops_running_scripts() {
    let runner = Runner::start(1, None).await;
    let client = runner.client(None).await.unwrap();
    let pid_file = tempfile::NamedTempFile::new().unwrap();
    let arguments = json!({"pid_file": pid_file.path().to_str().unwrap()});
    let peer = client.peer().clone();
    let call =
        tokio::spawn(async move { Box::pin(peer.call_tool(params("hold", &arguments))).await });
    let pid = common::read_pid(pid_file.path()).await;
    runner.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(10), runner.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(common::gone(pid).await, "process {pid} survived shutdown");
    call.abort();
}
