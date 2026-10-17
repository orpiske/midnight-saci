//! MCP tools in whole runs of the `saci-worker` binary, against the
//! controlled `saci-fake-mcp` server: registration and naming, calls
//! recorded as evidence under the run's redaction and limits, tool errors
//! kept apart from protocol errors, servers that cannot start, and runs
//! without MCP.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::process::Output;

use saci_core::event::{EventKind, RunEvent};
use saci_core::evidence::{EvidenceRecord, McpEvidence};
use saci_core::ids::RunId;
use saci_core::plan::PlanBundle;
use saci_core::run::{ReportHeader, RunReport};
use saci_core::spec::{
    EnvironmentSpec, ExecutionLimits, LlmSettings, McpServerSpec, McpTransport, RunPolicy, RunSpec,
    SCHEMA_VERSION, SecretRef, StdioServer,
};
use saci_core::status::Outcome;
use saci_testkit::{FakeModel, Reply, fake_mcp, tool_calls_reply};
use serde_json::{Value, json};
use tokio::process::Command;

const WORKER: &str = env!("CARGO_BIN_EXE_saci-worker");
const SECRET: &str = "mcp-token-value";

const PLAN: &str = "# Test Plan: Through MCP

## Phase 1: Main

### Test 1.1: Echo through the server

Use the fake server's `echo` tool with the text `hello`.

**Expected result:**

- The tool returns `echo: hello`.
";

/// An environment enabling the fake server, started as `command`.
fn environment(command: &str) -> EnvironmentSpec {
    let server = McpServerSpec {
        transport: McpTransport::Stdio(StdioServer {
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::from([("TOKEN".into(), "${MCP_TOKEN}".into())]),
        }),
        tools: None,
    };
    EnvironmentSpec {
        name: "local".into(),
        secrets: BTreeMap::from([("MCP_TOKEN".into(), SecretRef::Env { env: "X".into() })]),
        mcp: BTreeMap::from([("fake".into(), server)]),
        ..EnvironmentSpec::default()
    }
}

