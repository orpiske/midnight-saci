//! Compatibility checks for the selected agent library, Rig 0.44.
//!
//! These pin down the behavior Saci relies on (see docs/agent-library.md)
//! against a scripted OpenAI-compatible server, so a dependency upgrade that
//! changes any of it fails here first.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rig::AgentBuilder;
use rig::agent::{
    AgentHook, DispatchAction, DispatchEvent, HookContext, OutcomeAction, OutcomeEvent,
};
use rig::completion::PromptError;
use rig::http_client::ReqwestClient;
use rig::message::ToolName;
use rig::providers::openai::wire::{Auth, Dialect};
use rig::providers::openai::{OpenAI, OpenAIConfig};
use rig::tool::{DynamicTool, ToolOutput};
use saci_testkit::{FakeModel, Reply, text_reply, tool_calls_reply};
use serde_json::{Value, json};

static GATEWAY: Dialect = Dialect::gateway("saci", "http://127.0.0.1/v1", "SACI_LLM_API_KEY");

fn client(base_url: &str, key: Option<&str>, timeout: Duration) -> OpenAI {
    let http = rig::rig_reqwest::reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap();
    OpenAIConfig::with_key(&GATEWAY, key.unwrap_or(""))
        .with_auth(Auth::OptionalBearer)
        .with_base_url(base_url)
        .connect(ReqwestClient::from(http))
}

type Spans = Arc<Mutex<Vec<(String, Instant, Instant)>>>;

fn slow_tool(name: &str, spans: Spans) -> DynamicTool {
    let owned = name.to_owned();
    DynamicTool::new(
        ToolName::new(name).unwrap(),
        format!("test tool {name}"),
        json!({"type": "object", "properties": {"x": {"type": "string"}}}),
        move |args: Value| {
            let (spans, name) = (spans.clone(), owned.clone());
            Box::pin(async move {
                let start = Instant::now();
                tokio::time::sleep(Duration::from_millis(100)).await;
                spans
                    .lock()
                    .unwrap()
                    .push((name.clone(), start, Instant::now()));
                Ok(ToolOutput::text(format!("{name}:{args}")))
            })
        },
    )
}

#[derive(Clone, Default)]
struct Observer {
    seen: Arc<Mutex<Vec<String>>>,
    max_tool_calls: Option<usize>,
}

impl AgentHook for Observer {
    async fn on_dispatch(&self, ctx: &HookContext, event: DispatchEvent<'_>) -> DispatchAction {
        let Some(name) = event.tool_name() else {
            return DispatchAction::proceed();
        };
        let count = ctx.scratchpad().update(|n: &mut usize| {
            *n += 1;
            *n
        });
        self.seen
            .lock()
            .unwrap()
            .push(format!("call {name} {}", event.tool_args().unwrap_or("")));
        match self.max_tool_calls {
            Some(max) if count > max => {
                DispatchAction::stop(format!("tool call limit {max} reached"))
            }
            _ => DispatchAction::proceed(),
        }
    }

    async fn on_outcome(&self, _ctx: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        if let Some(result) = event.tool_result() {
            let id = event.call_id.map(ToString::to_string).unwrap_or_default();
            self.seen
                .lock()
                .unwrap()
                .push(format!("result {id} {}", result.status_name()));
        }
        OutcomeAction::proceed()
    }
}

#[tokio::test]
async fn chat_completions_url_and_optional_auth() {
    let model = FakeModel::start(vec![
        Reply::ok(&text_reply("a")),
        Reply::ok(&text_reply("b")),
    ])
    .await
    .unwrap();
    let anonymous = AgentBuilder::new(
        client(
            &format!("{}/", model.base_url()),
            None,
            Duration::from_secs(5),
        )
        .chat("m"),
    )
    .build();
    anonymous.prompt("hi").await.unwrap();
    let keyed = AgentBuilder::new(
        client(&model.base_url(), Some("sk-1"), Duration::from_secs(5)).chat("m"),
    )
    .build();
    keyed.prompt("hi").await.unwrap();
    let requests = model.requests();
    assert!(requests.iter().all(|r| r.path == "/v1/chat/completions"));
    assert!(!requests[0].headers.contains_key("authorization"));
    assert_eq!(requests[1].headers["authorization"], "Bearer sk-1");
    assert_eq!(requests[1].body["model"], "m");
}

fn assert_tool_results_correlate(body: &Value) {
    let messages = body["messages"].as_array().unwrap();
    let tool_ids: Vec<_> = messages
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["tool_call_id"].clone())
        .collect();
    assert_eq!(tool_ids, [json!("c1"), json!("c2")]);
    let assistant = messages.iter().find(|m| m["role"] == "assistant").unwrap();
    assert_eq!(assistant["tool_calls"][1]["id"], "c2");
}

