//! One attempt at a time through Rig, Saci's tools and controls, against a
//! scripted model.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use saci_core::bundle::{ExecutionUnit, expand};
use saci_core::event::{EventKind, RunEvent};
use saci_core::evidence::{EvidenceRecord, Termination};
use saci_core::ids::RunId;
use saci_core::plan::PlanBundle;
use saci_core::run::ErrorKind;
use saci_core::spec::{
    EnvironmentSpec, ExecutionLimits, JudgeProvider, JudgeSettings, LlmSettings,
};
use saci_core::status::Outcome;
use saci_testkit::{FakeModel, Reply, text_reply, tool_calls_reply};
use saci_worker::agent::{AttemptRequest, AttemptResult, Executor, Judge, Toolbox, prompt};
use saci_worker::credentials::Credentials;
use saci_worker::events::{EventReceiver, EventSink, Recorder};
use saci_worker::http::HttpTool;
use saci_worker::llm::LlmConnection;
use saci_worker::shell::{ShellConfig, ShellSession};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const PLAN: &str = "# Test Plan: Agent

## Phase 1: Main

### Step 1.1: Greet

Run:

```bash
echo hello
```

**Expected result:**

- The output is `hello`.

### Test 1.2: Reject

Run `false`.

**Expected result:**

- The command succeeds.

### Step 1.3: Prepared

```bash run
echo prepared
```

Then, if needed:

```bash
echo by the model
```

**Expected result:**

- The output is `prepared`.

### Step 1.4: Stops

```bash run
exit 4
```

```bash run
echo never
```

**Expected result:**

- The first block ends the shell.

### Step 1.5: Judged

```bash run
echo judged
```

**Expected result:**

- The output is `judged`.

### Step 1.6: After a lost session

```bash run
echo first
```

```bash run
echo second
```

**Expected result:**

- The output is `second`.
";

struct Harness {
    model: FakeModel,
    executor: Executor,
    events: EventReceiver,
    units: Vec<ExecutionUnit>,
    _workspace: tempfile::TempDir,
}

fn toolbox(
    llm: &LlmSettings,
    limits: &ExecutionLimits,
) -> (Toolbox, EventReceiver, tempfile::TempDir) {
    let path = std::env::var("PATH").unwrap();
    let credentials = Credentials::project(
        &EnvironmentSpec::default(),
        llm,
        [("PATH".to_owned(), path)],
    )
    .unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let shell = ShellSession::new(ShellConfig {
        workspace: workspace.path().to_owned(),
        environment: credentials.shell_environment().clone(),
        output_limit: limits.output_limit_bytes,
        redactor: credentials.redactor().clone(),
    })
    .unwrap();
    let (sink, events) = EventSink::new(RunId::generate());
    let toolbox = Toolbox {
        shell,
        http: HttpTool::new(credentials.clone(), limits.output_limit_bytes).unwrap(),
        recorder: Recorder::new(Arc::new(sink), credentials.redactor().clone()),
        environment: credentials.model_context(),
    };
    (toolbox, events, workspace)
}

async fn harness(script: Vec<Reply>, limits: ExecutionLimits) -> Harness {
    let model = FakeModel::start(script).await.unwrap();
    let llm = LlmSettings {
        base_url: model.base_url(),
        model: "scripted".into(),
        api_key_env: None,
        api_key_secret: None,
    };
    let (toolbox, events, workspace) = toolbox(&llm, &limits);
    let connection = LlmConnection::new(&llm, &limits, |_| None).unwrap();
    let bundle = PlanBundle {
        entry: "plan.md".into(),
        documents: BTreeMap::from([("plan.md".to_owned(), PLAN.to_owned())]),
    };
    Harness {
        model,
        executor: Executor::new(connection, toolbox, limits),
        events,
        units: expand(&bundle).unwrap().units,
        _workspace: workspace,
    }
}

impl Harness {
    async fn run(&self, unit: usize, attempt: u32, cancel: &CancellationToken) -> AttemptResult {
        self.run_until(
            unit,
            attempt,
            Instant::now() + Duration::from_secs(30),
            cancel,
        )
        .await
    }

