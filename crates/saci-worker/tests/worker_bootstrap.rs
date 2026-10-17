//! The `saci-worker` binary: validating its input before acting, running a
//! fixture-backed plan from a file or from the backend, and (when
//! `SACI_WORKER_IMAGE` names a built image) the same inside the image.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Output;
use std::sync::Arc;

use saci_backend::api::{Backend, ClientAuth, Supervisor, issue_token, router};
use saci_backend::store::Store;
use saci_core::config::Config;
use saci_core::event::{EventKind, RunEvent};
use saci_core::ids::RunId;
use saci_core::plan::PlanBundle;
use saci_core::run::{ReportHeader, RunReport};
use saci_core::spec::{
    EnvironmentSpec, ExecutionLimits, LlmSettings, RunPolicy, RunSpec, SCHEMA_VERSION, SecretRef,
};
use saci_core::status::Outcome;
use saci_testkit::{FakeModel, Reply, tool_calls_reply};
use serde_json::json;
use tokio::process::Command;

const WORKER: &str = env!("CARGO_BIN_EXE_saci-worker");

const PLAN: &str = "# Test Plan: Bootstrap

## Phase 1: Main

### Test 1.1: Leave a mark

Run `touch made-by-saci && echo marked`.

**Expected result:**

- The output is `marked`.
";

/// One shell call, then a passing assessment citing it.
fn script() -> Vec<Reply> {
    let shell = json!({ "command": "touch made-by-saci && echo marked" }).to_string();
    let assessment = json!({
        "outcome": "passed",
        "criteria": [{"criterion": 0, "status": "satisfied", "basis": "mechanical",
                      "evidence": ["ev-1"], "explanation": "printed marked"}],
        "explanation": "the mark was left"
    })
    .to_string();
    vec![
        Reply::ok(&tool_calls_reply(&[("s", "shell", &shell)])),
        Reply::ok(&tool_calls_reply(&[("a", "assess", &assessment)])),
    ]
}

fn spec(run_id: RunId, model_url: String) -> RunSpec {
    RunSpec {
        schema_version: SCHEMA_VERSION,
        run_id,
        created_at: time::OffsetDateTime::now_utc(),
        bundle: PlanBundle {
            entry: "plan.md".into(),
            documents: BTreeMap::from([("plan.md".to_owned(), PLAN.to_owned())]),
        },
        environment: EnvironmentSpec {
            name: "local".into(),
            ..EnvironmentSpec::default()
        },
        policy: RunPolicy::default(),
        limits: ExecutionLimits::default(),
        llm: LlmSettings {
            base_url: model_url,
            model: "scripted".into(),
            api_key_env: None,
            api_key_secret: None,
        },
    }
}

/// Runs the worker with only `PATH` and `vars` in its environment.
async fn worker(args: &[&str], workspace: &Path, vars: &[(&str, &str)]) -> Output {
    Command::new(WORKER)
        .args(args)
        .current_dir(workspace)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .envs(vars.iter().copied())
        .output()
        .await
        .unwrap()
}

/// Writes `spec` and runs it locally; returns the output and its events.
async fn local(spec: &RunSpec, workspace: &Path, vars: &[(&str, &str)]) -> (Output, Vec<RunEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.json");
    std::fs::write(&path, serde_json::to_string(spec).unwrap()).unwrap();
    let output = worker(&["--spec", path.to_str().unwrap()], workspace, vars).await;
    let events = parse(&output.stdout);
    (output, events)
}

