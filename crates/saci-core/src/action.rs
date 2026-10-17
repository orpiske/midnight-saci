//! Outcome actions: MCP tool calls the backend makes after a run's report
//! is durable, selected by the run's policy (`docs/actions.md`).
//!
//! An action's arguments are a template. Strings may contain `{{name}}`
//! variables, which are filled from the finished report only: its outcome,
//! unit results and the evidence they cite, all already redacted. Nothing
//! else about the run, its environment or its secrets can reach an action.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::evidence::{Evidence, EvidenceRecord, HttpEvidence};
use crate::ids::{EvidenceId, RunId};
use crate::run::{RunReport, UnitReport};
use crate::status::Outcome;

/// Template variables an action's arguments may use.
pub const VARIABLES: [&str; 14] = [
    "run.id",
    "run.outcome",
    "run.reason",
    "run.counts",
    "run.created_at",
    "run.finished_at",
    "plan.title",
    "plan.entry",
    "environment",
    "policy",
    "model",
    "summary",
    "failures",
    "evidence",
];

/// Longest text an evidence excerpt keeps of one captured output.
const EXCERPT: usize = 1000;
/// Longest `evidence` variable.
const EVIDENCE_LIMIT: usize = 16 * 1024;

/// Whether a person must approve an action before it is dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    /// Dispatched as soon as the report is durable.
    #[default]
    None,
    /// Recorded as pending until approved.
    Required,
}

/// One configured outcome action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDefinition {
    /// Server under `actions.servers`.
    pub server: String,
    /// Tool of that server.
    pub tool: String,
    /// Run outcomes that trigger the action.
    pub on: Vec<Outcome>,
    /// Whether the action waits for approval.
    #[serde(default)]
    pub approval: Approval,
    /// The tool's arguments, as a template.
    #[serde(default = "empty_object")]
    pub arguments: Value,
    /// The argument that carries Saci's stable idempotency key for this
    /// run and action ([`idempotency_key`]). Set it only when the tool
    /// deduplicates calls on that argument: Saci then replays a call whose
    /// answer was lost instead of leaving its delivery unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// The idempotency key a run's action sends: the same on every attempt.
#[must_use]
pub fn idempotency_key(run: &str, name: &str) -> String {
    format!("saci-{run}-{name}")
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

impl ActionDefinition {
    /// Whether a run that ended with `outcome` triggers this action.
    #[must_use]
    pub fn triggered_by(&self, outcome: Outcome) -> bool {
        self.on.contains(&outcome)
    }
}

/// Where an action is in its delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    /// Waiting for a person to approve it.
    PendingApproval,
    /// Waiting to be dispatched.
    Queued,
    /// Claimed by the dispatcher; the call may be in progress.
    Executing,
    /// A person rejected it; it is never dispatched.
    Rejected,
    /// The tool returned a successful result.
    Completed,
    /// Delivery failed; the result or reason says why.
    Error,
    /// The call may have reached the server, but no answer was recorded.
    /// It waits for reconciliation or an explicit redispatch.
    Unknown,
}

impl ActionState {
    /// The state's wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingApproval => "pending_approval",
            Self::Queued => "queued",
            Self::Executing => "executing",
            Self::Rejected => "rejected",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Unknown => "unknown",
        }
    }

    /// Parses a wire name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::PendingApproval,
            Self::Queued,
            Self::Executing,
            Self::Rejected,
            Self::Completed,
            Self::Error,
            Self::Unknown,
        ]
        .into_iter()
        .find(|state| state.as_str() == text)
    }
}

impl std::fmt::Display for ActionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A person's decision on an approval-required action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Dispatch the recorded arguments.
    Approved,
    /// Never dispatch it.
    Rejected,
}

impl Verdict {
    /// The verdict's wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

/// A recorded decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Approved or rejected.
    pub verdict: Verdict,
    /// When it was recorded.
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    /// The reviewer's note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Body of an approval or rejection request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRequest {
    /// The digest of the arguments the reviewer saw. A decision on any
    /// other payload is refused.
    pub digest: String,
    /// A note kept with the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Body of a reconciliation: what a person found the action's delivery to
/// be, after checking the remote system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileRequest {
    /// `completed` when the server acted on the call, `error` when it did
    /// not.
    pub state: ActionState,
    /// What was checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What happened to an action, as kept in its history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionEvent {
    /// Recorded with the final report.
    Recorded,
    /// A person approved it.
    Approved,
    /// A person rejected it.
    Rejected,
    /// The dispatcher claimed it for an attempt.
    Claimed,
    /// The server answered with a successful result.
    Answered,
    /// Delivery failed.
    Failed,
    /// Whether the server acted on the call is unknown.
    Uncertain,
    /// Queued again to replay the call with its idempotency key.
    Replayed,
    /// A person queued it again.
    Redispatched,
    /// A person recorded the delivery's outcome.
    Reconciled,
}