#[tokio::test]
async fn tool_calls_run_sequentially_and_correlate() {
    let model = FakeModel::start(vec![
        Reply::ok(&tool_calls_reply(&[
            ("c1", "alpha", r#"{"x":"1"}"#),
            ("c2", "beta", r#"{"x":"2"}"#),
        ])),
        Reply::ok(&text_reply("done")),
    ])
    .await
    .unwrap();
    let spans = Spans::default();
    let observer = Observer::default();
    let agent =
        AgentBuilder::new(client(&model.base_url(), None, Duration::from_secs(5)).chat("m"))
            .dynamic_tools(vec![
                slow_tool("alpha", spans.clone()),
                slow_tool("beta", spans.clone()),
            ])
            .add_hook(observer.clone())
            .build();
    let response = agent.prompt("go").max_turns(3).await.unwrap();
    assert_eq!(response.output(), "done");
    let spans = spans.lock().unwrap();
    assert_eq!(spans[0].0, "alpha");
    assert!(spans[0].2 <= spans[1].1, "tools overlapped");
    assert_tool_results_correlate(&model.requests()[1].body);
    assert_eq!(observer.seen.lock().unwrap()[1], "result c1 success");
}

#[tokio::test]
async fn hooks_enforce_call_limits() {
    let model = FakeModel::start(vec![Reply::ok(&tool_calls_reply(&[
        ("c1", "alpha", "{}"),
        ("c2", "alpha", "{}"),
    ]))])
    .await
    .unwrap();
    let observer = Observer {
        max_tool_calls: Some(1),
        ..Observer::default()
    };
    let agent =
        AgentBuilder::new(client(&model.base_url(), None, Duration::from_secs(5)).chat("m"))
            .dynamic_tool(slow_tool("alpha", Spans::default()))
            .add_hook(observer)
            .build();
    let error = agent.prompt("go").max_turns(5).await.unwrap_err();
    assert!(matches!(error, PromptError::Cancelled { .. }), "{error}");
    assert_eq!(
        model.requests().len(),
        1,
        "no further model request after a stop"
    );
}

#[tokio::test]
async fn turn_limit_bounds_model_calls() {
    let looping = Reply::ok(&tool_calls_reply(&[("c1", "alpha", "{}")]));
    let model = FakeModel::start(vec![looping.clone(), looping.clone(), looping])
        .await
        .unwrap();
    let agent =
        AgentBuilder::new(client(&model.base_url(), None, Duration::from_secs(5)).chat("m"))
            .dynamic_tool(slow_tool("alpha", Spans::default()))
            .build();
    let error = agent.prompt("go").max_turns(2).await.unwrap_err();
    assert!(
        matches!(error, PromptError::MaxTurns { max_turns: 2, .. }),
        "{error}"
    );
    assert_eq!(model.requests().len(), 2);
}

#[tokio::test]
async fn provider_errors_are_not_retried_and_keep_their_status() {
    let model = FakeModel::start(vec![Reply::raw(
        429,
        r#"{"error":{"message":"slow down"}}"#,
    )])
    .await
    .unwrap();
    let agent =
        AgentBuilder::new(client(&model.base_url(), None, Duration::from_secs(5)).chat("m"))
            .build();
    let error = agent.prompt("hi").await.unwrap_err();
    assert_eq!(
        error.provider_response_status().map(|s| s.as_u16()),
        Some(429)
    );
    assert_eq!(model.requests().len(), 1);
}

#[tokio::test]
async fn requests_have_deadlines_and_runs_can_be_cancelled() {
    let slow = Reply::ok(&text_reply("late")).delayed(Duration::from_secs(3));
    let model = FakeModel::start(vec![slow.clone(), slow]).await.unwrap();
    let start = Instant::now();
    let timed =
        AgentBuilder::new(client(&model.base_url(), None, Duration::from_millis(200)).chat("m"))
            .build();
    assert!(timed.prompt("hi").await.is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    let agent =
        AgentBuilder::new(client(&model.base_url(), None, Duration::from_secs(10)).chat("m"))
            .build();
    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    let run = tokio::spawn(async move {
        tokio::select! {
            result = agent.prompt("hi").into_future() => Some(result.is_ok()),
            _ = cancelled => None,
        }
    });
    cancel.send(()).unwrap();
    assert_eq!(run.await.unwrap(), None);
}
