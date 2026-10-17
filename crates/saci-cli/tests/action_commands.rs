//! `saci actions` against a backend with a dispatcher: listing pending
//! approvals, showing the reviewed payload, approving it by digest,
//! rejecting, and the actions section of `saci report`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use saci_backend::api::{Backend, ClientAuth, Supervisor, issue_token, router};
use saci_backend::store::Store;
use saci_core::config::Config;
use saci_core::ids::RunId;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::process::Command;

const SACI: &str = env!("CARGO_BIN_EXE_saci");

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

struct Idle;

impl Supervisor for Idle {
    fn submitted(&self, _: RunId) {}
    fn cancel_requested(&self, _: RunId) {}
}

fn config(dir: &std::path::Path) -> String {
    let server = saci_testkit::fake_mcp::binary().unwrap();
    format!(
        "llm:\n  base_url: http://m/v1\n  model: m\nenvironments:\n  local: {{}}\n\
         actions:\n  servers:\n    tickets:\n      command: {}\n      env:\n        FAKE_MCP_RECORD: {}\n\
         \x20 definitions:\n    escalate:\n      server: tickets\n      tool: record\n      on: [failed]\n\
         \x20     approval: required\n      arguments:\n        title: 'Escalate {{{{run.id}}}}'\n\
         policies:\n  default:\n    actions: [escalate]\n",
        server.display(),
        dir.join("record.jsonl").display()
    )
}

struct Setup {
    backend: Backend,
    url: String,
    dir: tempfile::TempDir,
}

async fn serve() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let config = Config::from_yaml(&config(dir.path()), "saci.yaml").unwrap();
    let backend = Backend::new(store, config, Arc::new(Idle));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = router(backend.clone(), ClientAuth::Open);
    tokio::spawn(async move { axum::serve(listener, app).await });
    let lookup = Arc::new(|name: &str| std::env::var(name).ok().filter(|_| name == "PATH"));
    tokio::spawn(saci_backend::actions::run_with(backend.clone(), lookup));
    Setup { backend, url, dir }
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

    /// Submits the plan with `saci run` and replays a failed run as its
    /// worker.
    async fn failed_run(&self) -> RunId {
        let plan = self.dir.path().join("plan.md");
        std::fs::write(&plan, PLAN).unwrap();
        let output = self
            .saci(&["run", plan.to_str().unwrap(), "--environment", "local"])
            .await;
        let run = RunId::parse(String::from_utf8_lossy(&output.stdout).trim()).unwrap();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../saci-core/tests/fixtures/runs/failure.json"
        );
        let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut events = fixture["events"].as_array().unwrap().clone();
        for event in &mut events {
            event["run_id"] = json!(run.as_str());
        }
        let token = issue_token(&self.backend, &run).await.unwrap();
        let status = reqwest::Client::new()
            .post(format!("{}/api/v1/worker/runs/{run}/events", self.url))
            .bearer_auth(token)
            .json(&json!({ "events": events }))
            .send()
            .await
            .unwrap()
            .status();
        assert!(status.is_success(), "{status}");
        run
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The digest `saci actions show` printed.
fn digest(shown: &str) -> String {
    shown
        .lines()
        .find_map(|line| line.strip_prefix("Digest: "))
        .unwrap()
        .to_owned()
}

/// Polls `saci report` until it contains `expected`.
async fn report_shows(setup: &Setup, run: &RunId, expected: &str) {
    let mut report = String::new();
    for _ in 0..100 {
        report = text(&setup.saci(&["report", run.as_str()]).await);
        if report.contains(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the report does not show `{expected}`:\n{report}");
}

#[tokio::test]
async fn reviewers_list_show_and_approve_the_exact_payload() {
    let setup = serve().await;
    let run = setup.failed_run().await;
    let list = setup.saci(&["actions", "list"]).await;
    assert_eq!(list.status.code(), Some(0), "{}", text(&list));
    let expected = format!("{run}  escalate: pending approval (tickets/record)");
    assert!(text(&list).contains(&expected), "{}", text(&list));
    let show = ["actions", "show", run.as_str(), "escalate"];
    let shown = text(&setup.saci(&show).await);
    let title = format!("\"title\": \"Escalate {run}\"");
    assert!(shown.contains(&title), "{shown}");
    let digest = digest(&shown);
    let mut approve = vec!["actions", "approve", run.as_str(), "escalate", "--digest"];
    let wrong = setup.saci(&[approve.as_slice(), &["abc"]].concat()).await;
    assert_eq!(wrong.status.code(), Some(2), "{}", text(&wrong));
    assert!(text(&wrong).contains("does not match the recorded arguments"));
    approve.push(&digest);
    let approved = setup.saci(&approve).await;
    assert_eq!(approved.status.code(), Some(0), "{}", text(&approved));
    assert!(text(&approved).contains("escalate: approved, queued"));
    let completed = "escalate: completed (tickets/record): recorded action 1";
    report_shows(&setup, &run, completed).await;
}

#[tokio::test]
async fn rejected_actions_show_as_rejected() {
    let setup = serve().await;
    let run = setup.failed_run().await;
    let note = ["--note", "known issue"];
    let reject = [&["actions", "reject", run.as_str(), "escalate"][..], &note].concat();
    let rejected = setup.saci(&reject).await;
    assert_eq!(rejected.status.code(), Some(0), "{}", text(&rejected));
    assert!(text(&rejected).contains("escalate: rejected (tickets/record)"));
    let show = ["actions", "show", run.as_str(), "escalate"];
    let shown = text(&setup.saci(&show).await);
    let decided = shown.contains("Decision: rejected at") && shown.contains(": known issue");
    assert!(decided, "{shown}");
    let again = setup.saci(&reject).await;
    assert_eq!(again.status.code(), Some(2));
    let refusal = text(&again);
    assert!(
        refusal.contains("not awaiting approval (it is rejected)"),
        "{refusal}"
    );
    let listed = text(&setup.saci(&["actions", "list"]).await);
    assert!(listed.contains("No actions."), "{listed}");
}