impl ActionEvent {
    /// The event's wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Claimed => "claimed",
            Self::Answered => "answered",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
            Self::Replayed => "replayed",
            Self::Redispatched => "redispatched",
            Self::Reconciled => "reconciled",
        }
    }

    /// Parses a wire name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::Recorded,
            Self::Approved,
            Self::Rejected,
            Self::Claimed,
            Self::Answered,
            Self::Failed,
            Self::Uncertain,
            Self::Replayed,
            Self::Redispatched,
            Self::Reconciled,
        ]
        .into_iter()
        .find(|event| event.as_str() == text)
    }
}

/// One entry of an action's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// When it happened.
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    /// What happened.
    pub event: ActionEvent,
    /// The action's state afterwards.
    pub state: ActionState,
    /// Dispatch attempts so far.
    pub attempt: u32,
    /// The reason, error or note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// An action of a run, as recorded and served by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRecord {
    /// The run.
    pub run_id: RunId,
    /// Configured action name.
    pub name: String,
    /// Action server.
    pub server: String,
    /// Tool called.
    pub tool: String,
    /// Whether approval was required.
    pub approval: Approval,
    /// Current state.
    pub state: ActionState,
    /// The exact arguments sent (or to be sent), rendered when the report
    /// became durable.
    pub arguments: Value,
    /// Hex SHA-256 of the canonical arguments.
    pub digest: String,
    /// The idempotency key in the arguments, when the action has one: a
    /// call whose answer was lost is then replayed with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// The approval decision, once made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Decision>,
    /// Dispatch attempts so far.
    pub attempts: u32,
    /// When it was recorded.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When its state last changed.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// The tool's result text, once it answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Why delivery failed or is uncertain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What happened to the action, oldest first. Served for a single
    /// action; lists leave it empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<HistoryEntry>,
}

/// Actions, as `GET /runs/{run_id}/actions` and `GET /actions` return them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionList {
    /// The actions: a run's in name order, or across runs oldest first.
    pub items: Vec<ActionRecord>,
}

/// The `{{name}}` variables used in a template, in order of appearance.
#[must_use]
pub fn variables(template: &Value) -> Vec<String> {
    let mut found = Vec::new();
    visit_strings(template, &mut |text| {
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else { break };
            found.push(after[..end].trim().to_owned());
            rest = &after[end + 2..];
        }
    });
    found
}

fn visit_strings(value: &Value, visit: &mut impl FnMut(&str)) {
    match value {
        Value::String(text) => visit(text),
        Value::Array(items) => items.iter().for_each(|item| visit_strings(item, visit)),
        Value::Object(map) => map.values().for_each(|item| visit_strings(item, visit)),
        _ => {}
    }
}

/// The values of every variable for a finished report.
#[must_use]
pub fn context(report: &RunReport) -> BTreeMap<&'static str, String> {
    let time = |at: Option<OffsetDateTime>| at.map(format_time).unwrap_or_default();
    let header = &report.header;
    let values = [
        header.run_id.to_string(),
        report.outcome.map(|o| o.to_string()).unwrap_or_default(),
        report.reason.clone().unwrap_or_default(),
        counts(report),
        format_time(header.created_at),
        time(report.finished_at),
        header.plan_title.clone(),
        header.entry.clone(),
        header.environment.clone(),
        header.policy.clone(),
        header.model.clone(),
        summary(report, false),
        summary(report, true),
        evidence(report),
    ];
    VARIABLES.into_iter().zip(values).collect()
}

