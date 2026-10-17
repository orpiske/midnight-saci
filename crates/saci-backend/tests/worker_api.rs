//! Run-scoped worker routes: credentials, input delivery, event ingestion.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::http::{Method, StatusCode};
use common::{TOKEN, TestBackend};
use saci_backend::api::{ClientAuth, issue_token};
use saci_core::event::{EventKind, RunEvent};
use saci_core::ids::RunId;
use saci_core::status::Outcome;
use serde_json::{Value, json};
use time::OffsetDateTime;

fn event(run: &RunId, seq: u64, kind: EventKind) -> RunEvent {
    RunEvent {
        run_id: run.clone(),
        seq,
        at: OffsetDateTime::now_utc(),
        unit: None,
        attempt: None,
        source: None,
        kind,
    }
}

fn started(run: &RunId, seq: u64) -> RunEvent {
    event(
        run,
        seq,
        EventKind::RunStarted {
            model: "test-model".into(),
        },
    )
}

fn batch(events: &[RunEvent]) -> Value {
    json!({ "events": events })
}

struct Worker {
    run: RunId,
    bearer: String,
}

async fn worker(api: &TestBackend) -> Worker {
    let run = api.submit().await;
    let token = issue_token(&api.backend, &run).await.unwrap();
    Worker {
        bearer: format!("Bearer {token}"),
        run,
    }
}

impl Worker {
    fn uri(&self, route: &str) -> String {
        format!("/api/v1/worker/runs/{}/{route}", self.run)
    }
}

async fn as_bearer(api: &TestBackend, uri: &str, bearer: &str) -> common::Answer {
    api.call(Method::GET, uri, None, &[("authorization", bearer)])
        .await
}

async fn submit_as(api: &TestBackend, bearer: &str) -> RunId {
    let body = common::submission(common::PLAN);
    let answer = api
        .call(
            Method::POST,
            "/api/v1/runs",
            Some(&body),
            &[("authorization", bearer)],
        )
        .await;
    RunId::parse(answer.body["id"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn worker_tokens_grant_one_run_only() {
    let api = TestBackend::start(ClientAuth::Token(TOKEN.into())).await;
    let client = format!("Bearer {TOKEN}");
    let first = submit_as(&api, &client).await;
    let second = submit_as(&api, &client).await;
    let token = issue_token(&api.backend, &first).await.unwrap();
    let token = format!("Bearer {token}");
    let spec = format!("/api/v1/worker/runs/{first}/specification");
    let own = as_bearer(&api, &spec, &token).await;
    assert_eq!(own.status, StatusCode::OK);
    assert_eq!(own.body["run_id"], first.as_str());
    let foreign = format!("/api/v1/worker/runs/{second}/control");
    let other = as_bearer(&api, &foreign, &token).await;
    assert_eq!(other.status, StatusCode::FORBIDDEN);
    assert_eq!(other.body["error"]["code"], "forbidden");
    let control = format!("/api/v1/worker/runs/{first}/control");
    let statuses = [
        as_bearer(&api, &control, &client).await.status,
        as_bearer(&api, "/api/v1/runs", &token).await.status,
    ];
    assert_eq!(statuses, [StatusCode::UNAUTHORIZED; 2]);
    assert_eq!(api.get(&control).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn retransmitted_events_are_stored_once() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let w = worker(&api).await;
    let auth = [("authorization", w.bearer.as_str())];
    let first = vec![
        started(&w.run, 1),
        event(&w.run, 2, EventKind::CancellationRequested),
    ];
    let ack = api
        .call(Method::POST, &w.uri("events"), Some(&batch(&first)), &auth)
        .await;
    assert_eq!(ack.status, StatusCode::OK, "{}", ack.body);
    assert_eq!(ack.body["acknowledged_seq"], 2);
    let again = api
        .call(Method::POST, &w.uri("events"), Some(&batch(&first)), &auth)
        .await;
    assert_eq!(again.body["acknowledged_seq"], 2);
    let (events, _) = api.backend.store.events(&w.run, 0, 50).await.unwrap();
    assert_eq!(events.len(), 2);
    let summary = api.get(&format!("/api/v1/runs/{}", w.run)).await;
    assert_eq!(summary.body["lifecycle"], "stopping");
}

impl Worker {
    async fn post(&self, api: &TestBackend, events: &[RunEvent]) -> common::Answer {
        let auth = [("authorization", self.bearer.as_str())];
        api.call(
            Method::POST,
            &self.uri("events"),
            Some(&batch(events)),
            &auth,
        )
        .await
    }
}

#[tokio::test]
async fn gaps_and_events_after_the_end_are_refused() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let w = worker(&api).await;
    let gap = w.post(&api, &[started(&w.run, 3)]).await;
    assert_eq!(gap.status, StatusCode::CONFLICT);
    assert_eq!(gap.body["error"]["code"], "sequence_gap");
    assert_eq!(gap.body["error"]["details"][0], "acknowledged_seq=0");
    let finished = EventKind::RunFinished {
        outcome: Outcome::Passed,
        reason: "done".into(),
    };
    let events = [started(&w.run, 1), event(&w.run, 2, finished)];
    assert_eq!(w.post(&api, &events).await.status, StatusCode::OK);
    let late = w.post(&api, &[started(&w.run, 3)]).await;
    assert_eq!(late.status, StatusCode::CONFLICT);
    assert_eq!(late.body["error"]["code"], "run_finished");
}

#[tokio::test]
async fn control_reports_cancellation_requests() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let w = worker(&api).await;
    let auth = [("authorization", w.bearer.as_str())];
    let before = api.call(Method::GET, &w.uri("control"), None, &auth).await;
    assert_eq!(before.body["cancel_requested"], false);
    api.post(&format!("/api/v1/runs/{}/cancellation", w.run), &json!({}))
        .await;
    let after = api.call(Method::GET, &w.uri("control"), None, &auth).await;
    assert_eq!(after.body["cancel_requested"], true);
}
