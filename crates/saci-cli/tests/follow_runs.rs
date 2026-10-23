//! `saci follow`, `status`, `report`, `cancel` and `list` against a backend
//! whose worker the test plays: it posts events through the worker routes,
//! and a proxy between the CLI and the backend can cut connections.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use saci_backend::api::{Backend, ClientAuth, Supervisor, issue_token, router};
use saci_backend::store::Store;
use saci_core::assessment::{Assessment, Basis, CriterionResult, CriterionStatus};
use saci_core::bundle::expand;
use saci_core::config::Config;
use saci_core::event::EventKind;
use saci_core::evidence::{CapturedText, EvidenceRecord, ShellEvidence, Termination};
use saci_core::ids::RunId;
use saci_core::plan::PlanBundle;
use saci_core::redaction::Redactor;
use saci_core::run::{ErrorKind, ExecutionError, UnitIdentity};
use saci_core::status::Outcome;
use saci_worker::events::{EventReceiver, EventSink, Recorder};
use serde_json::json;
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, ChildStdout, Command};
use tokio::task::JoinHandle;

const SACI: &str = env!("CARGO_BIN_EXE_saci");

const PLAN: &str = "# Test Plan: Service health

## Phase 1: Main

### Test 1.1: Check health

Run `curl $URL/health`.

**Expected result:**

- The status is 200.

### Test 1.2: Check version

Run `curl $URL/version`.

**Expected result:**

- The version is printed.
";

struct Idle;

impl Supervisor for Idle {
    fn submitted(&self, _: RunId) {}
    fn cancel_requested(&self, _: RunId) {}
}

