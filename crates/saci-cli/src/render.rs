//! Human-readable progress, summaries and reports. `--json` output bypasses
//! this module and prints the API's own bodies.

use std::fmt::Write as _;

use saci_core::action::{ActionRecord, ActionState, HistoryEntry, Verdict};
use saci_core::api::RunSummary;
use saci_core::event::{EventKind, RunEvent};
use saci_core::evidence::{Evidence, EvidenceRecord};
use saci_core::run::{Attempt, AttemptTiming, OutcomeCounts, RunReport, UnitReport};
use saci_core::status::Lifecycle;
use serde::Serialize;

/// The snake_case name an enum has in the API.
fn name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// One line per event.
#[must_use]
pub fn event(event: &RunEvent) -> String {
    let unit = event
        .unit
        .as_ref()
        .map(|unit| format!("{unit} "))
        .unwrap_or_default();
    let attempt = event
        .attempt
        .map(|n| format!("attempt {n} "))
        .unwrap_or_default();
    let what = match &event.kind {
        EventKind::RunStarted { model } => format!("run started with model {model}"),
        EventKind::AttemptStarted => "started".to_owned(),
        EventKind::Observation { evidence } => format!("  {}", observation(evidence)),
        EventKind::AttemptFinished {
            outcome,
            assessment,
            error,
            timing,
        } => attempt_ending(
            *outcome,
            assessment.as_ref(),
            error.as_ref(),
            timing.as_ref(),
        ),
        EventKind::UnitFinished { outcome } => format!("{outcome}"),
        EventKind::UnitSkipped { reason } => format!("skipped: {reason}"),
        EventKind::CancellationRequested => "cancellation requested".to_owned(),
        EventKind::RunFinished { outcome, reason } => format!("run finished {outcome}: {reason}"),
    };
    format!("[{}] {unit}{attempt}{what}", event.seq)
}

fn attempt_ending(
    outcome: saci_core::status::Outcome,
    assessment: Option<&saci_core::assessment::Assessment>,
    error: Option<&saci_core::run::ExecutionError>,
    timing: Option<&AttemptTiming>,
) -> String {
    let detail = assessment
        .map(|a| a.explanation.clone())
        .or_else(|| error.map(|e| format!("{}: {}", name(&e.kind), e.message)));
    format!(
        "{outcome}{}{}",
        timing_note(timing),
        detail.map(|d| format!(": {d}")).unwrap_or_default()
    )
}

/// Where an attempt spent its time, as a note after its outcome.
fn timing_note(timing: Option<&AttemptTiming>) -> String {
    timing.map_or_else(String::new, |t| {
        format!(
            " (model: {} request{}, {} ms{}; tools: {} ms)",
            t.model_requests,
            if t.model_requests == 1 { "" } else { "s" },
            t.model_ms,
            tokens_note(t),
            t.tool_ms
        )
    })
}

/// The token counts of an attempt; empty when the provider reported none.
fn tokens_note(t: &AttemptTiming) -> String {
    if t.input_tokens == 0 && t.output_tokens == 0 {
        return String::new();
    }
    let cached = t
        .cached_tokens
        .map(|cached| format!(" ({cached} cached)"))
        .unwrap_or_default();
    format!(
        ", tokens: {} in{cached}, {} out",
        t.input_tokens, t.output_tokens
    )
}

/// What one observation shows, in a line.
#[must_use]
pub fn observation(evidence: &Evidence) -> String {
    let what = match &evidence.record {
        EvidenceRecord::Shell(shell) => {
            let ending = shell
                .exit_code
                .map_or_else(|| name(&shell.termination), |code| format!("exited {code}"));
            format!("shell `{}` {ending}", first_line(&shell.command))
        }
        EvidenceRecord::Http(http) => http_line(http),
        EvidenceRecord::Mcp(mcp) => {
            let result = match (&mcp.protocol_error, mcp.is_error) {
                (Some(error), _) => format!("got no result: {error}"),
                (None, true) => "returned an error".to_owned(),
                (None, false) => "returned".to_owned(),
            };
            format!("mcp {}/{} {result}", mcp.server, mcp.tool)
        }
        EvidenceRecord::Rejected(call) => {
            format!("rejected call to {}: {}", call.name, call.reason)
        }
    };
    format!("{} {what} ({} ms)", evidence.id, evidence.duration_ms)
}

