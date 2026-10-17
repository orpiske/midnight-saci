//! Whole runs: unit order, run context, stop, continue and retry policy,
//! cancellation, and a report that folds every emitted event.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use saci_core::bundle::{ExpandedPlan, expand};
use saci_core::ids::RunId;
use saci_core::plan::PlanBundle;
use saci_core::run::{ReportHeader, RunReport};
use saci_core::spec::{
    EnvironmentSpec, ExecutionLimits, FailureAction, LlmSettings, RetryPolicy, RunPolicy,
};
use saci_core::status::Outcome;
use saci_testkit::{FakeModel, Reply, tool_calls_reply};
use saci_worker::agent::{Executor, Toolbox};
use saci_worker::credentials::Credentials;
use saci_worker::events::{EventReceiver, EventSink, Recorder};
use saci_worker::http::HttpTool;
use saci_worker::llm::LlmConnection;
use saci_worker::sequence::{RunEnding, RunPlan, run};
use saci_worker::shell::{ShellConfig, ShellSession};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const PLAN: &str = "# Test Plan: Sequence

## Phase 1: Checks

### Step 1.1: Prepare

Run `export READY=yes`.

**Expected result:**

- The command exits 0.

### Test 1.2: Check

Run `echo $READY`.

**Expected result:**

- The output is `yes`.

## Phase 2: Cleanup

### Step 2.1: Remove

Run `unset READY`.

**Expected result:**

- The command exits 0.
";

/// A scripted attempt: one shell call, then an assessment citing it.
fn attempt(evidence: u64, command: &str, outcome: &str) -> [Reply; 2] {
    let status = if outcome == "passed" {
        "satisfied"
    } else {
        "contradicted"
    };
    let shell = json!({ "command": command }).to_string();
    let assessment = json!({
        "outcome": outcome,
        "criteria": [{"criterion": 0, "status": status, "basis": "mechanical",
                      "evidence": [format!("ev-{evidence}")], "explanation": "checked"}],
        "explanation": format!("the unit {outcome}")
    })
    .to_string();
    [
        Reply::ok(&tool_calls_reply(&[("s", "shell", &shell)])),
        Reply::ok(&tool_calls_reply(&[("a", "assess", &assessment)])),
    ]
}

struct Run {
    ending: RunEnding,
    report: RunReport,
    model: FakeModel,
}

fn executor(
    llm: &LlmSettings,
    workspace: &std::path::Path,
    run_id: RunId,
) -> (Executor, EventReceiver) {
    let limits = ExecutionLimits::default();
    let path = std::env::var("PATH").unwrap();
    let credentials = Credentials::project(
        &EnvironmentSpec::default(),
        llm,
        [("PATH".to_owned(), path)],
    )
    .unwrap();
    let shell = ShellSession::new(ShellConfig {
        workspace: workspace.to_owned(),
        environment: credentials.shell_environment().clone(),
        output_limit: limits.output_limit_bytes,
        redactor: credentials.redactor().clone(),
    })
    .unwrap();
    let (sink, events) = EventSink::new(run_id);
    let toolbox = Toolbox {
        shell,
        http: HttpTool::new(credentials.clone(), limits.output_limit_bytes).unwrap(),
        recorder: Recorder::new(Arc::new(sink), credentials.redactor().clone()),
        environment: String::new(),
    };
    let connection = LlmConnection::new(llm, &limits, |_| None).unwrap();
    (Executor::new(connection, toolbox, limits), events)
}

fn report(plan: &ExpandedPlan, run_id: &RunId, mut events: EventReceiver) -> RunReport {
    let header = ReportHeader {
        run_id: run_id.clone(),
        plan_title: plan.plan.title.clone(),
        entry: "plan.md".into(),
        environment: "local".into(),
        policy: "default".into(),
        model: "scripted".into(),
        created_at: time::OffsetDateTime::now_utc(),
    };
    let mut report = RunReport::new(
        header,
        plan.units.iter().map(|u| u.identity.clone()).collect(),
    );
    while let Ok(event) = events.try_recv() {
        report
            .apply(&event)
            .unwrap_or_else(|error| panic!("{error}: {event:?}"));
    }
    report
}

fn plan() -> ExpandedPlan {
    let bundle = PlanBundle {
        entry: "plan.md".into(),
        documents: BTreeMap::from([("plan.md".to_owned(), PLAN.to_owned())]),
    };
    expand(&bundle).unwrap()
}

async fn execute(script: Vec<Reply>, policy: RunPolicy, cancel: &CancellationToken) -> Run {
    let model = FakeModel::start(script).await.unwrap();
    let llm = LlmSettings {
        base_url: model.base_url(),
        model: "scripted".into(),
        api_key_env: None,
        api_key_secret: None,
    };
    let workspace = tempfile::tempdir().unwrap();
    let run_id = RunId::generate();
    let (executor, events) = executor(&llm, workspace.path(), run_id.clone());
    let plan = plan();
    let run_plan = RunPlan {
        plan: &plan,
        policy: &policy,
        step_timeout: Duration::from_secs(30),
        deadline: Instant::now() + Duration::from_secs(60),
        model: "scripted",
    };
    let ending = Box::pin(run(&executor, run_plan, cancel)).await;
    executor.close().await;
    Run {
        ending,
        report: report(&plan, &run_id, events),
        model,
    }
}

