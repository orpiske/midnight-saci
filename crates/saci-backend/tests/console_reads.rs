//! Client reads added for the web console: environments, policies, a
//! run's plan, paged action listings and structured plan diagnostics.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use axum::http::StatusCode;
use common::{TOKEN, TestBackend};
use saci_backend::api::ClientAuth;
use saci_core::event::{EventKind, RunEvent};
use saci_core::ids::RunId;
use saci_core::redaction::Redactor;
use saci_core::status::Outcome;
use serde_json::{Value, json};
use time::OffsetDateTime;

const CONFIG: &str = "llm:
  base_url: http://model:11434/v1
  model: test-model
environments:
  local:
    description: Laptop services
    parameters:
      URL: http://service.internal
    secrets:
      TARGET_TOKEN:
        env: TARGET_TOKEN_SOURCE
    limits:
      step_timeout_secs: 90
actions:
  servers:
    tickets:
      command: saci-fake-mcp
      env:
        TICKET_TOKEN: ${TICKETS_TOKEN}
  secrets:
    TICKETS_TOKEN: { env: SACI_TICKETS_TOKEN }
  definitions:
    escalate:
      server: tickets
      tool: record
      on: [failed]
      approval: required
      arguments:
        title: 'Escalate {{run.id}}'
policies:
  review:
    on_failure: continue
    actions: [escalate]
";

async fn backend() -> TestBackend {
    TestBackend::start_with(ClientAuth::Open, CONFIG).await
}

fn bundle(documents: &[(&str, &str)]) -> Value {
    let documents: BTreeMap<_, _> = documents.iter().copied().collect();
    json!({"entry": "plan.md", "documents": documents})
}