fn spec(model_url: String, command: &str) -> RunSpec {
    let environment = environment(command);
    RunSpec {
        schema_version: SCHEMA_VERSION,
        run_id: RunId::generate(),
        created_at: time::OffsetDateTime::now_utc(),
        bundle: PlanBundle {
            entry: "plan.md".into(),
            documents: BTreeMap::from([("plan.md".to_owned(), PLAN.to_owned())]),
        },
        environment,
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

/// Runs `spec` with the fake server on `PATH`; returns the output and events.
async fn run(spec: &RunSpec) -> (Output, Vec<RunEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.json");
    std::fs::write(&path, serde_json::to_string(spec).unwrap()).unwrap();
    let binary = fake_mcp::binary().unwrap();
    let search = format!(
        "{}:{}",
        binary.parent().unwrap().display(),
        std::env::var("PATH").unwrap()
    );
    let output = Command::new(WORKER)
        .args([
            "--spec",
            path.to_str().unwrap(),
            "--workspace",
            dir.path().to_str().unwrap(),
        ])
        .env_clear()
        .env("PATH", search)
        .env("SACI_SECRET_MCP_TOKEN", SECRET)
        .output()
        .await
        .unwrap();
    let events = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (output, events)
}

fn report(spec: &RunSpec, events: &[RunEvent]) -> RunReport {
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
    let units = plan.units.iter().map(|u| u.identity.clone()).collect();
    let mut report = RunReport::new(header, units);
    for event in events {
        report.apply(event).unwrap();
    }
    report
}

fn mcp_evidence(events: &[RunEvent]) -> Vec<McpEvidence> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Observation { evidence } => match &evidence.record {
                EvidenceRecord::Mcp(mcp) => Some(mcp.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn call(name: &str, arguments: &Value) -> Reply {
    Reply::ok(&tool_calls_reply(&[("c", name, &arguments.to_string())]))
}

fn passing(evidence: &str) -> Reply {
    let assessment = json!({
        "outcome": "passed",
        "criteria": [{"criterion": 0, "status": "satisfied", "basis": "mechanical",
                      "evidence": [evidence], "explanation": "it echoed"}],
        "explanation": "the server echoed"
    });
    call("assess", &assessment)
}

/// The tool names offered to the model in its first request.
fn offered(model: &FakeModel) -> Vec<String> {
    let request = &model.requests()[0].body;
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn an_mcp_backed_plan_produces_a_report_with_redacted_evidence() {
    let script = vec![
        call("mcp__fake__echo", &json!({"text": "hello"})),
        call("mcp__fake__env", &json!({"name": "TOKEN"})),
        passing("ev-1"),
    ];
    let model = FakeModel::start(script).await.unwrap();
    let spec = spec(model.base_url(), "saci-fake-mcp");
    let (output, events) = run(&spec).await;
    assert_eq!(output.status.code(), Some(0), "{events:?}");
    assert_eq!(report(&spec, &events).outcome, Some(Outcome::Passed));
    let evidence = mcp_evidence(&events);
    let names = (evidence[0].server.as_str(), evidence[0].tool.as_str());
    assert_eq!(names, ("fake", "echo"));
    assert_eq!(evidence[0].result.text, "echo: hello");
    assert_eq!(evidence[0].arguments, json!({"text": "hello"}));
    assert!(
        !evidence[1].result.text.contains(SECRET),
        "{:?}",
        evidence[1]
    );
}

#[tokio::test]
async fn mcp_tools_are_offered_with_their_server_schema() {
    let script = vec![
        call("mcp__fake__echo", &json!({"text": "hello"})),
        passing("ev-1"),
    ];
    let model = FakeModel::start(script).await.unwrap();
    run(&spec(model.base_url(), "saci-fake-mcp")).await;
    let tools = offered(&model);
    let expected = [
        "shell",
        "http",
        "assess",
        "mcp__fake__echo",
        "mcp__fake__fail",
        "mcp__fake__sleep",
    ];
    for name in expected {
        assert!(tools.iter().any(|tool| tool == name), "{tools:?}");
    }
    let schema = &model.requests()[0].body["tools"];
    let echo = schema
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["function"]["name"] == "mcp__fake__echo");
    let parameters = &echo.unwrap()["function"]["parameters"];
    assert_eq!(parameters["required"], json!(["text"]));
}

#[tokio::test]
async fn tool_errors_and_protocol_errors_stay_distinct() {
    let script = vec![
        call("mcp__fake__fail", &json!({})),
        call("mcp__fake__sleep", &json!({"seconds": 30})),
        passing("ev-1"),
    ];
    let model = FakeModel::start(script).await.unwrap();
    let mut spec = spec(model.base_url(), "saci-fake-mcp");
    spec.limits.operation_timeout_secs = 1;
    let (_, events) = run(&spec).await;
    let evidence = mcp_evidence(&events);
    assert!(
        evidence[0].is_error && evidence[0].protocol_error.is_none(),
        "{:?}",
        evidence[0]
    );
    assert_eq!(evidence[0].result.text, "the tool failed on purpose");
    assert!(!evidence[1].is_error, "{:?}", evidence[1]);
    let protocol = evidence[1].protocol_error.as_deref().unwrap();
    assert!(protocol.contains("operation deadline (1s)"), "{protocol}");
    let replies = model.requests();
    let fed_back = replies[2].body["messages"].to_string();
    assert!(
        fed_back.contains("protocol error (no result)"),
        "{fed_back}"
    );
}

#[tokio::test]
async fn a_server_that_cannot_start_ends_the_run_before_anything_runs() {
    let model = FakeModel::start(vec![passing("ev-1")]).await.unwrap();
    let spec = spec(model.base_url(), "missing-mcp-server");
    let (_, events) = run(&spec).await;
    let [
        RunEvent {
            kind: EventKind::RunFinished { outcome, reason },
            ..
        },
    ] = events.as_slice()
    else {
        panic!("expected only run_finished, got {events:?}");
    };
    assert_eq!(*outcome, Outcome::Error);
    assert!(
        reason.contains("`fake`") && reason.contains("not found in the worker PATH"),
        "{reason}"
    );
    assert!(model.requests().is_empty(), "the model was not asked");
}

#[tokio::test]
async fn runs_without_mcp_servers_offer_only_the_built_in_tools() {
    let shell = json!({"command": "echo hello"});
    let model = FakeModel::start(vec![call("shell", &shell), passing("ev-1")])
        .await
        .unwrap();
    let mut spec = spec(model.base_url(), "unused");
    spec.environment.mcp.clear();
    let (output, events) = run(&spec).await;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(report(&spec, &events).outcome, Some(Outcome::Passed));
    assert_eq!(offered(&model), ["shell", "http", "assess"]);
}
