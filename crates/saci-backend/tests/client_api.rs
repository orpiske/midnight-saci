//! The client-facing REST API against `docs/openapi.yaml`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use common::{PLAN, TOKEN, TestBackend, submission};
use saci_backend::api::ClientAuth;
use saci_core::event::{EventKind, RunEvent};
use saci_core::evidence::{CapturedText, Evidence, EvidenceRecord, ShellEvidence, Termination};
use saci_core::ids::{EvidenceId, RunId, UnitKey};
use saci_core::redaction::Redactor;
use saci_core::spec::SecretRef;
use saci_core::status::Outcome;
use serde_json::json;
use time::OffsetDateTime;

fn event(run: &RunId, seq: u64, unit: Option<&str>, kind: EventKind) -> RunEvent {
    RunEvent {
        run_id: run.clone(),
        seq,
        at: OffsetDateTime::now_utc(),
        unit: unit.map(UnitKey::new),
        attempt: unit.map(|_| 1),
        source: None,
        kind,
    }
}

fn observation(run: &RunId, seq: u64) -> RunEvent {
    let evidence = Evidence {
        id: EvidenceId::from_index(1),
        unit: UnitKey::new("1.1"),
        attempt: 1,
        recorded_at: OffsetDateTime::now_utc(),
        duration_ms: 5,
        record: EvidenceRecord::Shell(ShellEvidence {
            command: "curl $URL/health".into(),
            stdout: CapturedText::complete("200"),
            stderr: CapturedText::default(),
            exit_code: Some(0),
            termination: Termination::Exited,
            restarted: false,
            session_lost: false,
        }),
    };
    let kind = EventKind::Observation {
        evidence: Box::new(evidence),
    };
    event(run, seq, Some("1.1"), kind)
}

async fn start_run(api: &TestBackend, run: &RunId) {
    let events = vec![
        event(
            run,
            1,
            None,
            EventKind::RunStarted {
                model: "test-model".into(),
            },
        ),
        event(run, 2, Some("1.1"), EventKind::AttemptStarted),
    ];
    api.backend
        .store
        .append_events(run, events, Redactor::new())
        .await
        .unwrap();
}

async fn follow_while_delivering(api: &TestBackend, run: &RunId) -> common::Answer {
    let uri = format!("/api/v1/runs/{run}/events?after=2&wait=10");
    let started = Instant::now();
    let follow = api.get(&uri);
    let deliver = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let batch = vec![observation(run, 3)];
        api.backend
            .store
            .append_events(run, batch, Redactor::new())
            .await
            .unwrap();
        api.backend.notify();
    };
    let (answer, ()) = tokio::join!(follow, deliver);
    assert!(started.elapsed() < Duration::from_secs(5));
    answer
}

#[tokio::test]
async fn tokens_are_required_when_configured() {
    let api = TestBackend::start(ClientAuth::Token(TOKEN.into())).await;
    assert_eq!(api.get("/api/v1/health").await.status, StatusCode::OK);
    let anonymous = api.get("/api/v1/runs").await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.body["error"]["code"], "unauthorized");
    let wrong = api
        .call(
            Method::GET,
            "/api/v1/runs",
            None,
            &[("authorization", "Bearer nope")],
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let bearer = format!("Bearer {TOKEN}");
    let allowed = api
        .call(
            Method::GET,
            "/api/v1/runs",
            None,
            &[("authorization", &bearer)],
        )
        .await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.body);
}