fn parse(stdout: &[u8]) -> Vec<RunEvent> {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn fold(spec: &RunSpec, events: &[RunEvent]) -> RunReport {
    let plan = saci_core::bundle::expand(&spec.bundle).unwrap();
    let header = ReportHeader {
        run_id: spec.run_id.clone(),
        plan_title: plan.plan.title.clone(),
        entry: "plan.md".into(),
        environment: "local".into(),
        policy: "default".into(),
        model: "scripted".into(),
        created_at: spec.created_at,
    };
    let mut report = RunReport::new(
        header,
        plan.units.iter().map(|u| u.identity.clone()).collect(),
    );
    for event in events {
        report.apply(event).unwrap();
    }
    report
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The reason of the only event, which must end the run with an error.
fn rejection(events: &[RunEvent]) -> &str {
    match events {
        [
            RunEvent {
                kind:
                    EventKind::RunFinished {
                        outcome: Outcome::Error,
                        reason,
                    },
                ..
            },
        ] => reason,
        other => panic!("expected one run_finished error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_specification_file_runs_and_every_event_is_written() {
    let model = FakeModel::start(script()).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let spec = spec(RunId::generate(), model.base_url());
    let (output, events) = local(&spec, workspace.path(), &[]).await;
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let report = fold(&spec, &events);
    assert_eq!(report.outcome, Some(Outcome::Passed), "{events:?}");
    assert!(workspace.path().join("made-by-saci").exists());
    assert!(stderr(&output).contains("ended passed"));
}

#[tokio::test]
async fn an_unknown_format_is_rejected_before_anything_runs() {
    let model = FakeModel::start(script()).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut spec = spec(RunId::generate(), model.base_url());
    spec.schema_version = 99;
    let (output, events) = local(&spec, workspace.path(), &[]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(rejection(&events).contains("format 99"), "{events:?}");
    assert!(model.requests().is_empty(), "the model was not asked");
    assert!(!workspace.path().join("made-by-saci").exists());
}

#[tokio::test]
async fn a_missing_secret_or_unusable_workspace_is_rejected() {
    let model = FakeModel::start(script()).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut with_secret = spec(RunId::generate(), model.base_url());
    let reference = SecretRef::Env {
        env: "TOKEN_SOURCE".into(),
    };
    with_secret
        .environment
        .secrets
        .insert("TARGET_TOKEN".into(), reference);
    let (output, events) = local(&with_secret, workspace.path(), &[]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(rejection(&events).contains("TARGET_TOKEN"), "{events:?}");
    let missing = workspace.path().join("missing");
    let plain = spec(RunId::generate(), model.base_url());
    let vars = [("SACI_WORKSPACE", missing.to_str().unwrap())];
    let (output, events) = local(&plain, workspace.path(), &vars).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        rejection(&events).contains("not a writable directory"),
        "{events:?}"
    );
    assert!(model.requests().is_empty());
}

#[tokio::test]
async fn without_a_specification_the_launcher_variables_are_required() {
    let workspace = tempfile::tempdir().unwrap();
    let output = worker(&[], workspace.path(), &[]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("SACI_BACKEND_URL is not set"),
        "{}",
        stderr(&output)
    );
}

struct Idle;

impl Supervisor for Idle {
    fn submitted(&self, _: RunId) {}
    fn cancel_requested(&self, _: RunId) {}
}

async fn backend(model_url: &str) -> (String, Backend, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let yaml =
        format!("llm:\n  base_url: {model_url}\n  model: scripted\nenvironments:\n  local: {{}}\n");
    let config = Config::from_yaml(&yaml, "saci.yaml").unwrap();
    let backend = Backend::new(store, config, Arc::new(Idle));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = router(backend.clone(), ClientAuth::Open);
    tokio::spawn(async move { axum::serve(listener, app).await });
    (url, backend, dir)
}

async fn submit(url: &str) -> RunId {
    let body = json!({"bundle": {"entry": "plan.md", "documents": {"plan.md": PLAN}}, "environment": "local"});
    let reply: serde_json::Value = rig::rig_reqwest::reqwest::Client::new()
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

#[tokio::test]
async fn an_assigned_run_is_fetched_executed_and_reported() {
    let model = FakeModel::start(script()).await.unwrap();
    let (url, backend, _dir) = backend(&model.base_url()).await;
    let run = submit(&url).await;
    let token = issue_token(&backend, &run).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let vars = [
        ("SACI_BACKEND_URL", url.as_str()),
        ("SACI_RUN_ID", run.as_str()),
        ("SACI_WORKER_TOKEN", token.as_str()),
    ];
    let output = worker(&[], workspace.path(), &vars).await;
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let record = backend.store.run(&run).await.unwrap();
    assert_eq!(record.report.outcome, Some(Outcome::Passed));
    assert_eq!(record.report.evidence.len(), 1);
    assert!(
        !stderr(&output).contains(&token),
        "the token is never printed"
    );
}

/// Runs `/input/run.json` from `input` in `image` as an arbitrary non-root
/// user (group 0, as on OpenShift).
async fn in_image(image: &str, input: &Path) -> Output {
    let runtime = std::env::var("SACI_CONTAINER_RUNTIME").unwrap_or_else(|_| "docker".into());
    // The image's user is not the test's user: let it read the input.
    std::fs::set_permissions(input, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mount = format!("{}:/input:ro", input.display());
    let args = [
        "run",
        "--rm",
        "--network=host",
        "--user=123456:0",
        "-v",
        &mount,
        image,
    ];
    Command::new(runtime)
        .args(args)
        .args(["--spec", "/input/run.json"])
        .output()
        .await
        .unwrap()
}

/// Runs the fixture plan inside the image named by `SACI_WORKER_IMAGE`, as
/// an arbitrary non-root user, with the model on the host network.
#[tokio::test]
async fn the_image_runs_a_fixture_backed_plan() {
    let Ok(image) = std::env::var("SACI_WORKER_IMAGE") else {
        eprintln!("SACI_WORKER_IMAGE is not set; skipping the image check");
        return;
    };
    let model = FakeModel::bind("127.0.0.1:0", script()).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let spec = spec(RunId::generate(), model.base_url());
    std::fs::write(
        dir.path().join("run.json"),
        serde_json::to_string(&spec).unwrap(),
    )
    .unwrap();
    let output = in_image(&image, dir.path()).await;
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let events = parse(&output.stdout);
    assert_eq!(
        fold(&spec, &events).outcome,
        Some(Outcome::Passed),
        "{events:?}"
    );
}

/// The script, with the first model reply held back by `delay`.
fn delayed_script(delay: std::time::Duration) -> Vec<Reply> {
    let mut replies = script();
    replies[0] = replies[0].clone().delayed(delay);
    replies
}

#[tokio::test]
async fn an_unwritable_events_file_stops_the_run_before_it_acts() {
    let model = FakeModel::start(script()).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.json");
    let spec = spec(RunId::generate(), model.base_url());
    std::fs::write(&path, serde_json::to_string(&spec).unwrap()).unwrap();
    let missing = dir.path().join("missing/events.jsonl");
    let args = [
        "--spec",
        path.to_str().unwrap(),
        "--events",
        missing.to_str().unwrap(),
    ];
    let output = worker(&args, workspace.path(), &[]).await;
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(stderr(&output).contains("cannot write events"));
    assert!(!workspace.path().join("made-by-saci").exists());
    assert!(model.requests().is_empty(), "the model was never asked");
}

/// A backend that assigns `spec` but refuses every event batch.
async fn refusing_backend(spec: RunSpec) -> String {
    use axum::routing::{get, post};
    let app = axum::Router::new()
        .route(
            "/api/v1/worker/runs/{run}/specification",
            get(move || async move { axum::Json(spec.clone()) }),
        )
        .route(
            "/api/v1/worker/runs/{run}/events",
            post(|| async { axum::http::StatusCode::FORBIDDEN }),
        )
        .route(
            "/api/v1/worker/runs/{run}/control",
            get(|| async { axum::Json(json!({"cancel_requested": false})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

#[tokio::test]
async fn refused_events_cancel_the_run_before_its_next_action() {
    let model = FakeModel::start(delayed_script(std::time::Duration::from_secs(3)))
        .await
        .unwrap();
    let run = RunId::generate();
    let url = refusing_backend(spec(run.clone(), model.base_url())).await;
    let workspace = tempfile::tempdir().unwrap();
    let vars = [
        ("SACI_BACKEND_URL", url.as_str()),
        ("SACI_RUN_ID", run.as_str()),
        ("SACI_WORKER_TOKEN", "token"),
    ];
    let output = worker(&[], workspace.path(), &vars).await;
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("event delivery failed; stopping the run"),
        "{}",
        stderr(&output)
    );
    assert!(!workspace.path().join("made-by-saci").exists());
}

/// A run whose only MCP server never answers, with `run_timeout_secs`.
fn hanging_mcp(model_url: String, run_timeout_secs: u64) -> RunSpec {
    use saci_core::spec::{McpServerSpec, McpTransport, StdioServer};
    let mut spec = spec(RunId::generate(), model_url);
    let server = McpServerSpec {
        transport: McpTransport::Stdio(StdioServer {
            command: "sleep".into(),
            args: vec!["60".into()],
            env: BTreeMap::new(),
        }),
        tools: None,
    };
    spec.environment.mcp = BTreeMap::from([("mute".to_owned(), server)]);
    spec.limits.run_timeout_secs = run_timeout_secs;
    spec.limits.step_timeout_secs = run_timeout_secs;
    spec.limits.operation_timeout_secs = 60;
    spec
}

#[tokio::test]
async fn connecting_mcp_servers_stops_at_the_run_deadline() {
    let model = FakeModel::start(script()).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let spec = hanging_mcp(model.base_url(), 1);
    let started = std::time::Instant::now();
    let (output, events) = local(&spec, workspace.path(), &[]).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let reason = rejection(&events);
    assert!(reason.contains("deadline"), "{reason}");
    assert!(model.requests().is_empty());
}

/// Stops `child` as a container runtime would.
fn terminate(child: &tokio::process::Child) {
    let pid = nix::unistd::Pid::from_raw(i32::try_from(child.id().unwrap()).unwrap());
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).unwrap();
}

#[tokio::test]
async fn cancelling_while_mcp_servers_connect_ends_the_run_cancelled() {
    let model = FakeModel::start(script()).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.json");
    let spec = hanging_mcp(model.base_url(), 600);
    std::fs::write(&path, serde_json::to_string(&spec).unwrap()).unwrap();
    let child = Command::new(WORKER)
        .args(["--spec", path.to_str().unwrap()])
        .current_dir(workspace.path())
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    terminate(&child);
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("the worker stops")
        .unwrap();
    let events = parse(&output.stdout);
    let Some(RunEvent {
        kind: EventKind::RunFinished { outcome, reason },
        ..
    }) = events.last()
    else {
        panic!("{events:?}");
    };
    assert_eq!(*outcome, Outcome::Cancelled, "{reason}");
    assert!(reason.contains("MCP servers were connecting"), "{reason}");
}
