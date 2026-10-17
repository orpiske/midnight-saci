//! Fixture tests of the configured model connection against a scripted
//! OpenAI-compatible server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rig::AgentBuilder;
use rig::message::ToolName;
use rig::tool::{DynamicTool, ToolOutput};
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::{ExecutionLimits, LlmSettings};
use saci_testkit::{FakeModel, Reply, text_reply, tool_calls_reply};
use saci_worker::llm::{Backoff, LlmConnection};
use serde_json::json;

const FAST: Backoff = Backoff {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(50),
};

fn settings(base_url: &str, api_key_env: Option<&str>) -> LlmSettings {
    LlmSettings {
        base_url: base_url.to_owned(),
        model: "test-model".to_owned(),
        api_key_env: api_key_env.map(str::to_owned),
        api_key_secret: None,
    }
}

fn connect(settings: &LlmSettings, limits: &ExecutionLimits, key: Option<&str>) -> LlmConnection {
    let key = key.map(str::to_owned);
    LlmConnection::with_backoff(settings, limits, |_| key.clone(), FAST).unwrap()
}

fn counting_tool(calls: Arc<AtomicUsize>) -> DynamicTool {
    DynamicTool::new(
        ToolName::new("touch").unwrap(),
        "Counts calls",
        json!({"type": "object", "properties": {}}),
        move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(ToolOutput::text("touched")) })
        },
    )
}

async fn ask(connection: &LlmConnection) -> Result<String, ExecutionError> {
    let agent = AgentBuilder::new(connection.model()).build();
    agent
        .prompt("hello")
        .max_turns(connection.max_turns())
        .await
        .map(|response| response.output().clone())
        .map_err(|error| connection.classify(&error))
}

#[tokio::test]
async fn prefixes_and_trailing_paths_reach_chat_completions_once() {
    let model = FakeModel::start(vec![]).await.unwrap();
    let root = model.base_url().trim_end_matches("/v1").to_owned();
    let limits = ExecutionLimits::default();
    for base in [
        format!("{root}/api/v1"),
        format!("{root}/v1/"),
        format!("{root}/v1/chat/completions"),
    ] {
        model.push([Reply::ok(&text_reply("hi"))]);
        let reply = ask(&connect(&settings(&base, None), &limits, None)).await;
        assert_eq!(reply.unwrap(), "hi");
    }
    let paths: Vec<_> = model.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        [
            "/api/v1/chat/completions",
            "/v1/chat/completions",
            "/v1/chat/completions"
        ]
    );
    assert_eq!(model.requests()[0].body["model"], "test-model");
}

#[tokio::test]
async fn credentials_are_optional_but_must_exist_when_configured() {
    let model = FakeModel::start(vec![
        Reply::ok(&text_reply("anon")),
        Reply::ok(&text_reply("authed")),
    ])
    .await
    .unwrap();
    let limits = ExecutionLimits::default();
    ask(&connect(&settings(&model.base_url(), None), &limits, None))
        .await
        .unwrap();
    let keyed = settings(&model.base_url(), Some("SACI_TEST_KEY"));
    ask(&connect(&keyed, &limits, Some("sk-test")))
        .await
        .unwrap();
    let requests = model.requests();
    assert!(!requests[0].headers.contains_key("authorization"));
    assert_eq!(requests[1].headers["authorization"], "Bearer sk-test");
    let missing = LlmConnection::new(&keyed, &limits, |_| None).unwrap_err();
    assert_eq!(missing.kind, ErrorKind::InvalidSpecification);
    assert!(
        missing.message.contains("SACI_TEST_KEY"),
        "{}",
        missing.message
    );
}

async fn failure(replies: Vec<Reply>, retries: u32) -> (ExecutionError, usize) {
    let model = FakeModel::start(replies).await.unwrap();
    let limits = ExecutionLimits {
        model_request_retries: retries,
        ..ExecutionLimits::default()
    };
    let keyed = settings(&model.base_url(), Some("KEY"));
    let error = ask(&connect(&keyed, &limits, Some("sk-secret-123")))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Provider);
    assert!(
        !error.message.contains("sk-secret-123"),
        "{}",
        error.message
    );
    (error, model.requests().len())
}

#[tokio::test]
async fn provider_errors_are_actionable_and_redacted() {
    let echo = r#"{"error":{"message":"bad key sk-secret-123"}}"#;
    let (auth, requests) = failure(vec![Reply::raw(401, echo)], 2).await;
    assert!(
        auth.message.contains("rejected the credentials"),
        "{}",
        auth.message
    );
    assert!(auth.message.contains("[REDACTED]"), "{}", auth.message);
    assert_eq!(requests, 1, "authentication failures are not retried");
    let quota = r#"{"error":{"code":"insufficient_quota"}}"#;
    let (quota, requests) = failure(vec![Reply::raw(429, quota)], 2).await;
    assert!(
        quota.message.contains("quota is exhausted"),
        "{}",
        quota.message
    );
    assert_eq!(requests, 1, "an exhausted quota is not retried");
    let (missing, _) = failure(vec![Reply::raw(404, "no such model")], 0).await;
    assert!(missing.message.contains("llm.base_url and llm.model"));
    let tools = r#"{"error":{"message":"tools are not supported"}}"#;
    let (tools, _) = failure(vec![Reply::raw(400, tools)], 0).await;
    assert!(tools.message.contains("tool calling"), "{}", tools.message);
}