async fn submit(api: &TestBackend, bundle: Value, policy: &str) -> RunId {
    let body = json!({"bundle": bundle, "environment": "local", "policy": policy});
    let answer = api.post("/api/v1/runs", &body).await;
    assert_eq!(answer.status, StatusCode::ACCEPTED, "{}", answer.body);
    RunId::parse(answer.body["id"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn environments_list_limits_without_parameters_or_secrets() {
    let api = backend().await;
    let answer = api.get("/api/v1/environments").await;
    assert_eq!(answer.status, StatusCode::OK);
    let local = &answer.body["items"][0];
    assert_eq!(local["name"], "local");
    assert_eq!(local["description"], "Laptop services");
    assert_eq!(local["limits"]["step_timeout_secs"], 90);
    let text = answer.body.to_string();
    for hidden in ["service.internal", "TARGET_TOKEN", "URL"] {
        assert!(!text.contains(hidden), "{hidden} leaked: {text}");
    }
}

#[tokio::test]
async fn policies_include_default_and_name_their_actions_without_arguments() {
    let api = backend().await;
    let answer = api.get("/api/v1/policies").await;
    assert_eq!(answer.status, StatusCode::OK);
    let items = answer.body["items"].as_array().unwrap();
    assert_eq!(items[0]["name"], "default");
    assert_eq!(items[0]["configured"], false);
    assert_eq!(items[0]["on_failure"], "stop");
    let review = &items[1];
    assert_eq!(review["on_failure"], "continue");
    assert_eq!(review["actions"][0]["name"], "escalate");
    assert_eq!(review["actions"][0]["approval"], "required");
    let text = answer.body.to_string();
    for hidden in ["Escalate", "TICKETS_TOKEN", "saci-fake-mcp"] {
        assert!(!text.contains(hidden), "{hidden} leaked: {text}");
    }
}

#[tokio::test]
async fn console_reads_require_the_client_token() {
    let api = TestBackend::start_with(ClientAuth::Token(TOKEN.into()), CONFIG).await;
    for route in ["/api/v1/environments", "/api/v1/policies"] {
        assert_eq!(api.get(route).await.status, StatusCode::UNAUTHORIZED);
    }
}

const ENTRY: &str = "# Test Plan: Twice

## Phase 1: Main

### Test 1.1: First

Follow [setup](common/setup.md).

**Expected result:**

- One.

### Test 1.2: Second

Follow [setup](common/setup.md).

**Expected result:**

- Two.
";

const SETUP: &str = "# Procedure: Setup

## Phase 1: Main

### Step 1: Prepare

Run `prepare`.

**Expected result:**

- Ready.
";

fn keys(plan: &Value) -> Vec<&str> {
    let units = plan["units"].as_array().unwrap();
    units
        .iter()
        .map(|unit| unit["key"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn the_plan_route_serves_the_retained_bundle_and_distinct_units() {
    let api = backend().await;
    let documents = bundle(&[("plan.md", ENTRY), ("common/setup.md", SETUP)]);
    let run = submit(&api, documents, "default").await;
    let answer = api.get(&format!("/api/v1/runs/{run}/plan")).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let plan = &answer.body;
    assert_eq!(plan["title"], "Twice");
    assert_eq!(plan["documents"]["common/setup.md"], SETUP);
    let expected = [
        "1.1>common/setup.md#1",
        "1.1",
        "1.2>common/setup.md#1",
        "1.2",
    ];
    assert_eq!(keys(plan), expected);
    assert_eq!(plan["units"][2]["invoked_by"], json!(["1.2"]));
    assert_eq!(plan["units"][2]["location"]["document"], "common/setup.md");
    assert_eq!(plan["units"][3]["criteria"][0]["text"], "Two.");
    assert!(plan.get("environment").is_none() && plan.get("llm").is_none());
    let missing = api.get("/api/v1/runs/run-missing/plan").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn invalid_plans_carry_diagnostics_by_document_and_line() {
    let api = backend().await;
    let broken = "# Test Plan: Broken\n\n## Phase 1: Main\n\n### Test 1.1: No criteria\n\nDo it.\n";
    let body = json!({"bundle": bundle(&[("plan.md", broken)]), "environment": "local"});
    let answer = api.post("/api/v1/runs", &body).await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error = &answer.body["error"];
    assert_eq!(error["diagnostics"][0]["document"], "plan.md");
    assert_eq!(error["diagnostics"][0]["line"], 5);
    let detail = error["details"][0].as_str().unwrap();
    let message = error["diagnostics"][0]["message"].as_str().unwrap();
    assert_eq!(detail, format!("plan.md:5: {message}"));
}

#[tokio::test]
async fn unknown_api_routes_answer_a_structured_error() {
    let api = backend().await;
    let answer = api.get("/api/v1/nothing-here").await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.body["error"]["code"], "not_found");
}

async fn fail(api: &TestBackend, run: &RunId) {
    let at = OffsetDateTime::now_utc();
    let event = |seq, kind| RunEvent {
        run_id: run.clone(),
        seq,
        at,
        unit: None,
        attempt: None,
        source: None,
        kind,
    };
    let started = EventKind::RunStarted {
        model: "m".into(),
        environment: None,
    };
    let finished = EventKind::RunFinished {
        outcome: Outcome::Failed,
        reason: "a test failed".into(),
    };
    let events = vec![event(1, started), event(2, finished)];
    api.backend
        .store
        .append_events(run, events, Redactor::new())
        .await
        .unwrap();
}

async fn pages(api: &TestBackend, limit: u32) -> Vec<Vec<String>> {
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut uri = format!("/api/v1/actions?state=pending_approval&limit={limit}");
        if let Some(cursor) = &cursor {
            uri.push_str(&format!("&cursor={cursor}"));
        }
        let answer = api.get(&uri).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
        let page = answer.body["items"].as_array().unwrap();
        pages.push(
            page.iter()
                .map(|item| item["run_id"].as_str().unwrap().to_owned())
                .collect(),
        );
        cursor = answer.body["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            return pages;
        }
    }
}

#[tokio::test]
async fn more_than_two_hundred_pending_actions_stay_reachable() {
    let api = backend().await;
    let simple = "# Test Plan: One\n\n## Phase 1: Main\n\n### Test 1.1: Check\n\nCheck.\n\n**Expected result:**\n\n- Fine.\n";
    let mut runs = Vec::new();
    for _ in 0..205 {
        let run = submit(&api, bundle(&[("plan.md", simple)]), "review").await;
        fail(&api, &run).await;
        runs.push(run.to_string());
    }
    let pages = pages(&api, 200).await;
    assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [200, 5]);
    let listed: Vec<String> = pages.into_iter().flatten().collect();
    assert_eq!(listed.iter().collect::<BTreeSet<_>>().len(), 205);
    assert_eq!(listed, runs, "oldest first, each once");
    let bad = api.get("/api/v1/actions?cursor=nope").await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn small_pages_follow_one_another_without_gaps() {
    let api = backend().await;
    let simple = "# Test Plan: One\n\n## Phase 1: Main\n\n### Test 1.1: Check\n\nCheck.\n\n**Expected result:**\n\n- Fine.\n";
    for _ in 0..5 {
        let run = submit(&api, bundle(&[("plan.md", simple)]), "review").await;
        fail(&api, &run).await;
    }
    let sizes: Vec<_> = pages(&api, 2).await.iter().map(Vec::len).collect();
    assert_eq!(sizes, [2, 2, 1]);
    let single = api.get("/api/v1/actions").await;
    assert!(single.body.get("next_cursor").is_none());
}
