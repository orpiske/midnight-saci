//! The worker in investigation mode: it clones the repositories and runs
//! the `prepare` commands before the model, checks the code locations of a
//! conclusion against the clones, and reports the result.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::Output;

use saci_core::event::{EventKind, RunEvent};
use saci_core::ids::RunId;
use saci_core::investigation::{InvestigationOutcome, InvestigationSpec, Repository};
use saci_core::run::{ReportHeader, RunReport};
use saci_core::spec::{
    EnvironmentSpec, ExecutionLimits, LlmSettings, RunPolicy, RunSpec, SCHEMA_VERSION,
};
use saci_core::status::Outcome;
use saci_testkit::{FakeModel, Reply, tool_calls_reply};
use serde_json::json;
use tokio::process::Command;

const WORKER: &str = env!("CARGO_BIN_EXE_saci-worker");

const FAILED_UNIT: &str = r#"{"key":"1.2","id":"1.2","kind":"step","title":"Check totals",
    "phase":"Smoke","location":{"document":"plan.md","line":9},"cleanup":false,
    "criteria":[{"index":0,"text":"The total is 12.50 EUR.",
      "location":{"document":"plan.md","line":12}}],"outcome":"failed",
    "attempts":[{"number":1,"started_at":"2026-10-22T10:00:00Z","outcome":"failed",
      "error":{"kind":"tool","message":"the total was 12.50 USD"},"evidence":[]}]}"#;

/// A repository with one commit: `src/total.rs` has 12 lines.
fn repository(dir: &Path) -> String {
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(["-c", "user.name=Saci", "-c", "user.email=saci@example.com"])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "--quiet", "--initial-branch=main"]);
    std::fs::create_dir(dir.join("src")).unwrap();
    let source: String = (1..=12).map(|n| format!("line {n}\n")).collect();
    std::fs::write(dir.join("src/total.rs"), source).unwrap();
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "Totals"]);
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(head.stdout).unwrap().trim().to_owned()
}

fn investigation(url: &str) -> InvestigationSpec {
    InvestigationSpec {
        parent_run: RunId::parse("run-2b").unwrap(),
        plan_title: "Orders".into(),
        unit: serde_json::from_str(FAILED_UNIT).unwrap(),
        action: "Request the order total.".into(),
        evidence: Vec::new(),
        repositories: vec![Repository {
            name: "orders".into(),
            url: url.into(),
            git_ref: "main".into(),
            description: Some("The orders service.".into()),
        }],
        prepare: vec!["echo \"map for {{failure.title}}: $(ls src)\"".into()],
        instructions: "Do not file bugs in this test.".into(),
    }
}

fn spec(model_url: String, url: &str) -> RunSpec {
    let investigation = investigation(url);
    RunSpec {
        schema_version: SCHEMA_VERSION,
        run_id: RunId::generate(),
        created_at: time::OffsetDateTime::now_utc(),
        bundle: None,
        investigation: Some(Box::new(investigation)),
        environment: EnvironmentSpec {
            name: "investigator".into(),
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
        judge: None,
    }
}

fn conclusion(commit: &str, end_line: u32) -> String {
    json!({
        "outcome": "concluded",
        "summary": "The total drops the currency.",
        "hypotheses": [{
            "statement": "Line 3 formats the total without the currency.",
            "confidence": "high",
            "evidence": ["ev-3"],
            "code": [{"repository": "orders", "commit": commit, "path": "src/total.rs",
                      "start_line": 3, "end_line": end_line}]
        }],
        "no_bug_reason": "Bugs are not filed in this test."
    })
    .to_string()
}

/// Reads the code, cites a line range past the end of the file, then
/// corrects it.
fn script(commit: &str) -> Vec<Reply> {
    let read = json!({ "command": "sed -n '1,5p' src/orders/src/total.rs" }).to_string();
    vec![
        Reply::ok(&tool_calls_reply(&[("s", "shell", &read)])),
        Reply::ok(&tool_calls_reply(&[(
            "c",
            "conclude",
            &conclusion(commit, 40),
        )])),
        Reply::ok(&tool_calls_reply(&[(
            "d",
            "conclude",
            &conclusion(commit, 5),
        )])),
    ]
}

async fn local(spec: &RunSpec, workspace: &Path) -> (Output, Vec<RunEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.json");
    std::fs::write(&path, serde_json::to_string(spec).unwrap()).unwrap();
    let output = Command::new(WORKER)
        .args(["--spec", path.to_str().unwrap()])
        .current_dir(workspace)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .output()
        .await
        .unwrap();
    let events = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (output, events)
}

fn fold(spec: &RunSpec, events: &[RunEvent]) -> RunReport {
    let investigation = spec.investigation.as_ref().unwrap();
    let header = ReportHeader {
        run_id: spec.run_id.clone(),
        plan_title: investigation.plan_title.clone(),
        entry: String::new(),
        environment: "investigator".into(),
        policy: "default".into(),
        model: "scripted".into(),
        created_at: spec.created_at,
    };
    let mut report = RunReport::new(header, vec![investigation.unit_identity()]);
    for event in events {
        report.apply(event).unwrap();
    }
    report
}

#[tokio::test]
async fn an_investigation_concludes_with_code_from_the_clone() {
    let source = tempfile::tempdir().unwrap();
    let commit = repository(source.path());
    let model = FakeModel::start(script(&commit)).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let url = format!("file://{}", source.path().display());
    let spec = spec(model.base_url(), &url);
    let (output, events) = local(&spec, workspace.path()).await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");

    let report = fold(&spec, &events);
    assert_eq!(report.outcome, Some(Outcome::Passed), "{events:#?}");
    let conclusion = report.investigation.unwrap();
    assert_eq!(conclusion.outcome, InvestigationOutcome::Concluded);
    assert_eq!(conclusion.sources[0].commit, commit);
    assert_eq!(conclusion.hypotheses[0].code[0].end_line, 5);

    // The clone and the prepare command ran before the first model request,
    // with the failure title passed as a value.
    let requests = model.requests();
    assert_first_prompt(&requests[0].body["messages"].to_string(), &commit);
    // The out-of-range lines were refused with a reason the model can act on.
    let refusal = requests[2].body["messages"].to_string();
    assert!(refusal.contains("has 12 lines"), "{refusal}");
    assert!(workspace.path().join("src/orders/src/total.rs").exists());
}

fn assert_first_prompt(prompt: &str, commit: &str) {
    assert!(prompt.contains("Failed step: 1.2"), "{prompt}");
    let source = format!("`orders` at commit {commit}");
    assert!(prompt.contains(&source), "{prompt}");
    assert!(
        prompt.contains("map for Check totals: total.rs"),
        "{prompt}"
    );
    assert!(prompt.contains("the total was 12.50 USD"), "{prompt}");
}

#[tokio::test]
async fn a_failed_clone_ends_the_investigation_before_the_model() {
    let model = FakeModel::start(Vec::new()).await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let missing = tempfile::tempdir().unwrap();
    let url = format!("file://{}/none", missing.path().display());
    let spec = spec(model.base_url(), &url);
    let (output, events) = local(&spec, workspace.path()).await;
    assert_eq!(output.status.code(), Some(0));
    let report = fold(&spec, &events);
    assert_eq!(report.outcome, Some(Outcome::Error));
    assert!(report.investigation.is_none());
    let reason = report.reason.unwrap();
    assert!(reason.contains("cannot clone `orders`"), "{reason}");
    assert!(model.requests().is_empty(), "the model was not asked");
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.kind, EventKind::InvestigationConcluded { .. }))
    );
}
