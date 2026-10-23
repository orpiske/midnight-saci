//! Failure investigations: an agent task that finds the probable cause of a
//! failed unit in the source code and reports a bug
//! (`docs/plans/investigations.md`).
//!
//! The backend builds an [`InvestigationSpec`] from a finished run, and a
//! worker in investigation mode produces an [`InvestigationReport`].

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::LimitsOverride;
use crate::evidence::Evidence;
use crate::ids::{EvidenceId, RunId, UnitKey};
use crate::plan::UnitKind;
use crate::run::{RunReport, UnitIdentity, UnitReport};
use crate::status::{Lifecycle, Outcome};
use time::OffsetDateTime;

/// A source repository the agent may clone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    /// Name used in requests, clone paths and code locations.
    pub name: String,
    /// Clone URL.
    pub url: String,
    /// Branch, tag or commit to check out.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// What the repository contains, shown to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The `investigations` section of the backend configuration. Without it,
/// the backend runs no investigations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationsConfig {
    /// The environment whose parameters, secrets and MCP servers the agent
    /// gets, for example a Git token and a bug tracker server.
    pub environment: String,
    /// The repositories requests may select, with their default refs.
    pub repositories: Vec<Repository>,
    /// Worker image for investigations; `worker.image` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Commands run in each clone before the first model request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prepare: Vec<String>,
    /// Extra guidance for the agent.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    /// Limits applied over the environment's limits.
    #[serde(default)]
    pub limits: LimitsOverride,
}

/// What a worker needs to investigate one failed unit. Like the rest of a
/// run specification, it holds no secret values: the evidence is copied
/// from the parent report, which is already redacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationSpec {
    /// The run whose unit failed.
    pub parent_run: RunId,
    /// Title of the parent run's plan.
    pub plan_title: String,
    /// The failed unit: identity, expected results, outcome and attempts.
    pub unit: UnitReport,
    /// The failed unit's authored action.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub action: String,
    /// The evidence that the unit's attempts cite.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// The repositories to clone, with the refs to check out.
    pub repositories: Vec<Repository>,
    /// Commands run in each clone before the first model request; see
    /// [`prepare_command`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prepare: Vec<String>,
    /// Extra guidance for the agent, such as how to use a code map tool and
    /// where to file bugs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
}

/// Key of the one unit of an investigation run. Its single attempt holds
/// the investigation's evidence.
pub const INVESTIGATION_UNIT: &str = "investigation";

impl InvestigationSpec {
    /// The one unit of the investigation run, as the backend lists it in the
    /// initial report and the worker reports it.
    #[must_use]
    pub fn unit_identity(&self) -> UnitIdentity {
        let failed = &self.unit.identity;
        UnitIdentity {
            key: UnitKey::new(INVESTIGATION_UNIT),
            id: INVESTIGATION_UNIT.to_owned(),
            kind: UnitKind::Step,
            title: format!("Investigate {} \"{}\"", failed.key, failed.title),
            phase: "Investigation".to_owned(),
            location: failed.location.clone(),
            cleanup: false,
            criteria: Vec::new(),
        }
    }

    /// Why the failed unit failed: the explanation of its last assessment,
    /// else the error of its last attempt, else the unit's reason.
    #[must_use]
    pub fn failure_reason(&self) -> String {
        let last = self.unit.attempts.last();
        last.and_then(|attempt| attempt.assessment.as_ref())
            .map(|assessment| assessment.explanation.clone())
            .or_else(|| {
                last.and_then(|a| a.error.as_ref())
                    .map(|e| e.message.clone())
            })
            .or_else(|| self.unit.reason.clone())
            .unwrap_or_default()
    }

    /// The environment variables that carry the values of
    /// [`PREPARE_VARIABLES`].
    #[must_use]
    pub fn prepare_values(&self) -> Vec<(&'static str, String)> {
        let failed = &self.unit.identity;
        PREPARE_VARIABLES
            .iter()
            .map(|(name, variable)| {
                let value = match *name {
                    "run.id" => self.parent_run.to_string(),
                    "plan.title" => self.plan_title.clone(),
                    "unit" => failed.key.to_string(),
                    "failure.title" => failed.title.clone(),
                    _ => self.failure_reason(),
                };
                (*variable, value)
            })
            .collect()
    }
}

/// Template variables of `prepare` commands, with the environment variable
/// that carries each value.
pub const PREPARE_VARIABLES: [(&str, &str); 5] = [
    ("run.id", "SACI_INVESTIGATED_RUN"),
    ("plan.title", "SACI_PLAN_TITLE"),
    ("unit", "SACI_UNIT"),
    ("failure.title", "SACI_FAILURE_TITLE"),
    ("failure.reason", "SACI_FAILURE_REASON"),
];

