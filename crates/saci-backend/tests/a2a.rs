//! The A2A endpoint, driven over HTTP by a JSON-RPC client: the agent
//! card, `message/send`, `tasks/get` and `tasks/cancel` on investigations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::actions::{PLAN, fixture_events};
use common::{TOKEN, TestBackend};
use saci_backend::api::ClientAuth;
use saci_core::event::RunEvent;
use saci_core::ids::RunId;
use saci_core::investigation::INVESTIGATION_UNIT;
use saci_core::redaction::Redactor;
use serde_json::{Value, json};

const CONFIG: &str = "llm:
  base_url: http://model:11434/v1
  model: test-model
environments:
  local:
    parameters:
      URL: http://service
  investigator: {}
investigations:
  environment: investigator
  repositories:
    - name: orders
      url: https://example.com/orders.git
      ref: main
  instructions: Search for an open bug first.
";

/// A backend served on a loopback port, and a JSON-RPC client for it.
struct Agent {
    api: TestBackend,
    base: String,
    http: reqwest::Client,
}

impl Agent {
    async fn start() -> Self {
        Self::start_with(ClientAuth::Open).await
    }

    async fn start_with(auth: ClientAuth) -> Self {
        let api = TestBackend::start_with(auth, CONFIG).await;
        let base = serve(&api).await;
        Self {
            api,
            base,
            http: reqwest::Client::new(),
        }
    }

    /// Sends one JSON-RPC request and returns the whole response.
    async fn call(&self, method: &str, params: &Value) -> Value {
        let request = json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params});
        self.raw(&request.to_string()).await
    }

    async fn raw(&self, body: &str) -> Value {
        let response = self
            .http
            .post(format!("{}/a2a", self.base))
            .bearer_auth(TOKEN)
            .header("content-type", "application/json")
            .body(body.to_owned())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "JSON-RPC answers use status 200");
        response.json().await.unwrap()
    }

    /// The result of a successful call.
    async fn result(&self, method: &str, params: &Value) -> Value {
        let response = self.call(method, params).await;
        assert_eq!(response["id"], 7, "{response}");
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    /// The error code of a failed call.
    async fn error(&self, method: &str, params: &Value) -> i64 {
        let response = self.call(method, params).await;
        response["error"]["code"].as_i64().unwrap()
    }

    async fn send(&self, message_id: &str, data: &Value) -> Value {
        let message = json!({
            "kind": "message",
            "role": "user",
            "messageId": message_id,
            "parts": [{"kind": "text", "text": "Investigate this."}, {"kind": "data", "data": data}],
        });
        let params = json!({"message": message, "configuration": {"blocking": true}});
        self.result("message/send", &params).await
    }

    /// A finished run that replayed the run fixture `name`.
    async fn finished(&self, name: &str) -> RunId {
        let run = self.api.submit_plan(PLAN).await;
        self.append(&run, fixture_events(name, &run)).await;
        run
    }

    async fn append(&self, run: &RunId, events: Vec<RunEvent>) {
        let store = &self.api.backend.store;
        store
            .append_events(run, events, Redactor::new())
            .await
            .unwrap();
    }
}

async fn serve(api: &TestBackend) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = api.app.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

fn task_id(task: &Value) -> RunId {
    RunId::parse(task["id"].as_str().unwrap()).unwrap()
}

/// Events of an investigation run, numbered from `first`.
fn events(run: &RunId, first: u64, events: Vec<Value>) -> Vec<RunEvent> {
    (first..)
        .zip(events)
        .map(|(seq, mut event)| {
            event["run_id"] = json!(run.as_str());
            event["seq"] = json!(seq);
            event["at"] = json!("2026-10-23T10:00:00Z");
            serde_json::from_value(event).unwrap()
        })
        .collect()
}

fn started() -> Vec<Value> {
    vec![
        json!({"type": "run_started", "model": "m"}),
        json!({"type": "attempt_started", "unit": INVESTIGATION_UNIT, "attempt": 1}),
    ]
}