    async fn run_until(
        &self,
        unit: usize,
        attempt: u32,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> AttemptResult {
        let request = AttemptRequest {
            unit: &self.units[unit],
            attempt,
            context: "Test plan: Agent",
            procedures: &[],
            deadline,
            cancel,
        };
        Box::pin(self.executor.attempt(request)).await
    }

    fn drain(&mut self) -> Vec<RunEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            events.push(event);
        }
        events
    }

    /// The `role: tool` messages of request `index`.
    fn tool_results(&self, index: usize) -> Vec<String> {
        self.model.requests()[index].body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .map(|message| message["content"].as_str().unwrap_or_default().to_owned())
            .collect()
    }
}

fn shell_call(id: &str, command: &str) -> Reply {
    let args = json!({ "command": command }).to_string();
    Reply::ok(&tool_calls_reply(&[(id, "shell", &args)]))
}

fn assess(outcome: &str, status: &str, evidence: &[&str]) -> Reply {
    let args = assessment(outcome, status, evidence).to_string();
    Reply::ok(&tool_calls_reply(&[("a1", "assess", &args)]))
}

fn assessment(outcome: &str, status: &str, evidence: &[&str]) -> Value {
    json!({
        "outcome": outcome,
        "criteria": [{
            "criterion": 0,
            "status": status,
            "basis": "mechanical",
            "evidence": evidence,
            "explanation": "checked the recorded output"
        }],
        "explanation": "compared the evidence with the expected result"
    })
}