/// Replaces each `{{variable}}` of a `prepare` command with a reference to
/// the environment variable that carries its value, such as
/// `${SACI_FAILURE_TITLE}`. The values never become part of the command
/// text, so a value cannot change the command; quote the placeholder where
/// the shell must not split it. Unknown variables stay as written.
#[must_use]
pub fn prepare_command(template: &str) -> String {
    let mut command = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        command.push_str(&rest[..start]);
        let name = after[..end].trim();
        match PREPARE_VARIABLES.iter().find(|(known, _)| *known == name) {
            Some((_, variable)) => {
                command.push_str("${");
                command.push_str(variable);
                command.push('}');
            }
            None => command.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    command.push_str(rest);
    command
}

/// A request to investigate a unit of a finished run, as clients send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationRequest {
    /// The finished run.
    pub run_id: RunId,
    /// The failed unit; the first unsuccessful unit when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<UnitKey>,
    /// Configured repositories to clone; all of them when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceSelection>,
}

/// One configured repository selected by a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSelection {
    /// The repository name from the configuration.
    pub name: String,
    /// The ref to check out instead of the configured one.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

/// How an investigation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationOutcome {
    /// At least one hypothesis is backed by code in a cloned commit.
    Concluded,
    /// The agent found no cause it could locate in the code.
    Inconclusive,
    /// A clone, the model, a tool or a limit prevented the investigation.
    Error,
    /// A user or client stopped the investigation.
    Cancelled,
}

impl InvestigationOutcome {
    /// The serialized name, such as `concluded`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Concluded => "concluded",
            Self::Inconclusive => "inconclusive",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }

    /// The outcome of the investigation run's unit and of the run: a
    /// conclusion passes, the other outcomes keep their names.
    #[must_use]
    pub const fn run_outcome(self) -> Outcome {
        match self {
            Self::Concluded => Outcome::Passed,
            Self::Inconclusive => Outcome::Inconclusive,
            Self::Error => Outcome::Error,
            Self::Cancelled => Outcome::Cancelled,
        }
    }
}

/// How strongly the evidence and the code support a hypothesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// The code clearly explains the evidence.
    High,
    /// The code plausibly explains the evidence.
    Medium,
    /// A lead worth checking.
    Low,
}

/// A cloned repository and the commit that was checked out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// The repository name.
    pub name: String,
    /// The full commit SHA.
    pub commit: String,
}

/// A line range in a cloned commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeLocation {
    /// The repository name.
    pub repository: String,
    /// The commit SHA; must be the cloned commit.
    pub commit: String,
    /// Path relative to the repository root, with `/` separators.
    pub path: String,
    /// First line, from 1.
    pub start_line: u32,
    /// Last line, at least `start_line`.
    pub end_line: u32,
}

/// One probable cause of the failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hypothesis {
    /// The cause, in a few sentences.
    pub statement: String,
    /// How strongly it is supported.
    pub confidence: Confidence,
    /// Evidence that supports it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceId>,
    /// The code that it points to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code: Vec<CodeLocation>,
}

/// A filed or existing bug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bug {
    /// Link to the bug.
    pub url: String,
    /// Its title.
    pub title: String,
}

/// The result of an investigation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationReport {
    /// How it ended.
    pub outcome: InvestigationOutcome,
    /// A short analysis, or why it ended without one.
    pub summary: String,
    /// The cloned repositories.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    /// Probable causes, most likely first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypotheses: Vec<Hypothesis>,
    /// The bug, when one was filed or found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bug: Option<Bug>,
    /// Why no bug was filed, when `bug` is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_bug_reason: Option<String>,
}

