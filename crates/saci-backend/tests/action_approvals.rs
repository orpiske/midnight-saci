//! Approval-required outcome actions: decisions name the digest of the
//! reviewed arguments, only approved actions are dispatched, and decisions
//! and pending approvals survive a backend restart.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::TestBackend;
use common::actions::{Fixture, action, actions, finish, recorded, settled, start};
use saci_core::ids::RunId;
use serde_json::{Value, json};

async fn decide(api: &TestBackend, run: &RunId, name: &str, kind: &str, body: &Value) -> Value {
    let uri = format!("/api/v1/runs/{run}/actions/{name}/{kind}");
    let answer = api.post(&uri, body).await;
    json!({"status": answer.status.as_u16(), "body": answer.body})
}

async fn escalate(api: &TestBackend, run: &RunId) -> Value {
    action(&actions(api, run).await, "escalate").clone()
}

/// A failed run whose automatic actions have settled, leaving `escalate`
/// pending.
async fn pending(fixture: &Fixture) -> RunId {
    let run = finish(&fixture.api, "failure").await;
    settled(&fixture.api, &run).await;
    assert_eq!(
        escalate(&fixture.api, &run).await["state"],
        "pending_approval"
    );
    run
}

/// A decision body naming the digest of `escalate`'s recorded arguments.
async fn reviewed(api: &TestBackend, run: &RunId, note: Option<&str>) -> Value {
    json!({"digest": escalate(api, run).await["digest"], "note": note})
}

fn titles(fixture: &Fixture) -> Vec<String> {
    recorded(&fixture.record)
        .iter()
        .map(|call| call["arguments"]["title"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn an_approval_dispatches_exactly_the_reviewed_payload() {
    let fixture = start().await;
    let api = &fixture.api;
    let run = pending(&fixture).await;
    let reviewed = escalate(api, &run).await;
    let wrong = decide(
        api,
        &run,
        "escalate",
        "approval",
        &json!({"digest": "0".repeat(64)}),
    )
    .await;
    assert_eq!(wrong["status"], 409);
    assert_eq!(wrong["body"]["error"]["code"], "digest_mismatch");
    assert_eq!(escalate(api, &run).await["state"], "pending_approval");
    let body = json!({"digest": reviewed["digest"], "note": "reviewed by the on-call"});
    let approved = decide(api, &run, "escalate", "approval", &body).await;
    assert_eq!(approved["status"], 200, "{approved}");
    assert_eq!(approved["body"]["decision"]["verdict"], "approved");
    let done = action(&settled(api, &run).await, "escalate").clone();
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(done["decision"]["note"], "reviewed by the on-call");
    let calls = recorded(&fixture.record);
    let sent = calls
        .iter()
        .find(|call| call["arguments"]["title"] == reviewed["arguments"]["title"]);
    assert_eq!(sent.unwrap()["arguments"], reviewed["arguments"]);
    let again = decide(api, &run, "escalate", "approval", &body).await;
    assert_eq!(again["body"]["error"]["code"], "action_not_pending");
}

#[tokio::test]
async fn rejected_actions_are_never_dispatched() {
    let fixture = start().await;
    let api = &fixture.api;
    let run = pending(&fixture).await;
    let digest = escalate(api, &run).await["digest"].clone();
    let rejected = decide(
        api,
        &run,
        "escalate",
        "rejection",
        &json!({"digest": digest}),
    )
    .await;
    assert_eq!(rejected["status"], 200, "{rejected}");
    assert_eq!(rejected["body"]["state"], "rejected");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(escalate(api, &run).await["state"], "rejected");
    assert!(
        !titles(&fixture)
            .iter()
            .any(|title| title.starts_with("Escalate"))
    );
    let late = decide(
        api,
        &run,
        "escalate",
        "approval",
        &json!({"digest": digest}),
    )
    .await;
    assert_eq!(late["body"]["error"]["code"], "action_not_pending");
}

#[tokio::test]
async fn only_approval_required_actions_take_decisions() {
    let fixture = start().await;
    let api = &fixture.api;
    let run = pending(&fixture).await;
    let ticket = action(&actions(api, &run).await, "open-ticket").clone();
    let body = json!({"digest": ticket["digest"]});
    let refused = decide(api, &run, "open-ticket", "approval", &body).await;
    assert_eq!(refused["status"], 409);
    assert_eq!(refused["body"]["error"]["code"], "action_not_pending");
    let missing = decide(api, &run, "no-such-action", "approval", &body).await;
    assert_eq!(missing["status"], 404);
    assert_eq!(missing["body"]["error"]["message"], "action not found");
    let malformed = api
        .post(
            &format!("/api/v1/runs/{run}/actions/escalate/approval"),
            &json!({}),
        )
        .await;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pending_approvals_and_decisions_survive_a_restart() {
    let fixture = start().await;
    let first = pending(&fixture).await;
    let second = pending(&fixture).await;
    let body = reviewed(&fixture.api, &first, Some("not needed")).await;
    decide(&fixture.api, &first, "escalate", "rejection", &body).await;
    let fixture = fixture.restart().await;
    let api = &fixture.api;
    let rejected = escalate(api, &first).await;
    assert_eq!(rejected["state"], "rejected");
    assert_eq!(rejected["decision"]["note"], "not needed");
    let waiting = api.get("/api/v1/actions").await.body;
    let runs: Vec<&Value> = waiting["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| &a["run_id"])
        .collect();
    assert_eq!(runs, [&json!(second.as_str())]);
    let body = reviewed(api, &second, None).await;
    decide(api, &second, "escalate", "approval", &body).await;
    let after = action(&settled(api, &second).await, "escalate").clone();
    assert_eq!(after["state"], "completed");
    let escalations = titles(&fixture)
        .into_iter()
        .filter(|t| t.starts_with("Escalate"))
        .count();
    assert_eq!(escalations, 1, "only the approved action was sent");
    let unknown = api.get("/api/v1/actions?state=sideways").await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
}