/// `template` with every `{{name}}` replaced from `context`.
#[must_use]
pub fn render(template: &Value, context: &BTreeMap<&'static str, String>) -> Value {
    match template {
        Value::String(text) => Value::String(fill(text, context)),
        Value::Array(items) => Value::Array(items.iter().map(|i| render(i, context)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), render(value, context)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn fill(text: &str, context: &BTreeMap<&'static str, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str("{{");
            rest = after;
            continue;
        };
        match context.get(after[..end].trim()) {
            Some(value) => out.push_str(value),
            None => out.push_str(&rest[start..start + end + 4]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

fn format_time(at: OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn counts(report: &RunReport) -> String {
    let parts: Vec<String> = report
        .counts
        .by_outcome()
        .iter()
        .filter(|(_, count)| *count > 0)
        .map(|(outcome, count)| format!("{count} {outcome}"))
        .collect();
    parts.join(", ")
}

/// One line per unit (or per unsuccessful unit, with its criteria).
fn summary(report: &RunReport, failures_only: bool) -> String {
    let mut text = String::new();
    for unit in &report.units {
        let Some(outcome) = unit.outcome else {
            continue;
        };
        if !failures_only || outcome.is_unsuccessful() {
            unit_line(&mut text, unit, outcome, failures_only);
        }
    }
    text
}

fn unit_line(text: &mut String, unit: &UnitReport, outcome: Outcome, criteria: bool) {
    let identity = &unit.identity;
    let _ = write!(text, "- [{outcome}] {} {}", identity.id, identity.title);
    let attempt = unit.attempts.last();
    if let Some(assessment) = attempt.and_then(|a| a.assessment.as_ref()) {
        let _ = write!(text, ": {}", assessment.explanation);
        for criterion in assessment.criteria.iter().filter(|_| criteria) {
            let _ = write!(
                text,
                "\n  - criterion {} {:?}: {}",
                criterion.criterion + 1,
                criterion.status,
                criterion.explanation
            );
        }
    } else if let Some(error) = attempt.and_then(|a| a.error.as_ref()) {
        let _ = write!(text, ": {}", error.message);
    } else if let Some(reason) = &unit.reason {
        let _ = write!(text, ": {reason}");
    }
    text.push('\n');
}

/// The evidence cited by unsuccessful units' assessments.
fn evidence(report: &RunReport) -> String {
    let cited: Vec<&EvidenceId> = report
        .units
        .iter()
        .filter(|unit| unit.outcome.is_some_and(Outcome::is_unsuccessful))
        .filter_map(|unit| unit.attempts.last()?.assessment.as_ref())
        .flat_map(|assessment| assessment.criteria.iter().flat_map(|c| &c.evidence))
        .collect();
    let mut text = String::new();
    let mut seen = Vec::new();
    for id in cited {
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        if let Some(record) = report.evidence.iter().find(|e| &e.id == id) {
            text.push_str(&excerpt(record));
        }
        if text.len() > EVIDENCE_LIMIT {
            text.truncate(floor_boundary(&text, EVIDENCE_LIMIT));
            text.push_str("\n[further evidence omitted]\n");
            break;
        }
    }
    text
}

fn short(text: &str) -> String {
    if text.len() <= EXCERPT {
        text.to_owned()
    } else {
        format!("{}…", &text[..floor_boundary(text, EXCERPT)])
    }
}

fn excerpt(evidence: &Evidence) -> String {
    let detail = match &evidence.record {
        EvidenceRecord::Shell(shell) => format!(
            "shell `{}` exit {}\nstdout: {}\nstderr: {}",
            shell.command,
            shell
                .exit_code
                .map_or_else(|| "none".to_owned(), |c| c.to_string()),
            short(&shell.stdout.text),
            short(&shell.stderr.text)
        ),
        EvidenceRecord::Http(http) => http_excerpt(http),
        EvidenceRecord::Mcp(mcp) => format!(
            "mcp {}/{}\n{}",
            mcp.server,
            mcp.tool,
            mcp.protocol_error
                .clone()
                .unwrap_or_else(|| short(&mcp.result.text))
        ),
        EvidenceRecord::Rejected(call) => format!("rejected {}: {}", call.name, call.reason),
    };
    format!("{} ({}): {detail}\n", evidence.id, evidence.unit)
}

fn http_excerpt(http: &HttpEvidence) -> String {
    match &http.response {
        Some(response) => format!(
            "http {} {} -> {}\n{}",
            http.method,
            http.url,
            response.status,
            short(&response.body.text)
        ),
        None => format!(
            "http {} {} -> {}",
            http.method,
            http.url,
            http.transport_error.as_deref().unwrap_or("no response")
        ),
    }
}

const fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{VARIABLES, fill, render, variables};

    #[test]
    fn templates_fill_known_variables_only() {
        let template = json!({
            "title": "{{ plan.title }}: {{run.outcome}}",
            "labels": ["saci", "{{environment}}"],
            "count": 3,
            "note": "{{secret}} and {{unclosed"
        });
        assert_eq!(
            variables(&template),
            ["plan.title", "run.outcome", "environment", "secret"]
        );
        let context = [
            ("plan.title", "Orders".to_owned()),
            ("run.outcome", "failed".to_owned()),
            ("environment", "staging".to_owned()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            render(&template, &context),
            json!({
                "title": "Orders: failed",
                "labels": ["saci", "staging"],
                "count": 3,
                "note": "{{secret}} and {{unclosed"
            })
        );
        assert_eq!(fill("{{}}", &context), "{{}}");
        assert!(VARIABLES.contains(&"evidence"));
    }
}