impl InvestigationReport {
    /// Checks the rules that do not need the clones: every code location
    /// names a cloned commit and a valid line range, every hypothesis has a
    /// statement, a concluded report has a hypothesis with code, and a
    /// concluded report gives a bug or the reason for none.
    ///
    /// The worker also checks that each path and line range exists at its
    /// commit.
    ///
    /// # Errors
    ///
    /// Returns every problem found, one per entry.
    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.summary.trim().is_empty() {
            problems.push("summary must not be empty".to_owned());
        }
        let cloned: BTreeSet<(&str, &str)> = self
            .sources
            .iter()
            .map(|source| (source.name.as_str(), source.commit.as_str()))
            .collect();
        for (index, hypothesis) in self.hypotheses.iter().enumerate() {
            let at = format!("hypotheses[{index}]");
            if hypothesis.statement.trim().is_empty() {
                problems.push(format!("{at}.statement must not be empty"));
            }
            for (number, location) in hypothesis.code.iter().enumerate() {
                let at = format!("{at}.code[{number}]");
                check_location(&at, location, &cloned, &mut problems);
            }
        }
        if self.outcome == InvestigationOutcome::Concluded {
            self.check_conclusion(&mut problems);
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    fn check_conclusion(&self, problems: &mut Vec<String>) {
        if !self.hypotheses.iter().any(|h| !h.code.is_empty()) {
            problems.push(
                "a concluded investigation needs a hypothesis with a code location".to_owned(),
            );
        }
        let reason = self.no_bug_reason.as_deref().unwrap_or_default();
        if self.bug.is_none() && reason.trim().is_empty() {
            problems.push("give the bug, or no_bug_reason when none was filed".to_owned());
        }
    }
}

/// JSON schema of the `conclude` tool. The worker adds the cloned sources,
/// so the model does not give them.
#[must_use]
pub fn conclusion_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "outcome": {
                "type": "string",
                "enum": ["concluded", "inconclusive"],
                "description": "concluded only when a hypothesis points to code in a cloned commit"
            },
            "summary": {"type": "string", "description": "A short analysis of the failure"},
            "hypotheses": {
                "type": "array",
                "description": "Probable causes, most likely first",
                "items": hypothesis_schema()
            },
            "bug": {
                "type": "object",
                "properties": {"url": {"type": "string"}, "title": {"type": "string"}},
                "required": ["url", "title"],
                "additionalProperties": false
            },
            "no_bug_reason": {"type": "string", "description": "Why no bug was filed, when none was"}
        },
        "required": ["outcome", "summary", "hypotheses"],
        "additionalProperties": false
    })
}

fn hypothesis_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "statement": {"type": "string"},
            "confidence": {"type": "string", "enum": ["high", "medium", "low"]},
            "evidence": {"type": "array", "items": {"type": "string", "pattern": "^ev-[0-9]+$"}},
            "code": {"type": "array", "items": location_schema()}
        },
        "required": ["statement", "confidence", "evidence", "code"],
        "additionalProperties": false
    })
}

fn location_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "repository": {"type": "string"},
            "commit": {"type": "string", "description": "The cloned commit SHA"},
            "path": {"type": "string", "description": "Relative to the repository root"},
            "start_line": {"type": "integer", "minimum": 1},
            "end_line": {"type": "integer", "minimum": 1}
        },
        "required": ["repository", "commit", "path", "start_line", "end_line"],
        "additionalProperties": false
    })
}

fn check_location(
    at: &str,
    location: &CodeLocation,
    cloned: &BTreeSet<(&str, &str)>,
    problems: &mut Vec<String>,
) {
    if !cloned.contains(&(location.repository.as_str(), location.commit.as_str())) {
        problems.push(format!(
            "{at}: `{}` at {} is not a cloned commit",
            location.repository, location.commit
        ));
    }
    let path = location.path.as_str();
    let escapes = path.split('/').any(|part| part == ".." || part.is_empty());
    if path.starts_with('/') || escapes {
        problems.push(format!("{at}.path must be relative to the repository root"));
    }
    if location.start_line == 0 || location.end_line < location.start_line {
        problems.push(format!(
            "{at}: lines must start at 1 and end at or after start_line"
        ));
    }
}

/// Outcomes of a unit that can be investigated.
pub const INVESTIGATED_OUTCOMES: [Outcome; 3] = [Outcome::Failed, Outcome::Error, Outcome::Blocked];

/// A clone URL or ref: not empty, no whitespace, and not an option that
/// `git` would read.
#[must_use]
pub fn is_git_value(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('-') && !value.chars().any(char::is_whitespace)
}

/// An investigation as clients see it. Its ID is the ID of the run that
/// executes it, so its events, evidence and report are the run's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Investigation {
    /// The investigation, and the run that executes it.
    pub id: RunId,
    /// The investigated run.
    pub parent_run: RunId,
    /// The investigated unit.
    pub unit: UnitKey,
    /// Title of the investigated run's plan.
    pub plan_title: String,
    /// Title of the investigated unit.
    pub unit_title: String,
    /// Where the investigation is in its lifecycle.
    pub lifecycle: Lifecycle,
    /// How it ended, once finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<InvestigationOutcome>,
    /// Why it ended, once finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The repositories it clones, with the selected refs.
    pub repositories: Vec<Repository>,
    /// When it was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When it finished.
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<OffsetDateTime>,
    /// The agent's result, when it concluded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<InvestigationReport>,
}

