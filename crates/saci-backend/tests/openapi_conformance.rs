//! Responses of the client routes match the schemas in
//! `docs/openapi.yaml`, which the web console's types are generated from.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use axum::http::StatusCode;
use common::{PLAN, TestBackend, submission};
use saci_backend::api::ClientAuth;
use saci_core::assessment::{Assessment, Basis, CriterionResult, CriterionStatus};
use saci_core::event::{EventKind, RunEvent};
use saci_core::evidence::{
    CapturedText, Evidence, EvidenceRecord, HttpEvidence, HttpResponseEvidence, McpEvidence,
    RejectedCall, ShellEvidence, Termination,
};
use saci_core::ids::{EvidenceId, RunId, UnitKey};
use saci_core::redaction::Redactor;
use saci_core::run::AttemptTiming;
use saci_core::status::Outcome;
use serde_json::{Value, json};
use time::OffsetDateTime;

const CONFIG: &str = "llm:
  base_url: http://model:11434/v1
  model: test-model
environments:
  local:
    parameters:
      URL: http://service
actions:
  servers:
    tickets:
      command: saci-fake-mcp
  definitions:
    escalate:
      server: tickets
      tool: record
      on: [cancelled]
      approval: required
policies:
  default:
    actions: [escalate]
";

/// `docs/openapi.yaml`'s schemas as JSON Schema definitions.
fn definitions() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/openapi.yaml");
    let spec: Value = serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let schemas = serde_json::to_string(&spec["components"]["schemas"]).unwrap();
    serde_json::from_str(&schemas.replace("#/components/schemas/", "#/$defs/")).unwrap()
}

