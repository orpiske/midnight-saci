//! Outcome delivery across duplicate triggers, lost answers and backend
//! crashes. A call is replayed only with an idempotency key; any other
//! uncertain delivery stays `unknown` until a person reconciles or
//! redispatches it. The run's verdict never changes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::http::StatusCode;
use common::TestBackend;
use common::actions::{
    Fixture, LOST_RESPONSES, RECOVERY, action, dispatch, finish, lookup, recorded, report_events,
    settled, start_from,
};
use saci_core::action::ActionRecord;
use saci_core::ids::RunId;
use serde_json::{Value, json};

/// How many calls the fake server recorded with a title starting `prefix`.
fn sent(fixture: &Fixture, prefix: &str) -> usize {
    recorded(&fixture.record)
        .iter()
        .filter(|call| {
            call["arguments"]["title"]
                .as_str()
                .is_some_and(|title| title.starts_with(prefix))
        })
        .count()
}

async fn show(api: &TestBackend, run: &RunId, name: &str) -> Value {
    let answer = api.get(&format!("/api/v1/runs/{run}/actions/{name}")).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    answer.body
}

fn events(action: &Value) -> Vec<&str> {
    action["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["event"].as_str().unwrap())
        .collect()
}

async fn assert_failed(api: &TestBackend, run: &RunId) {
    let report = api.get(&format!("/api/v1/runs/{run}/report")).await.body;
    assert_eq!(
        report["outcome"], "failed",
        "delivery never changes the verdict"
    );
}

/// Claims every queued action as a dispatcher that then stops would, and,
/// when `delivered`, makes the call without recording its answer.
async fn crash(fixture: &Fixture, delivered: bool) {
    let backend = &fixture.api.backend;
    while let Some(claimed) = backend.store.claim_action().await.unwrap() {
        if delivered {
            let lookup = lookup();
            let delivery =
                saci_backend::actions::dispatch(&backend.config, &claimed, &|n| lookup(n)).await;
            assert!(delivery.result.is_some(), "{delivery:?}");
        }
    }
}

#[tokio::test]
async fn duplicate_triggers_record_and_send_each_action_once() {
    let fixture = start_from(RECOVERY, true).await;
    let _second = dispatch(&fixture.api);
    let api = &fixture.api;
    let run = finish(api, "failure").await;
    let again = report_events(api, &run, "failure").await;
    assert!(again.status.is_success() || again.status == StatusCode::CONFLICT);
    let items = settled(api, &run).await;
    assert_eq!(items.as_array().unwrap().len(), 2, "{items}");
    for name in ["keyed", "plain"] {
        assert_eq!(action(&items, name)["state"], "completed");
        assert_eq!(action(&items, name)["attempts"], 1);
    }
    assert_eq!((sent(&fixture, "Keyed"), sent(&fixture, "Plain")), (1, 1));
    let keyed = show(api, &run, "keyed").await;
    assert_eq!(events(&keyed), ["recorded", "claimed", "answered"]);
    let key = format!("saci-{run}-keyed");
    assert_eq!(keyed["idempotency_key"], key.as_str());
    assert_eq!(keyed["arguments"]["key"], key.as_str());
    assert_failed(api, &run).await;
}

#[tokio::test]
async fn a_crash_before_the_call_replays_only_keyed_actions() {
    let fixture = start_from(RECOVERY, false).await;
    let run = finish(&fixture.api, "failure").await;
    crash(&fixture, false).await;
    let fixture = fixture.restart().await;
    let api = &fixture.api;
    settled(api, &run).await;
    let keyed = show(api, &run, "keyed").await;
    assert_eq!(keyed["state"], "completed", "{keyed}");
    assert_eq!(keyed["attempts"], 2);
    let replay = ["recorded", "claimed", "replayed", "claimed", "answered"];
    assert_eq!(events(&keyed), replay);
    let plain = show(api, &run, "plain").await;
    assert_eq!(plain["state"], "unknown", "{plain}");
    let reason = plain["error"].as_str().unwrap();
    assert!(reason.contains("no idempotency key"), "{reason}");
    assert_eq!((sent(&fixture, "Keyed"), sent(&fixture, "Plain")), (1, 0));
    let uri = format!("/api/v1/runs/{run}/actions/plain/redispatch");
    let wrong = api.post(&uri, &json!({"digest": "0"})).await;
    assert_eq!(wrong.body["error"]["code"], "digest_mismatch");
    let body = json!({"digest": plain["digest"], "note": "nothing was created"});
    let queued = api.post(&uri, &body).await;
    assert_eq!(queued.status, StatusCode::OK, "{}", queued.body);
    let items = settled(api, &run).await;
    assert_eq!(action(&items, "plain")["state"], "completed");
    assert_eq!(sent(&fixture, "Plain"), 1);
    let again = api.post(&uri, &body).await;
    assert_eq!(again.body["error"]["code"], "action_not_resolvable");
    assert_failed(api, &run).await;
}

#[tokio::test]
async fn a_crash_after_remote_success_never_sends_twice() {
    let fixture = start_from(RECOVERY, false).await;
    let run = finish(&fixture.api, "failure").await;
    crash(&fixture, true).await;
    assert_eq!((sent(&fixture, "Keyed"), sent(&fixture, "Plain")), (1, 1));
    let fixture = fixture.restart().await;
    let api = &fixture.api;
    settled(api, &run).await;
    let keyed = show(api, &run, "keyed").await;
    assert_eq!(keyed["state"], "completed", "{keyed}");
    assert_eq!(keyed["result"], "duplicate of an earlier call");
    let plain = show(api, &run, "plain").await;
    assert_eq!(plain["state"], "unknown", "{plain}");
    let uri = format!("/api/v1/runs/{run}/actions/plain/reconciliation");
    let invalid = api.post(&uri, &json!({"state": "queued"})).await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    let body = json!({"state": "completed", "note": "ticket exists"});
    let reconciled = api.post(&uri, &body).await;
    assert_eq!(reconciled.status, StatusCode::OK, "{}", reconciled.body);
    let plain: ActionRecord = serde_json::from_value(show(api, &run, "plain").await).unwrap();
    let last = plain.history.last().unwrap();
    assert_eq!(last.detail.as_deref(), Some("ticket exists"));
    assert_eq!(plain.state.as_str(), "completed");
    let again = api.post(&uri, &body).await;
    assert_eq!(again.body["error"]["code"], "action_not_resolvable");
    assert_eq!((sent(&fixture, "Keyed"), sent(&fixture, "Plain")), (1, 1));
    assert_failed(api, &run).await;
}

#[tokio::test]
async fn lost_answers_are_replayed_only_with_a_key() {
    let fixture = start_from(LOST_RESPONSES, true).await;
    let api = &fixture.api;
    let run = finish(api, "failure").await;
    let items = settled(api, &run).await;
    let keyed = action(&items, "slow-keyed");
    assert_eq!(keyed["state"], "completed", "{keyed}");
    assert_eq!(keyed["attempts"], 2);
    let slow = action(&items, "slow");
    assert_eq!(slow["state"], "unknown", "{slow}");
    assert_eq!(slow["attempts"], 1);
    assert!(
        slow["error"]
            .as_str()
            .unwrap()
            .starts_with("no answer within 2s")
    );
    assert_eq!(sent(&fixture, "Slow keyed"), 1);
    assert_eq!(
        sent(&fixture, "Slow"),
        2,
        "each call reached the server once"
    );
    assert_failed(api, &run).await;
}
