//! Outcome action fixtures: a backend whose actions call a stdio
//! `saci-fake-mcp` that records each call, and run fixtures replayed as a
//! worker.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use saci_backend::api::{ClientAuth, issue_token};
use saci_core::event::RunEvent;
use saci_core::ids::RunId;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::TestBackend;

pub const TICKETS_TOKEN: &str = "tickets-token-value";

/// A plan whose units match the run fixtures in `saci-core`.
pub const PLAN: &str = "# Test Plan: Service health

## Phase 1: Service availability

### Test 1.1: Check service health

Send `GET $URL/health`.

**Expected result:**

- The status is 200.

### Test 1.2: Reject anonymous writes

Send `POST $URL/items` without credentials.

**Expected result:**

- The status is 401.

## Phase 2: Cleanup

### Step 2.1: Remove test data

Delete the test items.

**Expected result:**

- No test items remain.
";

/// The configuration in `fixtures/actions.yaml`.
pub const ACTIONS: &str = include_str!("../fixtures/actions.yaml");
/// The configuration in `fixtures/recovery.yaml`.
pub const RECOVERY: &str = include_str!("../fixtures/recovery.yaml");
/// The configuration in `fixtures/lost-responses.yaml`.
pub const LOST_RESPONSES: &str = include_str!("../fixtures/lost-responses.yaml");

/// `template` with the fake server and its record file filled in.
pub fn config(template: &str, server: &Path, record: &Path) -> String {
    template
        .replace("@SERVER@", &server.display().to_string())
        .replace("@RECORD@", &record.display().to_string())
}

pub struct Fixture {
    pub api: TestBackend,
    pub record: PathBuf,
    dispatcher: Option<JoinHandle<()>>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    /// Stops the backend and its dispatcher, and starts them again on the
    /// same database and configuration.
    pub async fn restart(self) -> Self {
        if let Some(dispatcher) = self.dispatcher {
            dispatcher.abort();
            let _ = dispatcher.await;
        }
        let api = self.api.restart().await;
        let dispatcher = Some(dispatch(&api));
        Self {
            api,
            record: self.record,
            dispatcher,
            _dir: self._dir,
        }
    }
}

/// The backend variables the dispatcher reads.
pub fn lookup() -> saci_backend::actions::Lookup {
    Arc::new(|name: &str| match name {
        "SACI_TICKETS_TOKEN" => Some(TICKETS_TOKEN.to_owned()),
        "PATH" => std::env::var("PATH").ok(),
        _ => None,
    })
}

/// Starts a dispatcher for `api`'s backend.
pub fn dispatch(api: &TestBackend) -> JoinHandle<()> {
    tokio::spawn(saci_backend::actions::run_with(
        api.backend.clone(),
        lookup(),
    ))
}

/// A backend with `fixtures/actions.yaml` and its dispatcher.
pub async fn start() -> Fixture {
    start_from(ACTIONS, true).await
}

/// A backend with the configuration `template`, and a dispatcher when
/// `dispatching`.
pub async fn start_from(template: &str, dispatching: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("record.jsonl");
    let server = saci_testkit::fake_mcp::binary().unwrap();
    let api = TestBackend::start_with(ClientAuth::Open, &config(template, &server, &record)).await;
    let dispatcher = dispatching.then(|| dispatch(&api));
    Fixture {
        api,
        record,
        dispatcher,
        _dir: dir,
    }
}

/// The events of the run fixture `name`, as events of `run`.
pub fn fixture_events(name: &str, run: &RunId) -> Vec<RunEvent> {
    let path = format!(
        "{}/../saci-core/tests/fixtures/runs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    fixture["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            let mut event = event.clone();
            event["run_id"] = json!(run.as_str());
            serde_json::from_value(event).unwrap()
        })
        .collect()
}

/// Replays the run fixture `name` as the worker of a new run.
pub async fn finish(api: &TestBackend, name: &str) -> RunId {
    let run = api.submit_plan(PLAN).await;
    let answer = report_events(api, &run, name).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    run
}

/// Sends the events of the run fixture `name` as `run`'s worker.
pub async fn report_events(api: &TestBackend, run: &RunId, name: &str) -> super::Answer {
    let events = fixture_events(name, run);
    let token = issue_token(&api.backend, run).await.unwrap();
    let bearer = format!("Bearer {token}");
    let uri = format!("/api/v1/worker/runs/{run}/events");
    let body = json!({ "events": events });
    api.call(
        Method::POST,
        &uri,
        Some(&body),
        &[("authorization", &bearer)],
    )
    .await
}

pub async fn actions(api: &TestBackend, run: &RunId) -> Value {
    let answer = api.get(&format!("/api/v1/runs/{run}/actions")).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    answer.body["items"].clone()
}

pub fn action<'a>(items: &'a Value, name: &str) -> &'a Value {
    items
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("no action {name} in {items}"))
}

/// Waits until no action of `run` is queued or executing.
pub async fn settled(api: &TestBackend, run: &RunId) -> Value {
    for _ in 0..200 {
        let items = actions(api, run).await;
        let busy = items
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["state"] == "queued" || item["state"] == "executing");
        if !busy {
            return items;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("actions did not settle: {}", actions(api, run).await);
}

pub fn recorded(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