fn check(definitions: &Value, schema: &str, instance: &Value) {
    let schema = json!({"$defs": definitions, "$ref": format!("#/$defs/{schema}")});
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{schema}: {errors:#?}\n{instance:#}");
}

fn check_items(definitions: &Value, schema: &str, page: &Value) {
    let items = page["items"].as_array().unwrap();
    assert!(!items.is_empty(), "no {schema} to check");
    for item in items {
        check(definitions, schema, item);
    }
}

fn evidence(index: u64, record: EvidenceRecord) -> Evidence {
    Evidence {
        id: EvidenceId::from_index(index),
        unit: UnitKey::new("1.1"),
        attempt: 1,
        recorded_at: OffsetDateTime::now_utc(),
        duration_ms: 12,
        record,
    }
}

fn records() -> Vec<EvidenceRecord> {
    let mut truncated = CapturedText::complete("x".repeat(64));
    truncated.truncate_to(8);
    vec![
        EvidenceRecord::Shell(ShellEvidence {
            command: "curl -s $URL/health".into(),
            stdout: CapturedText::complete("ok"),
            stderr: truncated,
            exit_code: Some(0),
            termination: Termination::Exited,
            restarted: true,
            session_lost: false,
        }),
        EvidenceRecord::Http(HttpEvidence {
            method: "GET".into(),
            url: "http://service/health".into(),
            request_headers: BTreeMap::from([("accept".into(), "*/*".into())]),
            request_body: None,
            response: Some(HttpResponseEvidence {
                status: 503,
                headers: BTreeMap::from([("content-type".into(), "text/plain".into())]),
                body: CapturedText::complete("unavailable"),
                body_error: None,
            }),
            transport_error: None,
        }),
        EvidenceRecord::Http(HttpEvidence {
            method: "POST".into(),
            url: "http://service/items".into(),
            request_headers: BTreeMap::new(),
            request_body: Some(CapturedText::complete("{}")),
            response: None,
            transport_error: Some("connection refused".into()),
        }),
        EvidenceRecord::Mcp(McpEvidence {
            server: "tickets".into(),
            tool: "lookup".into(),
            arguments: json!({"id": 7}),
            result: CapturedText::complete("found"),
            is_error: false,
            protocol_error: None,
        }),
        EvidenceRecord::Rejected(RejectedCall {
            name: "rm".into(),
            arguments: "{\"path\": \"/\"}".into(),
            reason: "unknown tool".into(),
        }),
    ]
}

fn assessment() -> Assessment {
    Assessment {
        outcome: Outcome::Passed,
        criteria: vec![CriterionResult {
            criterion: 0,
            status: CriterionStatus::Satisfied,
            basis: Basis::Mechanical,
            evidence: vec![EvidenceId::from_index(1)],
            explanation: "exit code 0".into(),
        }],
        explanation: "healthy".into(),
    }
}

fn kinds() -> Vec<(Option<&'static str>, EventKind)> {
    let mut kinds = vec![
        (None, EventKind::RunStarted { model: "m".into() }),
        (Some("1.1"), EventKind::AttemptStarted),
    ];
    for (index, record) in (1..).zip(records()) {
        let evidence = Box::new(evidence(index, record));
        kinds.push((Some("1.1"), EventKind::Observation { evidence }));
    }
    kinds.extend(endings());
    kinds
}

fn endings() -> Vec<(Option<&'static str>, EventKind)> {
    let passed = Outcome::Passed;
    vec![
        (
            Some("1.1"),
            EventKind::AttemptFinished {
                outcome: passed,
                assessment: Some(assessment()),
                error: None,
                timing: Some(AttemptTiming {
                    model_requests: 2,
                    model_ms: 1500,
                    tool_ms: 40,
                    input_tokens: 5200,
                    cached_tokens: Some(4096),
                    output_tokens: 310,
                }),
            },
        ),
        (Some("1.1"), EventKind::UnitFinished { outcome: passed }),
        (None, EventKind::CancellationRequested),
        (
            None,
            EventKind::RunFinished {
                outcome: Outcome::Cancelled,
                reason: "cancelled by a client".into(),
            },
        ),
    ]
}

async fn finished_run(api: &TestBackend) -> RunId {
    let run = api.submit_plan(PLAN).await;
    let events = (1..)
        .zip(kinds())
        .map(|(seq, (unit, kind))| RunEvent {
            run_id: run.clone(),
            seq,
            at: OffsetDateTime::now_utc(),
            unit: unit.map(UnitKey::new),
            attempt: unit.map(|_| 1),
            source: None,
            kind,
        })
        .collect();
    api.backend
        .store
        .append_events(&run, events, Redactor::new())
        .await
        .unwrap();
    run
}

#[tokio::test]
async fn run_responses_match_the_documented_schemas() {
    let api = TestBackend::start_with(ClientAuth::Open, CONFIG).await;
    let defs = definitions();
    let run = finished_run(&api).await;
    let base = format!("/api/v1/runs/{run}");
    check(&defs, "RunSummary", &api.get(&base).await.body);
    check_items(&defs, "RunSummary", &api.get("/api/v1/runs").await.body);
    let report = api.get(&format!("{base}/report")).await.body;
    check(&defs, "RunReport", &report);
    assert_eq!(report["cleanup_not_run"], json!(["1.2"]));
    let events = api.get(&format!("{base}/events")).await.body;
    check(&defs, "EventPage", &events);
    for id in 1..=5 {
        let evidence = api.get(&format!("{base}/evidence/ev-{id}")).await.body;
        check(&defs, "Evidence", &evidence);
    }
    check(
        &defs,
        "RunPlan",
        &api.get(&format!("{base}/plan")).await.body,
    );
}

#[tokio::test]
async fn catalog_action_and_error_responses_match_the_documented_schemas() {
    let api = TestBackend::start_with(ClientAuth::Open, CONFIG).await;
    let defs = definitions();
    let run = finished_run(&api).await;
    check_items(
        &defs,
        "EnvironmentInfo",
        &api.get("/api/v1/environments").await.body,
    );
    check_items(&defs, "PolicyInfo", &api.get("/api/v1/policies").await.body);
    let actions = api.get(&format!("/api/v1/runs/{run}/actions")).await.body;
    check_items(&defs, "Action", &actions);
    check_items(&defs, "Action", &api.get("/api/v1/actions").await.body);
    let action = api
        .get(&format!("/api/v1/runs/{run}/actions/escalate"))
        .await;
    check(&defs, "Action", &action.body);
    let mut invalid = submission("# Test Plan: Broken\n");
    invalid["environment"] = json!("local");
    let answer = api.post("/api/v1/runs", &invalid).await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    check(&defs, "ErrorBody", &answer.body);
    let finished = api
        .post(&format!("/api/v1/runs/{run}/cancellation"), &json!({}))
        .await;
    assert_eq!(finished.status, StatusCode::CONFLICT);
    check(&defs, "ErrorBody", &finished.body);
}
