//! `saci investigate` and `saci investigations show` against a backend:
//! the request with its sources, and the investigation before and after
//! its worker concluded.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Output;
use std::sync::Arc;

use saci_backend::api::{Backend, ClientAuth, Supervisor, router};
use saci_backend::store::Store;
use saci_core::config::Config;
use saci_core::event::RunEvent;
use saci_core::ids::RunId;
use saci_core::investigation::INVESTIGATION_UNIT;
use saci_core::redaction::Redactor;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::process::Command;

const SACI: &str = env!("CARGO_BIN_EXE_saci");
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

const PLAN: &str = "# Test Plan: Service health

## Phase 1: Service availability

### Test 1.1: Check service health

Send `GET /health`.

**Expected result:**

- The status is 200.

### Test 1.2: Reject anonymous writes

Send `POST /items`.

**Expected result:**

- The status is 401.

## Phase 2: Cleanup

### Step 2.1: Remove test data

Delete the test items.

**Expected result:**

- No test items remain.
";

const CONFIG: &str = "llm:\n  base_url: http://m/v1\n  model: m\nenvironments:\n  local: {}\n\
    investigations:\n  environment: local\n  repositories:\n    - name: orders\n      \
    url: https://example.com/orders.git\n      ref: main\n";

struct Idle;

impl Supervisor for Idle {
    fn submitted(&self, _: RunId) {}
    fn cancel_requested(&self, _: RunId) {}
}

struct Setup {
    backend: Backend,
    url: String,
    _dir: tempfile::TempDir,
}

async fn serve() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let config = Config::from_yaml(CONFIG, "saci.yaml").unwrap();
    let backend = Backend::new(store, config, Arc::new(Idle));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = router(backend.clone(), ClientAuth::Open);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Setup {
        backend,
        url,
        _dir: dir,
    }
}

impl Setup {
    async fn saci(&self, args: &[&str]) -> Output {
        Command::new(SACI)
            .arg("--backend")
            .arg(&self.url)
            .args(args)
            .env_remove("SACI_TOKEN")
            .output()
            .await
            .unwrap()
    }

    async fn append(&self, run: &RunId, events: Vec<RunEvent>) {
        let store = &self.backend.store;
        store
            .append_events(run, events, Redactor::new())
            .await
            .unwrap();
    }

    /// A stored run that replayed the failed run fixture.
    async fn failed_run(&self) -> RunId {
        let mut submission = json!({"environment": "local"});
        submission["bundle"] = json!({"entry": "plan.md", "documents": {"plan.md": PLAN}});
        let answer: Value = reqwest::Client::new()
            .post(format!("{}/api/v1/runs", self.url))
            .json(&submission)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let run = RunId::parse(answer["id"].as_str().unwrap()).unwrap();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../saci-core/tests/fixtures/runs/failure.json"
        );
        let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let events = fixture["events"].as_array().unwrap().iter().map(|event| {
            let mut event = event.clone();
            event["run_id"] = json!(run.as_str());
            serde_json::from_value(event).unwrap()
        });
        self.append(&run, events.collect()).await;
        run
    }
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The clone the worker recorded.
const CLONE: &str = r#"{"id": "ev-1", "unit": "investigation", "attempt": 1,
    "recorded_at": "2026-10-23T10:00:00Z", "duration_ms": 5, "tool": "shell",
    "command": "git clone ...", "restarted": false, "session_lost": false,
    "stdout": {"text": "0123456789abcdef0123456789abcdef01234567", "truncated": false, "total_bytes": 40},
    "stderr": {"text": "", "truncated": false, "total_bytes": 0},
    "exit_code": 0, "termination": "exited"}"#;

/// The agent's conclusion.
const CONCLUSION: &str = r#"{"outcome": "concluded",
    "summary": "The health handler reports 503 while the database starts.",
    "sources": [{"name": "orders", "commit": "0123456789abcdef0123456789abcdef01234567"}],
    "hypotheses": [{"statement": "`health` checks the database too early.",
        "confidence": "medium", "evidence": ["ev-1"],
        "code": [{"repository": "orders", "commit": "0123456789abcdef0123456789abcdef01234567",
            "path": "src/health.rs", "start_line": 10, "end_line": 14}]}],
    "no_bug_reason": "Filing is off in this test."}"#;

/// The events of an investigation that cloned `orders` and concluded, as
/// the worker reports them.
fn concluded(run: &RunId) -> Vec<RunEvent> {
    let evidence: Value = serde_json::from_str(CLONE).unwrap();
    let report: Value = serde_json::from_str(CONCLUSION).unwrap();
    let unit = INVESTIGATION_UNIT;
    let events = [
        json!({"type": "run_started", "model": "m"}),
        json!({"type": "attempt_started", "unit": unit, "attempt": 1}),
        json!({"type": "observation", "unit": unit, "attempt": 1, "evidence": evidence}),
        json!({"type": "investigation_concluded", "unit": unit, "attempt": 1, "report": report}),
        json!({"type": "attempt_finished", "unit": unit, "attempt": 1, "outcome": "passed"}),
        json!({"type": "unit_finished", "unit": unit, "outcome": "passed"}),
        json!({"type": "run_finished", "outcome": "passed", "reason": "concluded"}),
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
async fn investigations_are_started_once_for_each_unit() {
    let setup = serve().await;
    let run = setup.failed_run().await;
    let output = setup
        .saci(&["investigate", run.as_str(), "--source", "orders=v2"])
        .await;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let id = RunId::parse(text(&output).trim()).unwrap();
    let note = String::from_utf8_lossy(&output.stderr);
    assert!(
        note.contains(&format!("unit 1.1 of {run} accepted")),
        "{note}"
    );

    let shown = setup.saci(&["investigations", "show", id.as_str()]).await;
    assert_eq!(shown.status.code(), Some(8), "running");
    let shown = text(&shown);
    let header = format!("Investigation {id} of unit 1.1 of {run}: accepted");
    assert!(shown.contains(&header), "{shown}");
    assert!(shown.contains("Source: orders at v2"), "{shown}");

    let again = setup.saci(&["investigate", run.as_str()]).await;
    assert_eq!(
        text(&again).trim(),
        id.as_str(),
        "one investigation for each unit"
    );
    let refused = setup.saci(&["investigate", id.as_str()]).await;
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(error.contains("cannot be investigated"), "{error}");
}

#[tokio::test]
async fn a_concluded_investigation_shows_its_code_and_exits_0() {
    let setup = serve().await;
    let run = setup.failed_run().await;
    let output = setup.saci(&["investigate", run.as_str()]).await;
    let id = RunId::parse(text(&output).trim()).unwrap();
    setup.append(&id, concluded(&id)).await;
    let shown = setup.saci(&["investigations", "show", id.as_str()]).await;
    assert_eq!(shown.status.code(), Some(0));
    let shown = text(&shown);
    assert!(shown.contains(": concluded\n"), "{shown}");
    assert!(
        shown.contains(&format!("Cloned: orders at commit {COMMIT}")),
        "{shown}"
    );
    let code = format!("   orders src/health.rs:10-14 at {COMMIT}");
    assert!(shown.contains(&code), "{shown}");
    assert!(
        shown.contains("No bug filed: Filing is off in this test."),
        "{shown}"
    );
}