#[tokio::test]
async fn a_valid_submission_is_stored_and_handed_to_the_supervisor() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let answer = api.post("/api/v1/runs", &submission(PLAN)).await;
    assert_eq!(answer.status, StatusCode::ACCEPTED, "{}", answer.body);
    let id = answer.body["id"].as_str().unwrap().to_owned();
    assert_eq!(answer.headers["location"], format!("/api/v1/runs/{id}"));
    assert_eq!(answer.body["lifecycle"], "accepted");
    assert_eq!(answer.body["total_units"], 2);
    assert_eq!(answer.body["plan_title"], "Service health");
    let run = RunId::parse(&id).unwrap();
    assert_eq!(
        *api.supervisor.submitted.lock().unwrap(),
        std::slice::from_ref(&run)
    );
    let stored = api.backend.store.run(&run).await.unwrap();
    let secret = &stored.spec.environment.secrets["TARGET_TOKEN"];
    assert_eq!(
        *secret,
        SecretRef::Env {
            env: "TARGET_TOKEN_SOURCE".into()
        }
    );
    assert_eq!(
        api.get(&format!("/api/v1/runs/{id}")).await.body["id"],
        id.as_str()
    );
    let report = api.get(&format!("/api/v1/runs/{id}/report")).await;
    assert_eq!(report.body["units"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn invalid_input_is_rejected_before_anything_is_stored() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let invalid = api
        .post("/api/v1/runs", &submission("# Test Plan: Empty\n"))
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid.body["error"]["code"], "validation_failed");
    assert!(
        invalid.body["error"]["details"][0]
            .as_str()
            .unwrap()
            .starts_with("plan.md:")
    );
    assert!(api.supervisor.submitted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_names_and_fields_are_rejected() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let mut unknown = submission(PLAN);
    unknown["environment"] = json!("staging");
    let answer = api.post("/api/v1/runs", &unknown).await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        answer.body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("staging")
    );
    let mut extra = submission(PLAN);
    extra["approve"] = json!(true);
    assert_eq!(
        api.post("/api/v1/runs", &extra).await.status,
        StatusCode::BAD_REQUEST
    );
    assert!(
        api.get("/api/v1/runs").await.body["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(api.supervisor.submitted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn idempotent_submission_returns_the_original_run() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let key = [("idempotency-key", "retry-1")];
    let body = submission(PLAN);
    let first = api
        .call(Method::POST, "/api/v1/runs", Some(&body), &key)
        .await;
    assert_eq!(first.status, StatusCode::ACCEPTED);
    let again = api
        .call(Method::POST, "/api/v1/runs", Some(&body), &key)
        .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body["id"], first.body["id"]);
    let mut changed = body.clone();
    changed["policy"] = json!("default");
    let conflict = api
        .call(Method::POST, "/api/v1/runs", Some(&changed), &key)
        .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT);
    assert_eq!(conflict.body["error"]["code"], "idempotency_conflict");
    assert_eq!(api.supervisor.submitted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn runs_are_listed_newest_first_in_pages() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(api.submit().await.to_string());
    }
    let first = api.get("/api/v1/runs?limit=2").await;
    let items: Vec<_> = first.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(items, [ids[2].clone(), ids[1].clone()]);
    let cursor = first.body["next_cursor"].as_str().unwrap();
    let second = api
        .get(&format!("/api/v1/runs?limit=2&cursor={cursor}"))
        .await;
    assert_eq!(second.body["items"][0]["id"], ids[0].as_str());
    assert!(second.body.get("next_cursor").is_none());
    assert_eq!(
        api.get("/api/v1/runs?limit=0").await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        api.get("/api/v1/runs?cursor=x").await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn progress_and_evidence_can_be_followed_and_resumed() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let run = api.submit().await;
    start_run(&api, &run).await;
    let page = api.get(&format!("/api/v1/runs/{run}/events?after=0")).await;
    assert_eq!(page.body["items"].as_array().unwrap().len(), 2);
    assert_eq!(page.body["next_after"], 2);
    assert_eq!(page.body["finished"], false);
    let waiting = follow_while_delivering(&api, &run).await;
    assert_eq!(waiting.body["items"][0]["type"], "observation");
    assert_eq!(waiting.body["next_after"], 3);
    assert_eq!(
        api.get(&format!("/api/v1/runs/{run}/events?wait=61"))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn evidence_and_the_current_unit_are_readable() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let run = api.submit().await;
    start_run(&api, &run).await;
    let batch = vec![observation(&run, 3)];
    api.backend
        .store
        .append_events(&run, batch, Redactor::new())
        .await
        .unwrap();
    let evidence = api.get(&format!("/api/v1/runs/{run}/evidence/ev-1")).await;
    assert_eq!(evidence.body["tool"], "shell");
    assert_eq!(
        api.get(&format!("/api/v1/runs/{run}/evidence/ev-2"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let summary = api.get(&format!("/api/v1/runs/{run}")).await;
    assert_eq!(summary.body["current_unit"], "1.1");
}

#[tokio::test]
async fn cancellation_is_idempotent_and_refused_after_the_run_finished() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let run = api.submit().await;
    let uri = format!("/api/v1/runs/{run}/cancellation");
    let first = api.post(&uri, &json!({})).await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{}", first.body);
    let requested = first.body["cancel_requested_at"].clone();
    assert!(requested.is_string());
    let again = api.post(&uri, &json!({})).await;
    assert_eq!(again.body["cancel_requested_at"], requested);
    assert_eq!(api.supervisor.cancelled.lock().unwrap().len(), 2);
    api.backend
        .store
        .update_report(&run, |report| {
            report.finish(
                Outcome::Cancelled,
                "cancelled by the user",
                OffsetDateTime::now_utc(),
            );
            Ok(())
        })
        .await
        .unwrap();
    let finished = api.post(&uri, &json!({})).await;
    assert_eq!(finished.status, StatusCode::CONFLICT);
    assert_eq!(finished.body["error"]["code"], "run_finished");
}

#[tokio::test]
async fn unknown_and_malformed_requests_get_structured_errors() {
    let api = TestBackend::start(ClientAuth::Open).await;
    let missing = api.get("/api/v1/runs/run-doesnotexist").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.body["error"]["code"], "not_found");
    assert_eq!(
        api.get("/api/v1/runs/../etc").await.status,
        StatusCode::NOT_FOUND
    );
    let malformed = api.call(Method::POST, "/api/v1/runs", None, &[]).await;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert_eq!(malformed.body["error"]["code"], "bad_request");
}