fn http_line(http: &saci_core::evidence::HttpEvidence) -> String {
    let result = match (&http.response, &http.transport_error) {
        (Some(response), _) => match &response.body_error {
            Some(error) => format!("answered {}, body incomplete: {error}", response.status),
            None => format!("answered {}", response.status),
        },
        (None, Some(error)) => format!("failed: {error}"),
        (None, None) => "got no response".to_owned(),
    };
    format!("http {} {} {result}", http.method, http.url)
}

fn first_line(text: &str) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    if lines.next().is_some() {
        format!("{first} …")
    } else {
        first.to_owned()
    }
}

/// An action's state as shown to people, including an approval.
#[must_use]
pub fn action_state(action: &ActionRecord) -> &'static str {
    let approved = action
        .decision
        .as_ref()
        .is_some_and(|d| d.verdict == Verdict::Approved);
    match action.state {
        ActionState::PendingApproval => "pending approval",
        ActionState::Queued if approved => "approved, queued",
        ActionState::Queued => "queued",
        ActionState::Executing => "executing",
        ActionState::Rejected => "rejected",
        ActionState::Completed => "completed",
        ActionState::Error => "error",
        ActionState::Unknown => "unknown, needs reconciliation",
    }
}

/// One line for an action: name, state, server and tool, and the first
/// line of its result or error.
#[must_use]
pub fn action_line(action: &ActionRecord) -> String {
    let mut text = format!(
        "{}: {} ({}/{})",
        action.name,
        action_state(action),
        action.server,
        action.tool
    );
    if let Some(detail) = action.error.as_deref().or(action.result.as_deref()) {
        let _ = write!(text, ": {}", first_line(detail));
    }
    text
}

/// A run's outcome actions, one line each, with what delivery returned.
/// Delivery is reported apart from the run's outcome, which it never changes.
#[must_use]
pub fn actions(actions: &[ActionRecord]) -> String {
    if actions.is_empty() {
        return String::new();
    }
    let mut text = String::from("\nActions (delivery does not change the outcome):\n");
    for action in actions {
        let _ = writeln!(text, "  {}", action_line(action));
    }
    text
}

/// Everything about one action, with the exact arguments to review.
#[must_use]
pub fn action(action: &ActionRecord) -> String {
    let mut text = format!("{}  run {}\n", action.name, action.run_id);
    let _ = writeln!(text, "State: {}", action_state(action));
    let _ = writeln!(text, "Tool: {} of server {}", action.tool, action.server);
    if let Some(decision) = &action.decision {
        let _ = write!(
            text,
            "Decision: {} at {}",
            decision.verdict.as_str(),
            timestamp(decision.at)
        );
        if let Some(note) = &decision.note {
            let _ = write!(text, ": {note}");
        }
        text.push('\n');
    }
    let _ = writeln!(text, "Attempts: {}", action.attempts);
    if let Some(key) = &action.idempotency_key {
        let _ = writeln!(text, "Idempotency key: {key}");
    }
    if let Some(result) = &action.result {
        let _ = writeln!(text, "Result: {result}");
    }
    if let Some(error) = &action.error {
        let _ = writeln!(text, "Error: {error}");
    }
    let arguments = serde_json::to_string_pretty(&action.arguments).unwrap_or_default();
    let _ = writeln!(text, "Digest: {}\nArguments:\n{arguments}", action.digest);
    text.push_str(&history(&action.history));
    text
}

/// An action's history, one line per entry.
fn history(entries: &[HistoryEntry]) -> String {
    let mut text = String::new();
    if !entries.is_empty() {
        text.push_str("History:\n");
    }
    for entry in entries {
        let _ = write!(
            text,
            "  {}  {} -> {} (attempt {})",
            timestamp(entry.at),
            entry.event.as_str(),
            entry.state,
            entry.attempt
        );
        if let Some(detail) = &entry.detail {
            let _ = write!(text, ": {}", first_line(detail));
        }
        text.push('\n');
    }
    text
}

/// A run's state in a few lines.
#[must_use]
pub fn summary(run: &RunSummary) -> String {
    let mut text = format!("{}  {}\n", run.id, run.plan_title);
    let _ = writeln!(
        text,
        "  environment {}, policy {}",
        run.environment, run.policy
    );
    text.push_str(&state(run));
    if let Some(at) = run.cancel_requested_at {
        let status = if run.lifecycle == Lifecycle::Finished {
            "completed"
        } else {
            "pending"
        };
        let _ = writeln!(
            text,
            "  cancellation requested at {}, {status}",
            timestamp(at)
        );
    }
    if let Some(outcome) = run.outcome {
        let reason = run.reason.as_deref().unwrap_or_default();
        let _ = writeln!(text, "  outcome {outcome}: {reason}");
    }
    text
}