fn concluded() -> Vec<Value> {
    let unit = INVESTIGATION_UNIT;
    let report = json!({
        "outcome": "concluded",
        "summary": "The handler drops the currency.",
        "sources": [{"name": "orders", "commit": "abc123"}],
        "hypotheses": [{
            "statement": "`total` ignores the currency field.",
            "confidence": "high",
            "evidence": [],
            "code": [{"repository": "orders", "commit": "abc123", "path": "src/orders.rs",
                      "start_line": 10, "end_line": 14}],
        }],
        "bug": {"url": "https://example.com/issues/7", "title": "Totals ignore the currency"},
    });
    vec![
        json!({"type": "investigation_concluded", "unit": unit, "attempt": 1, "report": report}),
        json!({"type": "attempt_finished", "unit": unit, "attempt": 1, "outcome": "passed"}),
        json!({"type": "unit_finished", "unit": unit, "outcome": "passed"}),
        json!({"type": "run_finished", "outcome": "passed", "reason": "concluded"}),
    ]
}

#[tokio::test]
async fn the_agent_card_names_the_skill_and_the_endpoint() {
    let agent = Agent::start_with(ClientAuth::Token(TOKEN.into())).await;
    let response = agent
        .http
        .get(format!("{}/.well-known/agent-card.json", agent.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "the card needs no token");
    let card: Value = response.json().await.unwrap();
    assert_eq!(card["url"], format!("{}/a2a", agent.base));
    assert_eq!(card["preferredTransport"], "JSONRPC");
    assert_eq!(card["skills"][0]["id"], "investigate-failure");
    assert_eq!(card["skills"].as_array().unwrap().len(), 1);
    assert_eq!(card["capabilities"]["streaming"], false);
    assert_eq!(card["capabilities"]["pushNotifications"], false);
    assert_eq!(card["securitySchemes"]["bearer"]["scheme"], "bearer");

    let unauthorized = agent
        .http
        .post(format!("{}/a2a", agent.base))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tasks/get", "params": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401, "the methods need the token");
}

#[tokio::test]
async fn the_endpoint_is_served_only_with_investigations_configured() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let base = serve(&api).await;
    let http = reqwest::Client::new();
    let card = http
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(card.status(), 404);
    let rpc = http
        .post(format!("{base}/a2a"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tasks/get"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rpc.status(), 404);
}

#[tokio::test]
async fn a_message_starts_an_investigation_followed_as_a_task() {
    let agent = Agent::start().await;
    let run = agent.finished("failure").await;
    let task = agent.send("message-1", &json!({"run_id": run})).await;
    assert_eq!(task["kind"], "task");
    assert_eq!(task["status"]["state"], "submitted", "also when blocking");
    assert_eq!(task["contextId"], json!(run));
    assert_eq!(task["metadata"]["unit"], "1.1");
    let id = task_id(&task);
    let submitted = agent.api.supervisor.submitted.lock().unwrap().clone();
    assert_eq!(submitted.last(), Some(&id), "the worker is started");
    let shown = agent.api.get(&format!("/api/v1/investigations/{id}")).await;
    assert_eq!(
        shown.body["parent_run"],
        json!(run),
        "the same investigation"
    );

    agent.append(&id, events(&id, 1, started())).await;
    let working = agent.result("tasks/get", &json!({"id": id})).await;
    assert_eq!(working["status"]["state"], "working");
    assert!(working.get("artifacts").is_none());

    agent.append(&id, events(&id, 3, concluded())).await;
    check_completed(&agent.result("tasks/get", &json!({"id": id})).await);
    let code = agent.error("tasks/cancel", &json!({"id": id})).await;
    assert_eq!(code, -32002, "a finished task cannot be canceled");
}

/// A completed task has the result and the summary as artifacts.
fn check_completed(task: &Value) {
    assert_eq!(task["status"]["state"], "completed", "{task}");
    let artifacts = task["artifacts"].as_array().unwrap();
    let result = &artifacts[0]["parts"][0];
    assert_eq!(result["kind"], "data");
    assert_eq!(result["data"]["outcome"], "concluded");
    assert_eq!(result["data"]["bug"]["url"], "https://example.com/issues/7");
    let summary = &artifacts[1]["parts"][0];
    assert_eq!(summary["kind"], "text");
    assert_eq!(summary["text"], "The handler drops the currency.");
}

#[tokio::test]
async fn the_same_message_id_returns_the_same_task() {
    let agent = Agent::start().await;
    let run = agent.finished("failure").await;
    let data = json!({"run_id": run, "unit": "1.1"});
    let first = agent.send("message-1", &data).await;
    let again = agent.send("message-1", &data).await;
    assert_eq!(task_id(&again), task_id(&first));
    assert_eq!(agent.api.supervisor.submitted.lock().unwrap().len(), 2);

    let other = agent.finished("failure").await;
    let reused = json!({"run_id": other});
    let message = json!({"messageId": "message-1", "parts": [{"kind": "data", "data": reused}]});
    let code = agent
        .error("message/send", &json!({"message": message}))
        .await;
    assert_eq!(code, -32602, "a message ID with another request");
    // A REST idempotency key with the same text is another key.
    let keyed = agent
        .api
        .call(
            axum::http::Method::POST,
            "/api/v1/investigations",
            Some(&reused),
            &[("idempotency-key", "message-1")],
        )
        .await;
    assert_eq!(keyed.status, 202, "{}", keyed.body);
}

#[tokio::test]
async fn invalid_requests_give_rejected_tasks() {
    let agent = Agent::start().await;
    let run = agent.finished("failure").await;
    let passed = agent.finished("success").await;
    for (data, expected) in [
        (json!({"run_id": passed}), "has no unit that failed"),
        (json!({"run_id": run, "unit": "1.2"}), "did not fail"),
        (json!({"run_id": RunId::generate()}), "run not found"),
        (
            json!({"run_id": run, "sources": [{"name": "web"}]}),
            "`web` is not a configured",
        ),
        (json!({"run": run}), "invalid investigation request"),
    ] {
        let task = agent.send(&format!("message-{data}"), &data).await;
        assert_eq!(task["status"]["state"], "rejected", "{data}");
        let reason = task["status"]["message"]["parts"][0]["text"]
            .as_str()
            .unwrap();
        assert!(reason.contains(expected), "{data}: {reason}");
        let code = agent.error("tasks/get", &json!({"id": task["id"]})).await;
        assert_eq!(code, -32001, "a rejected task is not stored");
    }
    let message = json!({"messageId": "text-only", "parts": [{"kind": "text", "text": "hi"}]});
    let task = agent
        .result("message/send", &json!({"message": message}))
        .await;
    assert_eq!(task["status"]["state"], "rejected");
    let submitted = agent.api.supervisor.submitted.lock().unwrap().len();
    assert_eq!(submitted, 2, "only the two runs");
}

#[tokio::test]
async fn cancel_requests_cancellation_and_the_task_ends_canceled() {
    let agent = Agent::start().await;
    let run = agent.finished("failure").await;
    let id = task_id(&agent.send("message-1", &json!({"run_id": run})).await);
    agent.append(&id, events(&id, 1, started())).await;
    let task = agent.result("tasks/cancel", &json!({"id": id})).await;
    assert_eq!(task["id"], json!(id));
    let cancelled = agent.api.supervisor.cancelled.lock().unwrap().clone();
    assert_eq!(cancelled, std::slice::from_ref(&id));

    let unit = INVESTIGATION_UNIT;
    let stopped = vec![
        json!({"type": "attempt_finished", "unit": unit, "attempt": 1, "outcome": "cancelled"}),
        json!({"type": "unit_finished", "unit": unit, "outcome": "cancelled"}),
        json!({"type": "run_finished", "outcome": "cancelled", "reason": "cancelled by a client"}),
    ];
    agent.append(&id, events(&id, 3, stopped)).await;
    let done = agent.result("tasks/get", &json!({"id": id})).await;
    assert_eq!(done["status"]["state"], "canceled", "{done}");
    let reason = &done["status"]["message"]["parts"][0]["text"];
    assert_eq!(reason, "cancelled by a client");
}

#[tokio::test]
async fn unknown_tasks_methods_and_malformed_requests_are_errors() {
    let agent = Agent::start().await;
    let run = agent.finished("failure").await;
    assert_eq!(
        agent.error("tasks/get", &json!({"id": run})).await,
        -32001,
        "a test run"
    );
    assert_eq!(agent.error("tasks/get", &json!({"id": "x"})).await, -32001);
    assert_eq!(
        agent.error("tasks/cancel", &json!({"id": run})).await,
        -32001
    );
    assert_eq!(agent.error("message/stream", &json!({})).await, -32601);
    assert_eq!(agent.error("tasks/get", &json!({})).await, -32602);
    assert_eq!(agent.raw("{").await["error"]["code"], -32700);
    let no_version = agent.raw(r#"{"id": 3, "method": "tasks/get"}"#).await;
    assert_eq!(no_version["error"]["code"], -32600);
    assert_eq!(no_version["id"], 3);
}
