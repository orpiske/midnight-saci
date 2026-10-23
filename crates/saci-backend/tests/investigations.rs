//! Investigations through the REST API: created from the stored report of
//! a finished run, one for each run and unit, and read as the run that
//! executes them reports.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::http::{Method, StatusCode};
use common::TestBackend;
use common::actions::{PLAN, fixture_events};
use saci_backend::api::ClientAuth;
use saci_backend::store::RunRecord;
use saci_core::event::RunEvent;
use saci_core::evidence::EvidenceRecord;
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
    secrets:
      TARGET_TOKEN:
        env: TARGET_TOKEN_SOURCE
  investigator:
    secrets:
      GIT_TOKEN:
        env: GIT_TOKEN_SOURCE
investigations:
  environment: investigator
  image: localhost/saci-investigator:test
  repositories:
    - name: orders
      url: https://example.com/orders.git
      ref: main
    - name: billing
      url: https://example.com/billing.git
      ref: main
  prepare:
    - ls
  instructions: Search for an open bug first.
  limits:
    output_limit_bytes: 8
";

async fn backend() -> TestBackend {
    TestBackend::start_with(ClientAuth::Open, CONFIG).await
}

/// A run that replayed the run fixture `name`.
async fn finished(api: &TestBackend, name: &str) -> RunId {
    let run = api.submit_plan(PLAN).await;
    let events = fixture_events(name, &run);
    api.backend
        .store
        .append_events(&run, events, Redactor::new())
        .await
        .unwrap();
    run
}

async fn investigate(api: &TestBackend, request: &Value) -> common::Answer {
    api.post("/api/v1/investigations", request).await
}

fn id(answer: &common::Answer) -> RunId {
    RunId::parse(answer.body["id"].as_str().unwrap()).unwrap()
}

/// The stored specification holds the failed unit, its cut evidence, and
/// the investigator's environment and limits, never the parent's.
fn check_spec(record: &RunRecord) {
    let spec = record.spec.investigation.as_deref().unwrap();
    assert_eq!(spec.action, "Send `GET $URL/health`.");
    assert_eq!(spec.unit.attempts.len(), 1);
    assert_eq!(spec.evidence[0].id.as_str(), "ev-1");
    let EvidenceRecord::Http(http) = &spec.evidence[0].record else {
        panic!("{:?}", spec.evidence[0].record);
    };
    let body = &http.response.as_ref().unwrap().body;
    assert!(body.truncated && body.text.len() <= 8, "cut to the limit");
    assert_eq!(spec.prepare, ["ls"]);
    assert_eq!(record.spec.environment.name, "investigator");
    assert!(record.spec.environment.secrets.contains_key("GIT_TOKEN"));
    assert!(!record.spec.environment.secrets.contains_key("TARGET_TOKEN"));
    assert_eq!(record.spec.limits.output_limit_bytes, 8);
    let unit = &record.report.units[0].identity.key;
    assert_eq!(unit.as_str(), INVESTIGATION_UNIT);
}