fn outcomes(report: &RunReport) -> Vec<Option<Outcome>> {
    report.units.iter().map(|unit| unit.outcome).collect()
}

#[tokio::test]
async fn units_run_in_order_with_shared_state_and_context() {
    let script = [
        attempt(1, "export READY=yes", "passed"),
        attempt(2, "echo $READY", "passed"),
        attempt(3, "unset READY", "passed"),
    ]
    .concat();
    let run = execute(script, RunPolicy::default(), &CancellationToken::new()).await;
    assert_eq!(run.ending.outcome, Outcome::Passed, "{:?}", run.ending);
    assert_eq!(run.report.outcome, Some(Outcome::Passed));
    let stdout = match &run.report.evidence[1].record {
        saci_core::evidence::EvidenceRecord::Shell(shell) => shell.stdout.text.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(stdout, "yes\n");
    let second = run.model.requests()[2].body["messages"].to_string();
    assert!(
        second.contains("Earlier results:") && second.contains("1.1"),
        "{second}"
    );
    assert!(second.contains("Test plan: Sequence"));
}

#[tokio::test]
async fn the_default_policy_stops_and_skips_the_rest_with_a_reason() {
    let script = attempt(1, "false", "failed").to_vec();
    let run = execute(script, RunPolicy::default(), &CancellationToken::new()).await;
    assert_eq!(run.ending.outcome, Outcome::Failed);
    assert!(
        run.ending.reason.contains("stopped after 1.1 ended failed"),
        "{}",
        run.ending.reason
    );
    assert_eq!(
        outcomes(&run.report),
        [
            Some(Outcome::Failed),
            Some(Outcome::Skipped),
            Some(Outcome::Skipped)
        ]
    );
    assert!(
        run.report.units[1]
            .reason
            .as_deref()
            .unwrap()
            .starts_with("not attempted: stopped after 1.1")
    );
    assert_eq!(
        run.report.cleanup_not_run,
        [saci_core::ids::UnitKey::new("2.1")]
    );
}

#[tokio::test]
async fn continue_policy_runs_every_unit() {
    let script = [
        attempt(1, "false", "failed"),
        attempt(2, "echo $READY", "failed"),
        attempt(3, "true", "passed"),
    ]
    .concat();
    let policy = RunPolicy {
        name: "keep-going".into(),
        on_failure: FailureAction::Continue,
        ..RunPolicy::default()
    };
    let run = execute(script, policy, &CancellationToken::new()).await;
    assert_eq!(
        outcomes(&run.report),
        [
            Some(Outcome::Failed),
            Some(Outcome::Failed),
            Some(Outcome::Passed)
        ]
    );
    assert_eq!(run.ending.outcome, Outcome::Failed);
    assert!(run.report.cleanup_not_run.is_empty());
}

#[tokio::test]
async fn retries_are_bounded_and_keep_every_attempt() {
    let script = [
        attempt(1, "false", "failed"),
        attempt(2, "true", "passed"),
        attempt(3, "false", "failed"),
        attempt(4, "false", "failed"),
    ]
    .concat();
    let policy = RunPolicy {
        name: "retry".into(),
        retry: RetryPolicy {
            max_attempts: 2,
            on: vec![Outcome::Failed],
        },
        ..RunPolicy::default()
    };
    let run = execute(script, policy, &CancellationToken::new()).await;
    let first = &run.report.units[0];
    assert_eq!(first.outcome, Some(Outcome::Passed));
    let attempts: Vec<_> = first.attempts.iter().map(|a| a.outcome).collect();
    assert_eq!(attempts, [Some(Outcome::Failed), Some(Outcome::Passed)]);
    let second = &run.report.units[1];
    assert_eq!(second.attempts.len(), 2, "the retry limit holds");
    assert_eq!(second.outcome, Some(Outcome::Failed));
    assert_eq!(run.report.units[2].outcome, Some(Outcome::Skipped));
}

#[tokio::test]
async fn a_cancelled_run_attempts_nothing_more() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let run = execute(Vec::new(), RunPolicy::default(), &cancel).await;
    assert_eq!(run.ending.outcome, Outcome::Cancelled);
    assert_eq!(run.report.outcome, Some(Outcome::Cancelled));
    assert!(
        run.report
            .units
            .iter()
            .all(|u| u.outcome == Some(Outcome::Skipped))
    );
    assert_eq!(run.report.cleanup_not_run.len(), 1);
    assert!(run.model.requests().is_empty());
}