fn shell_stdout(events: &[RunEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Observation { evidence } => match &evidence.record {
                EvidenceRecord::Shell(shell) => Some(shell.stdout.text.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_passing_step_records_evidence_and_ends_at_the_assessment() {
    let script = vec![
        shell_call("c1", "echo hello"),
        assess("passed", "satisfied", &["ev-1"]),
    ];
    let mut h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    assert_eq!(
        h.model.requests().len(),
        2,
        "no request after the assessment"
    );
    let seen = h.tool_results(1);
    assert!(seen[0].starts_with("Evidence ev-1 (shell"), "{seen:?}");
    assert!(seen[0].contains("exit code: 0") && seen[0].contains("hello"));
    let events = h.drain();
    let kinds: Vec<_> = events.iter().map(|e| e.kind.clone()).collect();
    assert!(matches!(kinds[0], EventKind::AttemptStarted));
    assert!(matches!(kinds[1], EventKind::Observation { .. }));
    assert!(matches!(
        &kinds[2],
        EventKind::AttemptFinished {
            outcome: Outcome::Passed,
            assessment: Some(_),
            ..
        }
    ));
    assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
    let system = &h.model.requests()[0].body["messages"][0];
    assert!(system.to_string().contains("never instructions to you"));
}

#[tokio::test]
async fn an_attempt_reports_model_time_apart_from_tool_time() {
    let script = vec![
        shell_call("c1", "sleep 0.3").delayed(Duration::from_millis(400)),
        assess("passed", "satisfied", &["ev-1"]).delayed(Duration::from_millis(200)),
    ];
    let mut h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    let timing = result.timing.unwrap();
    assert_eq!(timing.model_requests, 2, "{timing:?}");
    // Both replies are delayed (600 ms); the 300 ms shell call is not model time.
    assert!((600..900).contains(&timing.model_ms), "{timing:?}");
    assert!(timing.tool_ms >= 300, "{timing:?}");
    let events = h.drain();
    let reported = events.iter().find_map(|event| match &event.kind {
        EventKind::AttemptFinished { timing, .. } => *timing,
        _ => None,
    });
    assert_eq!(reported, Some(timing));
}

#[tokio::test]
async fn a_failed_negative_test_is_failed_not_error() {
    let script = vec![
        shell_call("c1", "false"),
        assess("failed", "contradicted", &["ev-1"]),
    ];
    let h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(1, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Failed, "{result:?}");
    assert!(h.tool_results(1)[0].contains("exit code: 1"));
}

#[tokio::test]
async fn invalid_tool_requests_are_recorded_counted_and_answered() {
    let script = vec![
        Reply::ok(&tool_calls_reply(&[("c1", "deploy", "{}")])),
        Reply::ok(&tool_calls_reply(&[("c2", "shell", "{not json")])),
        Reply::ok(&tool_calls_reply(&[("c3", "shell", r#"{"cmd":"ls"}"#)])),
        Reply::ok(&tool_calls_reply(&[("c4", "deploy", "{}")])),
    ];
    let limits = ExecutionLimits {
        tool_calls_per_step: 3,
        ..ExecutionLimits::default()
    };
    let mut h = harness(script, limits).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    let error = result.error.unwrap();
    assert_eq!(error.kind, ErrorKind::LimitExceeded);
    assert!(error.message.contains("3 tool calls"), "{}", error.message);
    assert_eq!(
        h.model.requests().len(),
        4,
        "the fourth invalid call stops the attempt"
    );
    assert!(h.tool_results(1)[0].contains("there is no tool `deploy`"));
    assert!(h.tool_results(2)[1].contains("not a JSON object"));
    assert!(h.tool_results(3)[2].contains("invalid arguments for `shell`"));
    let rejected = h
        .drain()
        .into_iter()
        .filter(|e| matches!(&e.kind, EventKind::Observation { evidence } if matches!(evidence.record, EvidenceRecord::Rejected(_))))
        .count();
    assert_eq!(rejected, 4);
}

#[tokio::test]
async fn several_calls_in_one_reply_cannot_exceed_the_tool_limit() {
    let calls = [
        ("c1", "shell", r#"{"command":"echo one"}"#),
        ("c2", "shell", r#"{"command":"echo two"}"#),
    ];
    let limits = ExecutionLimits {
        tool_calls_per_step: 1,
        ..ExecutionLimits::default()
    };
    let mut h = harness(vec![Reply::ok(&tool_calls_reply(&calls))], limits).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    assert_eq!(result.error.unwrap().kind, ErrorKind::LimitExceeded);
    assert_eq!(
        shell_stdout(&h.drain()),
        ["one\n"],
        "the second call never ran"
    );
    assert_eq!(h.model.requests().len(), 1);
}

#[tokio::test]
async fn missing_evidence_is_refused_and_can_be_corrected() {
    let script = vec![
        shell_call("c1", "echo hello"),
        assess("passed", "satisfied", &["ev-9"]),
        Reply::ok(&text_reply("I think it passed.")),
        assess("passed", "satisfied", &["ev-1"]),
    ];
    let h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    assert!(h.tool_results(2)[1].contains("evidence `ev-9` was not recorded in this run"));
    let retry = h.model.requests()[3].body["messages"].to_string();
    assert!(
        retry.contains("call the `assess` tool"),
        "a text reply is sent back"
    );
}

#[tokio::test]
async fn an_unassessed_step_is_an_error_at_the_model_call_limit() {
    let script = vec![
        Reply::ok(&text_reply("done")),
        Reply::ok(&text_reply("still done")),
    ];
    let limits = ExecutionLimits {
        model_calls_per_step: 2,
        ..ExecutionLimits::default()
    };
    let h = harness(script, limits).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    let error = result.error.unwrap();
    assert_eq!(error.kind, ErrorKind::LimitExceeded);
    assert!(
        error.message.contains("2 model requests"),
        "{}",
        error.message
    );
    assert_eq!(h.model.requests().len(), 2);
}

#[tokio::test]
async fn provider_failures_are_execution_errors() {
    let script = vec![Reply::raw(401, r#"{"error":{"message":"bad key"}}"#)];
    let h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Error);
    let error = result.error.unwrap();
    assert_eq!(error.kind, ErrorKind::Provider);
    assert!(
        error.message.contains("rejected the credentials"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn cancellation_stops_a_running_command_and_keeps_the_session() {
    let script = vec![
        shell_call("c1", "export KEPT=yes; sleep 30"),
        shell_call("c2", "echo $KEPT"),
        assess("passed", "satisfied", &["ev-2"]),
    ];
    let mut h = harness(script, ExecutionLimits::default()).await;
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        trigger.cancel();
    });
    let started = Instant::now();
    let result = h.run(0, 1, &cancel).await;
    assert_eq!(result.outcome, Outcome::Cancelled, "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let events = h.drain();
    let cancelled = events.iter().any(|e| matches!(&e.kind, EventKind::Observation { evidence } if matches!(&evidence.record, EvidenceRecord::Shell(s) if s.termination == Termination::Cancelled)));
    assert!(cancelled, "the interrupted command is evidence");
    let result = h.run(0, 2, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    assert_eq!(shell_stdout(&h.drain()), ["yes\n"]);
}

#[tokio::test]
async fn the_attempt_deadline_interrupts_a_pending_model_request() {
    let script = vec![Reply::ok(&text_reply("late")).delayed(Duration::from_secs(20))];
    let h = harness(script, ExecutionLimits::default()).await;
    let started = Instant::now();
    let deadline = started + Duration::from_millis(500);
    let result = h.run_until(0, 1, deadline, &CancellationToken::new()).await;
    assert!(started.elapsed() < Duration::from_secs(3));
    let error = result.error.unwrap();
    assert_eq!(error.kind, ErrorKind::LimitExceeded);
    assert!(error.message.contains("deadline"), "{}", error.message);
}

#[tokio::test]
async fn shell_state_carries_over_between_units() {
    let script = vec![
        shell_call(
            "c1",
            "cd /tmp && GREETING=hi && greet() { echo \"$GREETING from $PWD\"; }",
        ),
        assess("passed", "satisfied", &["ev-1"]),
        shell_call("c2", "greet"),
        assess("failed", "contradicted", &["ev-2"]),
    ];
    let mut h = harness(script, ExecutionLimits::default()).await;
    h.run(0, 1, &CancellationToken::new()).await;
    let result = h.run(1, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Failed);
    assert_eq!(shell_stdout(&h.drain()), ["", "hi from /tmp\n"]);
}

/// A port nothing listens on.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

#[tokio::test]
async fn a_refused_connection_cannot_pass_a_step() {
    let url = format!("http://127.0.0.1:{}/health", closed_port());
    let args = json!({"method": "GET", "url": url}).to_string();
    let script = vec![
        Reply::ok(&tool_calls_reply(&[("c1", "http", &args)])),
        assess("passed", "satisfied", &["ev-1"]),
        assess("blocked", "undetermined", &["ev-1"]),
    ];
    let h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Blocked, "{result:?}");
    let refusal = &h.tool_results(2)[1];
    assert!(
        refusal.contains("the cited evidence holds no answer from the target (`ev-1`: the request got no response"),
        "{refusal}"
    );
}

/// The user prompt and offered tool names of request `index`.
fn request(h: &Harness, index: usize) -> (String, Vec<String>) {
    let body = &h.model.requests()[index].body;
    let prompt = body["messages"][1]["content"].to_string();
    let tools = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap().to_owned())
        .collect();
    (prompt, tools)
}

#[tokio::test]
async fn marked_blocks_run_before_the_model_which_keeps_its_tools() {
    let script = vec![assess("passed", "satisfied", &["ev-1"])];
    let mut h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(2, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    assert_eq!(h.model.requests().len(), 1, "the model only assessed");
    let (prompt, tools) = request(&h, 0);
    assert!(prompt.contains("Saci already ran"), "{prompt}");
    assert!(prompt.contains("Evidence ev-1 (shell") && prompt.contains("prepared"));
    assert!(!prompt.contains("did not run"), "{prompt}");
    assert!(tools.contains(&"shell".to_owned()), "{tools:?}");
    let stdout = shell_stdout(&h.drain());
    assert_eq!(
        stdout,
        ["prepared\n"],
        "the unmarked block is left to the model"
    );
}

#[tokio::test]
async fn marked_blocks_stop_at_one_that_does_not_exit_normally() {
    let script = vec![assess("passed", "satisfied", &["ev-1"])];
    let mut h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(3, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    let (prompt, _) = request(&h, 0);
    assert!(prompt.contains("1 marked block(s) did not run"), "{prompt}");
    assert_eq!(
        shell_stdout(&h.drain()).len(),
        1,
        "`echo never` did not run"
    );
}

#[tokio::test]
async fn marked_blocks_count_against_the_tool_call_limit() {
    let limits = ExecutionLimits {
        tool_calls_per_step: 0,
        ..ExecutionLimits::default()
    };
    let h = harness(vec![], limits).await;
    let result = h.run(2, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Error, "{result:?}");
    assert_eq!(result.error.unwrap().kind, ErrorKind::LimitExceeded);
    assert!(
        h.model.requests().is_empty(),
        "no model request after the limit"
    );
}

/// `reply` with the token usage a caching provider reports.
fn with_usage(mut reply: Value) -> Reply {
    reply["usage"] = json!({
        "prompt_tokens": 1000, "completion_tokens": 50, "total_tokens": 1050,
        "prompt_tokens_details": {"cached_tokens": 800}
    });
    Reply::ok(&reply)
}

#[tokio::test]
async fn an_attempt_reports_the_tokens_the_provider_counted() {
    let args = assessment("passed", "satisfied", &["ev-1"]).to_string();
    let script = vec![
        with_usage(tool_calls_reply(&[(
            "c1",
            "shell",
            r#"{"command":"echo hello"}"#,
        )])),
        with_usage(tool_calls_reply(&[("a1", "assess", &args)])),
    ];
    let h = harness(script, ExecutionLimits::default()).await;
    let result = h.run(0, 1, &CancellationToken::new()).await;
    let timing = result.timing.unwrap();
    assert_eq!(
        (
            timing.input_tokens,
            timing.cached_tokens,
            timing.output_tokens
        ),
        (2000, Some(1600), 100)
    );
}

#[tokio::test]
async fn every_unit_gets_the_same_system_prompt_and_tools() {
    let script = vec![
        assess("passed", "satisfied", &["ev-1"]),
        assess("passed", "satisfied", &["ev-2"]),
    ];
    let h = harness(script, ExecutionLimits::default()).await;
    h.run(2, 1, &CancellationToken::new()).await;
    h.run(3, 1, &CancellationToken::new()).await;
    let requests = h.model.requests();
    let prefix = |i: usize| {
        let body = &requests[i].body;
        (body["tools"].clone(), body["messages"][0].clone())
    };
    assert_eq!(prefix(0), prefix(1), "the cacheable prefix is stable");
}

#[test]
fn the_environment_is_part_of_the_system_prompt() {
    let environment = "Environment parameters (also exported to the shell):\n- REGION = eu\n";
    let system = prompt::system(environment);
    assert!(system.starts_with(prompt::PREAMBLE) && system.ends_with("- REGION = eu"));
    assert_eq!(prompt::system(""), prompt::PREAMBLE);
}

/// A fake judge whose replies give `scores` per criterion.
fn judge_reply(scores: &[f64]) -> Reply {
    let answers: serde_json::Map<String, Value> = scores
        .iter()
        .enumerate()
        .map(|(i, noul)| {
            (
                format!("criterion_{i}"),
                json!({"type": "noul", "noul": noul}),
            )
        })
        .collect();
    Reply::ok(&json!({"model": "jev-test", "answers": answers}))
}

async fn judged_harness(script: Vec<Reply>, judge: Vec<Reply>) -> (Harness, FakeModel) {
    Box::pin(judged_by(
        JudgeProvider::Typesafe,
        Some("judge-key-value"),
        script,
        judge,
    ))
    .await
}

async fn judged_by(
    provider: JudgeProvider,
    key: Option<&str>,
    script: Vec<Reply>,
    judge: Vec<Reply>,
) -> (Harness, FakeModel) {
    let limits = ExecutionLimits::default();
    let mut h = harness(script, limits).await;
    let fake = FakeModel::start(judge).await.unwrap();
    let settings = JudgeSettings {
        provider,
        endpoint: format!("{}/systemone", fake.base_url()),
        model: "jev-test".into(),
        api_key_env: key.map(|_| "JUDGE_KEY".into()),
        api_key_secret: None,
        threshold_percent: 90,
    };
    h.executor
        .attach_judge(Judge::new(&settings, key, &limits).unwrap());
    (h, fake)
}

#[tokio::test]
async fn a_julia_judge_without_a_key_passes_a_step_with_a_placeholder_token() {
    let (h, judge) = Box::pin(judged_by(
        JudgeProvider::Julia1,
        None,
        vec![],
        vec![judge_reply(&[0.95])],
    ))
    .await;
    let result = h.run(4, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    assert!(h.model.requests().is_empty(), "the model is not asked");
    let requests = judge.requests();
    assert_eq!(requests[0].headers["authorization"], "Bearer none");
    assert_eq!(requests[0].body["questions"]["criterion_0"]["type"], "noul");
}

#[tokio::test]
async fn the_judge_passes_a_fully_run_step_without_the_model() {
    let (h, judge) = judged_harness(vec![], vec![judge_reply(&[0.97])]).await;
    let result = h.run(4, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    assert!(h.model.requests().is_empty(), "the model is not asked");
    assert_eq!(result.timing.unwrap().model_requests, 0);
    let assessment = result.assessment.unwrap();
    assert_eq!(assessment.criteria[0].evidence[0].as_str(), "ev-1");
    assert!(assessment.criteria[0].explanation.contains("0.97"));
    assert!(assessment.explanation.contains("jev-test"));
    let requests = judge.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.path, "/v1/systemone");
    assert_eq!(request.headers["authorization"], "Bearer judge-key-value");
    assert_eq!(request.body["model"], "jev-test");
    let question = &request.body["questions"]["criterion_0"];
    assert_eq!(question["type"], "noul");
    assert!(
        question["instructions"]
            .as_str()
            .unwrap()
            .contains("The output is `judged`.")
    );
    let evidence = request.body["state"]["evidence"][0].as_str().unwrap();
    assert!(evidence.starts_with("Evidence ev-1 (shell") && evidence.contains("judged"));
}

#[tokio::test]
async fn the_model_assesses_what_the_judge_does_not_pass() {
    for reply in [judge_reply(&[0.89]), Reply::raw(500, "unavailable")] {
        let script = vec![assess("passed", "satisfied", &["ev-1"])];
        let (h, judge) = judged_harness(script, vec![reply]).await;
        let result = h.run(4, 1, &CancellationToken::new()).await;
        assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
        assert_eq!(judge.requests().len(), 1);
        assert_eq!(h.model.requests().len(), 1, "the model assesses the step");
        let assessment = result.assessment.unwrap();
        assert!(!assessment.explanation.contains("jev-test"));
    }
}

#[tokio::test]
async fn the_judge_is_not_asked_when_the_model_must_act() {
    let script = vec![assess("passed", "satisfied", &["ev-1"])];
    let (h, judge) = judged_harness(script, vec![judge_reply(&[0.99])]).await;
    let result = h.run(2, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{result:?}");
    assert!(
        judge.requests().is_empty(),
        "step 1.3 has an unmarked block"
    );
    let (h, judge) = judged_harness(vec![], vec![judge_reply(&[0.99])]).await;
    let result = h.run(3, 1, &CancellationToken::new()).await;
    assert!(judge.requests().is_empty(), "step 1.4 stopped at `exit 4`");
    assert_ne!(result.outcome, Outcome::Passed);
}

#[tokio::test]
async fn marked_blocks_stop_after_a_lost_session_and_the_judge_is_not_asked() {
    let script = vec![
        assess("passed", "satisfied", &["ev-1"]),
        assess("blocked", "undetermined", &["ev-2"]),
    ];
    let (mut h, judge) = judged_harness(script, vec![judge_reply(&[0.99])]).await;
    h.run(3, 1, &CancellationToken::new()).await;
    let result = h.run(5, 1, &CancellationToken::new()).await;
    assert_eq!(result.outcome, Outcome::Blocked, "{result:?}");
    assert!(judge.requests().is_empty(), "the session was replaced");
    let (prompt, _) = request(&h, 1);
    assert!(prompt.contains("ran in a new session"), "{prompt}");
    assert!(prompt.contains("1 marked block(s) did not run"), "{prompt}");
    assert_eq!(
        shell_stdout(&h.drain()),
        ["", "first\n"],
        "`echo second` did not run"
    );
}