#[tokio::test]
async fn an_investigation_copies_the_failed_unit_and_its_evidence() {
    let api = backend().await;
    let run = finished(&api, "failure").await;
    let request = json!({"run_id": run, "sources": [{"name": "orders", "ref": "v2"}]});
    let answer = investigate(&api, &request).await;
    assert_eq!(answer.status, StatusCode::ACCEPTED, "{}", answer.body);
    let investigation = id(&answer);
    let location = answer.headers["location"].to_str().unwrap();
    assert_eq!(location, format!("/api/v1/investigations/{investigation}"));
    assert_eq!(answer.body["parent_run"], json!(run));
    assert_eq!(answer.body["unit"], "1.1", "the first failed unit");
    assert_eq!(answer.body["lifecycle"], "accepted");
    assert_eq!(answer.body["repositories"][0]["ref"], "v2");
    assert_eq!(answer.body["repositories"].as_array().unwrap().len(), 1);
    let submitted = api.supervisor.submitted.lock().unwrap().last().cloned();
    assert_eq!(submitted, Some(investigation.clone()));
    check_spec(&api.backend.store.run(&investigation).await.unwrap());
    let actions = format!("/api/v1/runs/{investigation}/actions");
    let actions = api.get(&actions).await;
    assert_eq!(actions.body["items"], json!([]), "no outcome actions");

    let shown = format!("/api/v1/investigations/{investigation}");
    assert_eq!(api.get(&shown).await.body, answer.body);
    let listed = api.get(&format!("/api/v1/runs/{run}/investigations")).await;
    assert_eq!(listed.body["items"], json!([answer.body]));
    // Investigations are listed with their run, not as test runs.
    let runs = api.get("/api/v1/runs").await;
    assert_eq!(runs.body["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn one_investigation_exists_for_each_run_and_unit() {
    let api = backend().await;
    let run = finished(&api, "failure").await;
    let first = investigate(&api, &json!({"run_id": run})).await;
    assert_eq!(first.status, StatusCode::ACCEPTED);
    let again = json!({"run_id": run, "unit": "1.1", "sources": [{"name": "billing"}]});
    let second = investigate(&api, &again).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    assert_eq!(id(&second), id(&first));
    assert_eq!(
        api.supervisor.submitted.lock().unwrap().len(),
        2,
        "the run and one investigation"
    );

    // The same idempotency key with another request is a conflict.
    let other = finished(&api, "failure").await;
    let key = [("idempotency-key", "message-1")];
    let body = json!({"run_id": other});
    let keyed = api
        .call(Method::POST, "/api/v1/investigations", Some(&body), &key)
        .await;
    assert_eq!(keyed.status, StatusCode::ACCEPTED);
    let repeated = api
        .call(Method::POST, "/api/v1/investigations", Some(&body), &key)
        .await;
    assert_eq!(id(&repeated), id(&keyed));
    let changed = json!({"run_id": other, "unit": "1.1"});
    let conflict = api
        .call(Method::POST, "/api/v1/investigations", Some(&changed), &key)
        .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{}", conflict.body);
}

#[tokio::test]
async fn runs_and_units_that_cannot_be_investigated_are_rejected() {
    let api = backend().await;
    let run = finished(&api, "failure").await;
    let passed = finished(&api, "success").await;
    let unfinished = api.submit_plan(PLAN).await;
    let investigation = id(&investigate(&api, &json!({"run_id": run})).await);
    let web = json!([{"name": "web"}]);
    let option = json!([{"name": "orders", "ref": "-x"}]);
    for (request, expected) in [
        (json!({"run_id": unfinished}), "has not finished"),
        (json!({"run_id": passed}), "has no unit that failed"),
        (json!({"run_id": run, "unit": "1.2"}), "did not fail"),
        (json!({"run_id": run, "unit": "9.9"}), "has no unit 9.9"),
        (
            json!({"run_id": run, "sources": web}),
            "`web` is not a configured",
        ),
        (
            json!({"run_id": run, "sources": option}),
            "must be a branch",
        ),
        (json!({"run_id": investigation}), "cannot be investigated"),
    ] {
        let answer = investigate(&api, &request).await;
        assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY, "{request}");
        let message = answer.body["error"]["message"].as_str().unwrap();
        assert!(message.contains(expected), "{request}: {message}");
    }
    let submitted = api.supervisor.submitted.lock().unwrap().len();
    assert_eq!(submitted, 4, "three runs, one investigation");
}

#[tokio::test]
async fn unknown_runs_and_investigations_are_not_found() {
    let api = backend().await;
    let missing = investigate(&api, &json!({"run_id": RunId::generate()})).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let run = finished(&api, "failure").await;
    let shown = api.get(&format!("/api/v1/investigations/{run}")).await;
    assert_eq!(
        shown.status,
        StatusCode::NOT_FOUND,
        "a test run is no investigation"
    );

    let unconfigured = TestBackend::start(ClientAuth::Open).await;
    let answer = investigate(&unconfigured, &json!({"run_id": RunId::generate()})).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    let message = answer.body["error"]["message"].as_str().unwrap();
    assert!(message.contains("not configured"), "{message}");
}

/// The events of an investigation that ended in error without a result.
fn failed_investigation(run: &RunId, reason: &str) -> Vec<RunEvent> {
    let unit = INVESTIGATION_UNIT;
    let events = [
        json!({"type": "run_started", "model": "m"}),
        json!({"type": "attempt_started", "unit": unit, "attempt": 1}),
        json!({"type": "attempt_finished", "unit": unit, "attempt": 1, "outcome": "error"}),
        json!({"type": "unit_finished", "unit": unit, "outcome": "error"}),
        json!({"type": "run_finished", "outcome": "error", "reason": reason}),
    ];
    (1..)
        .zip(events)
        .map(|(seq, mut event)| {
            event["run_id"] = json!(run.as_str());
            event["seq"] = json!(seq);
            event["at"] = json!("2026-10-23T10:00:00Z");
            serde_json::from_value(event).unwrap()
        })
        .collect()
}

#[tokio::test]
async fn an_investigation_without_a_result_ends_in_error() {
    let api = backend().await;
    let run = finished(&api, "failure").await;
    let investigation = id(&investigate(&api, &json!({"run_id": run})).await);
    let reason = "the investigation could not conclude: cannot clone `orders`";
    let events = failed_investigation(&investigation, reason);
    let store = &api.backend.store;
    store
        .append_events(&investigation, events, Redactor::new())
        .await
        .unwrap();
    let shown = api
        .get(&format!("/api/v1/investigations/{investigation}"))
        .await;
    assert_eq!(shown.body["lifecycle"], "finished");
    assert_eq!(shown.body["outcome"], "error");
    assert_eq!(shown.body["reason"], reason);
    assert!(shown.body.get("result").is_none());
}

#[tokio::test]
async fn an_investigation_that_ended_in_error_is_replaced() {
    let api = backend().await;
    let run = finished(&api, "failure").await;
    let request = json!({"run_id": run});
    let first = id(&investigate(&api, &request).await);
    let events = failed_investigation(&first, "the worker could not be launched");
    let store = &api.backend.store;
    store
        .append_events(&first, events, Redactor::new())
        .await
        .unwrap();

    let again = investigate(&api, &request).await;
    assert_eq!(again.status, StatusCode::ACCEPTED, "{}", again.body);
    let second = id(&again);
    assert_ne!(second, first, "a new investigation");
    let repeated = investigate(&api, &request).await;
    assert_eq!(repeated.status, StatusCode::OK);
    assert_eq!(id(&repeated), second, "the new one is not replaced");
    let listed = api.get(&format!("/api/v1/runs/{run}/investigations")).await;
    let ids: Vec<&str> = listed.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [first.as_str(), second.as_str()], "oldest first");
}

#[tokio::test]
async fn all_investigations_are_listed_newest_first_with_their_titles() {
    let api = backend().await;
    let first_run = finished(&api, "failure").await;
    let second_run = finished(&api, "failure").await;
    let first = id(&investigate(&api, &json!({"run_id": first_run})).await);
    let second = id(&investigate(&api, &json!({"run_id": second_run})).await);

    let page = api.get("/api/v1/investigations?limit=1").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    let item = &page.body["items"][0];
    assert_eq!(item["id"], json!(second), "newest first");
    assert_eq!(item["plan_title"], "Service health");
    assert_eq!(item["unit_title"], "Check service health");
    let cursor = page.body["next_cursor"].as_str().unwrap();
    let next = api
        .get(&format!("/api/v1/investigations?limit=1&cursor={cursor}"))
        .await;
    assert_eq!(next.body["items"][0]["id"], json!(first));
    assert!(next.body.get("next_cursor").is_none(), "{}", next.body);
    let runs = api.get("/api/v1/runs").await;
    assert_eq!(
        runs.body["items"].as_array().unwrap().len(),
        2,
        "test runs only"
    );
}