impl Investigation {
    /// The investigation that `spec` describes, in the state of `report`,
    /// the report of the run that executes it.
    #[must_use]
    pub fn new(spec: &InvestigationSpec, report: &RunReport) -> Self {
        let result = report.investigation.as_deref().cloned();
        let outcome = match (&result, report.outcome) {
            (Some(result), _) => result.outcome,
            (None, Some(Outcome::Cancelled)) => InvestigationOutcome::Cancelled,
            (None, _) => InvestigationOutcome::Error,
        };
        let outcome = (report.lifecycle == Lifecycle::Finished).then_some(outcome);
        Self {
            id: report.header.run_id.clone(),
            parent_run: spec.parent_run.clone(),
            unit: spec.unit.identity.key.clone(),
            plan_title: spec.plan_title.clone(),
            unit_title: spec.unit.identity.title.clone(),
            lifecycle: report.lifecycle,
            outcome,
            reason: report.reason.clone(),
            repositories: spec.repositories.clone(),
            created_at: report.header.created_at,
            finished_at: report.finished_at,
            result,
        }
    }
}

/// A run's investigations, as `GET /runs/{run_id}/investigations` returns
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationList {
    /// The investigations of the run, oldest first.
    pub items: Vec<Investigation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(commit: &str) -> CodeLocation {
        CodeLocation {
            repository: "orders".into(),
            commit: commit.into(),
            path: "src/orders.rs".into(),
            start_line: 10,
            end_line: 14,
        }
    }

    fn concluded() -> InvestigationReport {
        InvestigationReport {
            outcome: InvestigationOutcome::Concluded,
            summary: "The handler drops the currency.".into(),
            sources: vec![Source {
                name: "orders".into(),
                commit: "abc123".into(),
            }],
            hypotheses: vec![Hypothesis {
                statement: "`total` ignores the currency field.".into(),
                confidence: Confidence::High,
                evidence: vec![EvidenceId::from_index(3)],
                code: vec![location("abc123")],
            }],
            bug: Some(Bug {
                url: "https://example.com/issues/7".into(),
                title: "Totals ignore the currency".into(),
            }),
            no_bug_reason: None,
        }
    }

    #[test]
    fn a_concluded_report_with_cloned_code_passes() {
        assert_eq!(concluded().check(), Ok(()));
    }

    #[test]
    fn a_conclusion_needs_code_from_a_cloned_commit() {
        let mut report = concluded();
        report.hypotheses[0].code = vec![location("def456")];
        let problems = report.check().unwrap_err();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("not a cloned commit"));

        report.hypotheses[0].code.clear();
        let problems = report.check().unwrap_err();
        assert_eq!(
            problems,
            ["a concluded investigation needs a hypothesis with a code location"]
        );

        report.outcome = InvestigationOutcome::Inconclusive;
        assert_eq!(report.check(), Ok(()));
    }

    #[test]
    fn locations_stay_in_the_repository_and_have_lines() {
        let mut report = concluded();
        let code = &mut report.hypotheses[0].code[0];
        code.path = "../etc/passwd".into();
        code.start_line = 0;
        let problems = report.check().unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    #[test]
    fn a_conclusion_gives_a_bug_or_the_reason_for_none() {
        let mut report = concluded();
        report.bug = None;
        assert!(report.check().is_err());
        report.no_bug_reason = Some("The cause is in a dependency.".into());
        assert_eq!(report.check(), Ok(()));
    }

    #[test]
    fn prepare_values_stay_out_of_the_command() {
        assert_eq!(
            prepare_command(r#"ripwire . --for="{{ failure.title }}: {{failure.reason}}""#),
            r#"ripwire . --for="${SACI_FAILURE_TITLE}: ${SACI_FAILURE_REASON}""#
        );
        assert_eq!(
            prepare_command("echo {{unknown}} {{unit}} {{open"),
            "echo {{unknown}} ${SACI_UNIT} {{open"
        );
    }

    #[test]
    fn requests_use_ref_and_reject_unknown_fields() {
        let request: InvestigationRequest = serde_json::from_str(
            r#"{"run_id":"run-1a","unit":"1.2","sources":[{"name":"orders","ref":"v2.4.1"}]}"#,
        )
        .unwrap();
        assert_eq!(request.sources[0].git_ref.as_deref(), Some("v2.4.1"));
        assert!(
            serde_json::from_str::<InvestigationRequest>(r#"{"run_id":"run-1a","url":"x"}"#)
                .is_err()
        );
    }
}