#[tokio::test]
async fn malformed_responses_are_reported() {
    let (error, requests) = failure(vec![Reply::raw(200, "not json")], 2).await;
    assert!(
        error.message.contains("malformed response"),
        "{}",
        error.message
    );
    assert_eq!(requests, 1);
}

#[tokio::test]
async fn transient_failures_are_retried_within_the_bound() {
    let model = FakeModel::start(vec![
        Reply::raw(503, "busy"),
        Reply::raw(429, "slow down").header("Retry-After", "0"),
        Reply::ok(&text_reply("recovered")),
    ])
    .await
    .unwrap();
    let limits = ExecutionLimits::default();
    let reply = ask(&connect(&settings(&model.base_url(), None), &limits, None)).await;
    assert_eq!(reply.unwrap(), "recovered");
    assert_eq!(model.requests().len(), 3);
    let busy = vec![Reply::raw(503, "busy"); 4];
    let (error, requests) = failure(busy, 2).await;
    assert!(
        error.message.contains("unavailable (HTTP 503)"),
        "{}",
        error.message
    );
    assert_eq!(requests, 3, "one request plus two retries");
    let (_, requests) = failure(vec![Reply::raw(500, "down")], 0).await;
    assert_eq!(requests, 1, "zero retries sends one request");
}

#[tokio::test]
async fn retries_never_replay_tool_calls() {
    let model = FakeModel::start(vec![
        Reply::ok(&tool_calls_reply(&[("c1", "touch", "{}")])),
        Reply::raw(502, "gateway"),
        Reply::ok(&text_reply("done")),
    ])
    .await
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = counting_tool(calls.clone());
    let connection = connect(
        &settings(&model.base_url(), None),
        &ExecutionLimits::default(),
        None,
    );
    let agent = AgentBuilder::new(connection.model())
        .dynamic_tool(tool)
        .build();
    let response = agent.prompt("go").max_turns(5).await.unwrap();
    assert_eq!(response.output(), "done");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1].body, requests[2].body,
        "only the request repeats"
    );
}

#[tokio::test]
async fn model_requests_time_out() {
    let slow = Reply::ok(&text_reply("late")).delayed(Duration::from_secs(5));
    let model = FakeModel::start(vec![slow.clone(), slow]).await.unwrap();
    let limits = ExecutionLimits {
        model_request_timeout_secs: 1,
        ..ExecutionLimits::default()
    };
    let started = Instant::now();
    let error = ask(&connect(&settings(&model.base_url(), None), &limits, None))
        .await
        .unwrap_err();
    assert!(error.message.contains("timed out"), "{}", error.message);
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[tokio::test]
async fn unreachable_providers_are_retried_then_reported() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let limits = ExecutionLimits {
        model_request_retries: 1,
        ..ExecutionLimits::default()
    };
    let base = format!("http://{address}/v1");
    let error = ask(&connect(&settings(&base, None), &limits, None))
        .await
        .unwrap_err();
    assert!(error.message.contains("cannot reach"), "{}", error.message);
}

#[tokio::test]
async fn step_call_limits_become_limit_errors() {
    let looping = Reply::ok(&tool_calls_reply(&[("c1", "touch", "{}")]));
    let model = FakeModel::start(vec![looping.clone(), looping])
        .await
        .unwrap();
    let limits = ExecutionLimits {
        model_calls_per_step: 2,
        ..ExecutionLimits::default()
    };
    let connection = connect(&settings(&model.base_url(), None), &limits, None);
    let tool = counting_tool(Arc::default());
    let agent = AgentBuilder::new(connection.model())
        .dynamic_tool(tool)
        .build();
    let error = agent
        .prompt("go")
        .max_turns(connection.max_turns())
        .await
        .unwrap_err();
    let classified = connection.classify(&error);
    assert_eq!(classified.kind, ErrorKind::LimitExceeded);
}

/// Captures what the worker logs while a test runs.
#[derive(Clone, Default)]
struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn retry_warnings_never_log_the_provider_body() {
    let echo = r#"{"error":{"message":"upstream failed using key sk-review-dummy-secret"}}"#;
    let model = FakeModel::start(vec![Reply::raw(503, echo), Reply::ok(&text_reply("ok"))])
        .await
        .unwrap();
    // The request may run on any runtime thread, so the capture is global;
    // the key below is used by this test only.
    let logs = Captured::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    let keyed = settings(&model.base_url(), Some("KEY"));
    let limits = ExecutionLimits::default();
    let reply = ask(&connect(&keyed, &limits, Some("sk-review-dummy-secret"))).await;
    assert_eq!(reply.unwrap(), "ok");
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(text.contains("retrying model request"), "{text}");
    assert!(text.contains("the provider answered HTTP 503"), "{text}");
    assert!(!text.contains("sk-review-dummy-secret"), "{text}");
    assert!(!text.contains("upstream failed"), "{text}");
}