/// "running (unit 1.2), 1 of 3 units done: 1 passed".
fn state(run: &RunSummary) -> String {
    let mut state = format!("  {}", run.lifecycle);
    if let Some(unit) = &run.current_unit {
        let _ = write!(state, " (unit {unit})");
    }
    let done = run.counts.total();
    let _ = writeln!(
        state,
        ", {done} of {} units done{}",
        run.total_units,
        counts(run)
    );
    state
}

/// "2 passed, 1 failed", leaving out outcomes no unit has.
fn nonzero(counts: &OutcomeCounts) -> Vec<String> {
    counts
        .by_outcome()
        .iter()
        .filter(|(_, count)| *count > 0)
        .map(|(outcome, count)| format!("{count} {outcome}"))
        .collect()
}

fn counts(run: &RunSummary) -> String {
    let parts = nonzero(&run.counts);
    if parts.is_empty() {
        String::new()
    } else {
        format!(": {}", parts.join(", "))
    }
}

fn timestamp(at: time::OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// The full report: every unit, attempt, criterion and observation.
#[must_use]
pub fn report(report: &RunReport) -> String {
    let mut text = heading(&report.header);
    match report.outcome {
        Some(outcome) => {
            let reason = report.reason.as_deref().unwrap_or_default();
            let _ = writeln!(text, "Outcome: {outcome}: {reason}");
        }
        None => {
            let _ = writeln!(text, "Not finished: {}", report.lifecycle);
        }
    }
    let parts = nonzero(&report.counts);
    if !parts.is_empty() {
        let _ = writeln!(text, "Units: {}", parts.join(", "));
    }
    for unit in &report.units {
        text.push('\n');
        text.push_str(&unit_report(unit, report));
    }
    if !report.cleanup_not_run.is_empty() {
        let keys: Vec<String> = report
            .cleanup_not_run
            .iter()
            .map(ToString::to_string)
            .collect();
        let _ = writeln!(text, "\nCleanup not run: {}", keys.join(", "));
    }
    text
}

fn heading(header: &saci_core::run::ReportHeader) -> String {
    format!(
        "Test plan: {} ({})\nEnvironment {}, policy {}, model {}\n",
        header.plan_title, header.run_id, header.environment, header.policy, header.model
    )
}

fn unit_report(unit: &UnitReport, report: &RunReport) -> String {
    let identity = &unit.identity;
    let outcome = unit
        .outcome
        .map_or_else(|| "pending".to_owned(), |o| o.to_string());
    let mut text = format!(
        "{} {} [{}]: {outcome}",
        identity.key,
        identity.title,
        name(&identity.kind)
    );
    if let Some(reason) = &unit.reason {
        let _ = write!(text, ": {reason}");
    }
    text.push('\n');
    for attempt in &unit.attempts {
        text.push_str(&attempt_report(attempt, report));
    }
    text
}

fn attempt_report(attempt: &Attempt, report: &RunReport) -> String {
    let outcome = attempt.outcome.map_or_else(
        || "running".to_owned(),
        |o| format!("{o}{}", timing_note(attempt.timing.as_ref())),
    );
    let mut text = format!("  attempt {}: {outcome}", attempt.number);
    if let Some(assessment) = &attempt.assessment {
        let _ = writeln!(text, ": {}", assessment.explanation);
        for criterion in &assessment.criteria {
            let cited: Vec<String> = criterion.evidence.iter().map(ToString::to_string).collect();
            let _ = writeln!(
                text,
                "    expected result {}: {} ({}; {}): {}",
                criterion.criterion + 1,
                name(&criterion.status),
                name(&criterion.basis),
                cited.join(", "),
                criterion.explanation
            );
        }
    } else if let Some(error) = &attempt.error {
        let _ = writeln!(text, ": {}: {}", name(&error.kind), error.message);
    } else {
        text.push('\n');
    }
    for id in &attempt.evidence {
        if let Some(evidence) = report.evidence.iter().find(|e| &e.id == id) {
            let _ = writeln!(text, "    {}", observation(evidence));
        }
    }
    text
}
