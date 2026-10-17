//! Outcome actions end to end in the backend: a finished run's report is
//! stored through the worker API, the dispatcher calls the configured tool
//! of a stdio `saci-fake-mcp`, and the API shows the delivery. The server
//! records what it received.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::actions::{TICKETS_TOKEN, action, actions, finish, recorded, settled, start};
use saci_core::ids::RunId;
use serde_json::{Value, json};

#[tokio::test]
async fn a_failed_run_dispatches_only_the_authorized_context() {
    let fixture = start().await;
    let api = &fixture.api;
    let run = finish(api, "failure").await;
    let report = api.get(&format!("/api/v1/runs/{run}/report")).await.body;
    let items = settled(api, &run).await;
    let ticket = action(&items, "open-ticket");
    assert_eq!(ticket["state"], "completed", "{ticket}");
    assert_eq!(ticket["attempts"], 1);
    assert_eq!(ticket["result"], "recorded action 1");
    let calls = recorded(&fixture.record);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(
        calls[0]["arguments"], ticket["arguments"],
        "the recorded payload was sent"
    );
    check_context(&calls[0], &run);
    let after = api.get(&format!("/api/v1/runs/{run}/report")).await.body;
    assert_eq!(report, after, "delivery does not change the report");
    assert_eq!(after["outcome"], "failed");
}

/// The server got the rendered title and body, its configured variables,
/// and nothing else of the run.
fn check_context(call: &Value, run: &RunId) {
    let arguments = &call["arguments"];
    let keys: Vec<&String> = arguments.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["title", "body"]);
    assert_eq!(arguments["title"], "Service health: failed");
    let body = arguments["body"].as_str().unwrap();
    assert!(
        body.contains(&format!("Run {run} in local: 1 failed, 2 skipped")),
        "{body}"
    );
    assert!(
        body.contains("- [failed] 1.1 Check service health"),
        "{body}"
    );
    assert!(body.contains("ev-1 (1.1): http GET"), "{body}");
    assert!(
        !body.contains("http://service"),
        "environment parameters are not context"
    );
    assert_eq!(
        call["env"],
        json!(["FAKE_MCP_RECORD", "PATH", "TICKET_TOKEN"])
    );
    assert!(!call.to_string().contains(TICKETS_TOKEN));
}

#[tokio::test]
async fn failed_deliveries_are_recorded_without_changing_the_verdict() {
    let fixture = start().await;
    let api = &fixture.api;
    let run = finish(api, "failure").await;
    let items = settled(api, &run).await;
    let broken = action(&items, "broken-tool");
    assert_eq!(broken["state"], "error");
    assert_eq!(
        broken["error"],
        "the tool reported an error: the tool failed on purpose"
    );
    let wrong = action(&items, "wrong-arguments");
    assert_eq!(wrong["state"], "error");
    let reason = wrong["error"].as_str().unwrap();
    assert!(
        reason.contains("`title` is required; nothing was sent"),
        "{reason}"
    );
    let summary = api.get(&format!("/api/v1/runs/{run}")).await.body;
    assert_eq!(summary["outcome"], "failed");
    assert_eq!(
        recorded(&fixture.record).len(),
        1,
        "only open-ticket reached the server"
    );
}

#[tokio::test]
async fn approval_required_actions_stay_pending() {
    let fixture = start().await;
    let api = &fixture.api;
    let run = finish(api, "failure").await;
    settled(api, &run).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let escalate = action(&actions(api, &run).await, "escalate").clone();
    assert_eq!(escalate["state"], "pending_approval");
    assert_eq!(escalate["approval"], "required");
    assert_eq!(escalate["attempts"], 0);
    assert_eq!(escalate["arguments"]["title"], format!("Escalate {run}"));
    let titles: Vec<Value> = recorded(&fixture.record)
        .iter()
        .map(|call| call["arguments"]["title"].clone())
        .collect();
    assert_eq!(titles, [json!("Service health: failed")]);
}

#[tokio::test]
async fn only_the_outcomes_an_action_names_trigger_it() {
    let fixture = start().await;
    let api = &fixture.api;
    let run = finish(api, "success").await;
    let items = settled(api, &run).await;
    let names: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["on-pass"]);
    assert_eq!(action(&items, "on-pass")["state"], "completed");
    let unknown = api
        .get("/api/v1/runs/run-00000000000000000000000000000000/actions")
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}