/// A TCP proxy whose open connections can be cut.
struct Proxy {
    url: String,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Proxy {
    async fn start(upstream: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let connections = Arc::new(Mutex::new(Vec::new()));
        let open = Arc::clone(&connections);
        tokio::spawn(async move {
            loop {
                let (mut inbound, _) = listener.accept().await.unwrap();
                let upstream = upstream.clone();
                let task = tokio::spawn(async move {
                    let mut outbound = TcpStream::connect(&upstream).await.unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                });
                open.lock().unwrap().push(task);
            }
        });
        Self { url, connections }
    }

    fn cut(&self) {
        for task in self.connections.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

/// Serves a backend on a local port; returns it with its address.
async fn serve(dir: &std::path::Path) -> (Backend, String) {
    let store = Store::open(dir).await.unwrap();
    let yaml = "llm:\n  base_url: http://m/v1\n  model: m\nenvironments:\n  local: {}\n";
    let config = Config::from_yaml(yaml, "saci.yaml").unwrap();
    let backend = Backend::new(store, config, Arc::new(Idle));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let app = router(backend.clone(), ClientAuth::Open);
    tokio::spawn(async move { axum::serve(listener, app).await });
    (backend, address)
}

fn units() -> Vec<UnitIdentity> {
    let bundle = PlanBundle {
        entry: "plan.md".into(),
        documents: [("plan.md".to_owned(), PLAN.to_owned())].into(),
    };
    let plan = expand(&bundle).unwrap();
    plan.units.into_iter().map(|unit| unit.identity).collect()
}

/// A backend with one submitted run, and the test as that run's worker.
struct Harness {
    proxy: Proxy,
    run: RunId,
    token: String,
    units: Vec<UnitIdentity>,
    recorder: Recorder,
    events: Mutex<EventReceiver>,
    backend_url: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (backend, backend_url) = serve(dir.path()).await;
        let proxy = Proxy::start(backend_url.clone()).await;
        let run = submit(&proxy.url).await;
        let token = issue_token(&backend, &run).await.unwrap();
        let (sink, events) = EventSink::new(run.clone());
        Self {
            proxy,
            run,
            token,
            units: units(),
            recorder: Recorder::new(Arc::new(sink), Redactor::new()),
            events: Mutex::new(events),
            backend_url: format!("http://{backend_url}"),
            _dir: dir,
        }
    }

    fn emit(&self, unit: Option<usize>, attempt: Option<u32>, kind: EventKind) {
        let unit = unit.map(|index| &self.units[index]);
        self.recorder.events().emit(unit, attempt, kind);
    }

    /// Records a passing shell observation for attempt 1 of `unit`.
    fn observe(&self, unit: usize) -> saci_core::ids::EvidenceId {
        let shell = ShellEvidence {
            command: "curl $URL/health".into(),
            stdout: CapturedText {
                text: "ok\n".into(),
                ..CapturedText::default()
            },
            stderr: CapturedText::default(),
            exit_code: Some(0),
            termination: Termination::Exited,
            restarted: false,
            session_lost: false,
        };
        let record = EvidenceRecord::Shell(shell);
        self.recorder
            .record(&self.units[unit], 1, Duration::from_millis(12), record)
            .id
    }

    /// Unit `unit` runs one attempt that ends with `outcome`.
    fn unit(&self, unit: usize, outcome: Outcome) {
        self.emit(Some(unit), Some(1), EventKind::AttemptStarted);
        let evidence = self.observe(unit);
        let finished = EventKind::AttemptFinished {
            outcome,
            assessment: Some(assessment(outcome, evidence)),
            error: None,
            timing: None,
        };
        self.emit(Some(unit), Some(1), finished);
        self.emit(Some(unit), None, EventKind::UnitFinished { outcome });
    }

    /// Unit `unit`'s attempt ends with `outcome` and no assessment.
    fn ended(&self, unit: usize, outcome: Outcome, error: Option<ExecutionError>) {
        let attempt = EventKind::AttemptFinished {
            outcome,
            assessment: None,
            error,
            timing: None,
        };
        self.emit(Some(unit), Some(1), attempt);
        self.emit(Some(unit), None, EventKind::UnitFinished { outcome });
    }

    fn finish(&self, outcome: Outcome, reason: &str) {
        let reason = reason.to_owned();
        self.emit(None, None, EventKind::RunFinished { outcome, reason });
    }

    /// Posts every event emitted so far, as the worker would.
    async fn deliver(&self) {
        let mut batch = Vec::new();
        while let Ok(event) = self.events.lock().unwrap().try_recv() {
            batch.push(event);
        }
        let url = format!(
            "{}/api/v1/worker/runs/{}/events",
            self.backend_url, self.run
        );
        let response = reqwest::Client::new()
            .post(url)
            .bearer_auth(&self.token)
            .json(&json!({ "events": batch }))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{}",
            response.text().await.unwrap()
        );
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(SACI);
        command.arg("--backend").arg(&self.proxy.url).args(args);
        command.env_remove("SACI_TOKEN").kill_on_drop(true);
        command
    }

    async fn saci(&self, args: &[&str]) -> Output {
        self.command(args).output().await.unwrap()
    }

    fn child(&self, args: &[&str]) -> Child {
        let mut command = self.command(args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        command.spawn().unwrap()
    }

    fn spawn(&self, args: &[&str]) -> (Child, tokio::io::Lines<BufReader<ChildStdout>>) {
        let mut child = self
            .command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        (child, lines)
    }
}

fn assessment(outcome: Outcome, evidence: saci_core::ids::EvidenceId) -> Assessment {
    let status = if outcome == Outcome::Passed {
        CriterionStatus::Satisfied
    } else {
        CriterionStatus::Contradicted
    };
    let criterion = CriterionResult {
        criterion: 0,
        status,
        basis: Basis::Mechanical,
        evidence: vec![evidence],
        explanation: "checked the exit code".into(),
    };
    Assessment {
        outcome,
        criteria: vec![criterion],
        explanation: format!("the check {outcome}"),
    }
}

async fn submit(url: &str) -> RunId {
    let body = json!({"bundle": {"entry": "plan.md", "documents": {"plan.md": PLAN}}, "environment": "local"});
    let reply: serde_json::Value = reqwest::Client::new()
        .post(format!("{url}/api/v1/runs"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    RunId::parse(reply["id"].as_str().unwrap()).unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The sequence number a progress line starts with.
fn seq(line: &str) -> u64 {
    line.strip_prefix('[')
        .and_then(|rest| rest.split(']').next())
        .unwrap()
        .parse()
        .unwrap()
}

/// The first `count` progress lines.
async fn first(lines: &mut tokio::io::Lines<BufReader<ChildStdout>>, count: usize) -> Vec<String> {
    let mut seen = Vec::new();
    while seen.len() < count {
        let line = tokio::time::timeout(Duration::from_secs(20), lines.next_line()).await;
        seen.push(
            line.expect("a progress line")
                .unwrap()
                .expect("more output"),
        );
    }
    seen
}

#[tokio::test]
async fn follow_survives_lost_connections_without_gaps_or_repeats() {
    let harness = Harness::start().await;
    harness.emit(None, None, started());
    harness.unit(0, Outcome::Passed);
    harness.deliver().await;
    let run = harness.run.to_string();
    let (child, mut lines) = harness.spawn(&["follow", &run]);
    let mut seen = first(&mut lines, 5).await;
    harness.proxy.cut();
    harness.unit(1, Outcome::Failed);
    harness.finish(Outcome::Failed, "1.2 failed");
    harness.deliver().await;
    while let Some(line) = lines.next_line().await.unwrap() {
        seen.push(line);
    }
    let output = child.wait_with_output().await.unwrap();
    assert_eq!(output.status.code(), Some(1), "a failed run exits 1");
    let numbers: Vec<u64> = seen.iter().map(|line| seq(line)).collect();
    assert_eq!(numbers, (1..=10).collect::<Vec<_>>(), "{seen:#?}");
    let observation = &seen[2];
    assert!(
        observation.contains("ev-1 shell `curl $URL/health` exited 0"),
        "{observation}"
    );
    assert!(seen[9].contains("run finished failed: 1.2 failed"));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("reconnecting after event"), "{stderr}");
    assert!(
        stderr.contains("the request to http://"),
        "the reason: {stderr}"
    );
}

#[tokio::test]
async fn follow_resumes_after_a_given_event_and_prints_json_lines() {
    let harness = Harness::start().await;
    harness.emit(None, None, started());
    harness.unit(0, Outcome::Passed);
    harness.unit(1, Outcome::Passed);
    harness.finish(Outcome::Passed, "all units passed");
    harness.deliver().await;
    let output = harness
        .saci(&["follow", harness.run.as_str(), "--after", "7", "--json"])
        .await;
    assert_eq!(output.status.code(), Some(0));
    let seqs: Vec<u64> = text(&output.stdout)
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["seq"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert_eq!(seqs, [8, 9, 10]);
}

#[tokio::test]
async fn status_shows_a_running_run_and_exits_8() {
    let harness = Harness::start().await;
    harness.emit(None, None, started());
    harness.unit(0, Outcome::Passed);
    harness.emit(Some(1), Some(1), EventKind::AttemptStarted);
    harness.deliver().await;
    let status = harness.saci(&["status", harness.run.as_str()]).await;
    assert_eq!(status.status.code(), Some(8), "still running");
    let shown = text(&status.stdout);
    let expected = "running (unit 1.2), 1 of 2 units done: 1 passed";
    assert!(shown.contains(expected), "{shown}");
    let report = harness.saci(&["report", harness.run.as_str()]).await;
    assert_eq!(report.status.code(), Some(8));
    assert!(text(&report.stdout).contains("Not finished: running"));
}

#[tokio::test]
async fn the_report_shows_units_criteria_evidence_and_errors() {
    let harness = Harness::start().await;
    harness.emit(None, None, started());
    harness.unit(0, Outcome::Passed);
    harness.emit(Some(1), Some(1), EventKind::AttemptStarted);
    let error = ExecutionError::new(ErrorKind::Provider, "the provider rejected the API key");
    harness.ended(1, Outcome::Error, Some(error));
    harness.finish(Outcome::Error, "1.2 ended error");
    harness.deliver().await;
    let report = harness.saci(&["report", harness.run.as_str()]).await;
    assert_eq!(report.status.code(), Some(6), "an error run exits 6");
    let shown = text(&report.stdout);
    for expected in [
        "Outcome: error: 1.2 ended error",
        "1.1 Check health [test]: passed",
        "expected result 1: satisfied (mechanical; ev-1): checked the exit code",
        "ev-1 shell `curl $URL/health` exited 0 (12 ms)",
        "attempt 1: error: provider: the provider rejected the API key",
    ] {
        assert!(
            shown.contains(expected),
            "missing {expected:?} in:\n{shown}"
        );
    }
    let json = harness
        .saci(&["status", harness.run.as_str(), "--json"])
        .await;
    let summary: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(summary["outcome"], "error");
}

impl Harness {
    /// The worker stops unit 1.1 and skips 1.2, as after cancellation.
    fn cancelled(&self) {
        self.ended(0, Outcome::Cancelled, None);
        let reason = "not attempted: the run was cancelled".to_owned();
        self.emit(Some(1), None, EventKind::UnitSkipped { reason });
        self.finish(Outcome::Cancelled, "the run was cancelled");
    }

    /// A run whose first unit is executing.
    async fn running() -> Self {
        let harness = Self::start().await;
        harness.emit(None, None, started());
        harness.emit(Some(0), Some(1), EventKind::AttemptStarted);
        harness.deliver().await;
        harness
    }
}

#[tokio::test]
async fn a_cancellation_request_is_shown_as_pending() {
    let harness = Harness::running().await;
    let run = harness.run.to_string();
    let requested = harness.saci(&["cancel", &run]).await;
    assert_eq!(
        requested.status.code(),
        Some(0),
        "{}",
        text(&requested.stderr)
    );
    let printed = text(&requested.stdout);
    assert!(
        printed.contains("Cancellation requested") && printed.contains("running"),
        "{printed}"
    );
    let status = harness.saci(&["status", &run]).await;
    assert_eq!(status.status.code(), Some(8));
    assert!(
        text(&status.stdout).contains(", pending"),
        "{}",
        text(&status.stdout)
    );
}

#[tokio::test]
async fn cancel_wait_returns_once_cancellation_completed() {
    let harness = Harness::running().await;
    let run = harness.run.to_string();
    let waiting = harness.child(&["cancel", &run, "--wait"]);
    tokio::time::sleep(Duration::from_millis(300)).await;
    harness.cancelled();
    harness.deliver().await;
    let output = waiting.wait_with_output().await.unwrap();
    assert_eq!(output.status.code(), Some(7), "{}", text(&output.stderr));
    let printed = text(&output.stdout);
    assert!(
        printed.contains(", completed") && printed.contains("outcome cancelled"),
        "{printed}"
    );
    let again = harness.saci(&["cancel", &run]).await;
    assert_eq!(again.status.code(), Some(2));
    assert!(
        text(&again.stderr).contains("already finished"),
        "{}",
        text(&again.stderr)
    );
}

#[tokio::test]
async fn unknown_runs_and_listing() {
    let harness = Harness::start().await;
    let missing = harness.saci(&["status", "run-doesnotexist"]).await;
    assert_eq!(missing.status.code(), Some(2));
    let malformed = harness.saci(&["report", "not-a-run"]).await;
    assert_eq!(malformed.status.code(), Some(2), "clap rejects it");
    let list = harness.saci(&["list"]).await;
    assert_eq!(list.status.code(), Some(0));
    let listed = text(&list.stdout);
    assert!(
        listed.contains(harness.run.as_str()) && listed.contains("Service health"),
        "{listed}"
    );
}

#[tokio::test]
async fn following_a_finished_run_prints_every_page() {
    let harness = Harness::start().await;
    harness.emit(None, None, started());
    harness.emit(Some(0), Some(1), EventKind::AttemptStarted);
    // More observations than one page of events holds.
    for _ in 0..70 {
        harness.observe(0);
    }
    let evidence = harness.observe(0);
    let finished = EventKind::AttemptFinished {
        outcome: Outcome::Passed,
        assessment: Some(assessment(Outcome::Passed, evidence)),
        error: None,
        timing: None,
    };
    harness.emit(Some(0), Some(1), finished);
    let passed = Outcome::Passed;
    harness.emit(Some(0), None, EventKind::UnitFinished { outcome: passed });
    harness.unit(1, Outcome::Failed);
    harness.finish(Outcome::Failed, "1.2 failed");
    harness.deliver().await;
    let output = harness.saci(&["follow", &harness.run.to_string()]).await;
    let printed: Vec<u64> = text(&output.stdout).lines().map(seq).collect();
    let last = *printed.last().unwrap();
    assert_eq!(
        printed,
        (1..=last).collect::<Vec<_>>(),
        "{}",
        text(&output.stderr)
    );
    assert!(last > 70, "{last}");
}

fn started() -> EventKind {
    EventKind::RunStarted {
        model: "m".into(),
        environment: None,
    }
}
