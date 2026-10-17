//! The structured verdict the agent submits for one attempt.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ids::EvidenceId;
use crate::plan::Criterion;
use crate::status::Outcome;

/// A verdict for one attempt of one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    /// The attempt's outcome.
    pub outcome: Outcome,
    /// One result per expected criterion, in criterion order.
    pub criteria: Vec<CriterionResult>,
    /// Concise explanation of the verdict.
    pub explanation: String,
}

/// Whether the evidence satisfies one criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionStatus {
    /// Evidence shows the criterion holds.
    Satisfied,
    /// Evidence shows the criterion does not hold.
    Contradicted,
    /// The evidence cannot establish either.
    Undetermined,
}

/// How a criterion result was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// A direct, mechanical check of recorded evidence (an exit code, a
    /// status code, an exact value).
    Mechanical,
    /// The model's interpretation of recorded evidence.
    ModelJudgment,
}

/// The result for one expected criterion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionResult {
    /// Index of the criterion within the unit's expected results.
    pub criterion: usize,
    /// Whether it holds.
    pub status: CriterionStatus,
    /// How that was established.
    pub basis: Basis,
    /// Evidence supporting the result; each must exist in the run.
    pub evidence: Vec<EvidenceId>,
    /// Why the evidence supports the status.
    pub explanation: String,
}

/// Outcomes an assessment may report. `error`, `skipped` and `cancelled`
/// are decided by Saci from what happened, never by the model.
pub const ASSESSABLE: [Outcome; 4] = [
    Outcome::Passed,
    Outcome::Failed,
    Outcome::Blocked,
    Outcome::Inconclusive,
];

/// Checks a submitted assessment against the unit's criteria and the
/// evidence recorded in the run. Returns every problem, phrased so the model
/// can correct its submission.
///
/// # Errors
///
/// Returns the problems when the assessment is incomplete, inconsistent, or
/// cites evidence that does not exist.
pub fn validate(
    assessment: &Assessment,
    criteria: &[Criterion],
    recorded: &BTreeSet<EvidenceId>,
) -> Result<(), Vec<String>> {
    let mut problems = Vec::new();
    if !ASSESSABLE.contains(&assessment.outcome) {
        problems.push(format!(
            "outcome `{}` cannot be assessed; use passed, failed, blocked or inconclusive",
            assessment.outcome.as_str()
        ));
    }
    if assessment.explanation.trim().is_empty() {
        problems.push("the assessment needs an explanation".to_owned());
    }
    check_coverage(assessment, criteria, &mut problems);
    for result in &assessment.criteria {
        check_result(result, recorded, &mut problems);
    }
    check_consistency(assessment, &mut problems);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// Every criterion has exactly one result and no result names an unknown one.
fn check_coverage(assessment: &Assessment, criteria: &[Criterion], problems: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    for result in &assessment.criteria {
        if result.criterion >= criteria.len() {
            problems.push(format!(
                "criterion {} does not exist; the unit has {} expected results (0 to {})",
                result.criterion,
                criteria.len(),
                criteria.len().saturating_sub(1)
            ));
        } else if !seen.insert(result.criterion) {
            problems.push(format!("criterion {} is assessed twice", result.criterion));
        }
    }
    for criterion in criteria.iter().filter(|c| !seen.contains(&c.index)) {
        problems.push(format!(
            "criterion {} (\"{}\") is not assessed",
            criterion.index, criterion.text
        ));
    }
}

fn check_result(
    result: &CriterionResult,
    recorded: &BTreeSet<EvidenceId>,
    problems: &mut Vec<String>,
) {
    for id in result.evidence.iter().filter(|id| !recorded.contains(id)) {
        problems.push(format!(
            "criterion {}: evidence `{id}` was not recorded in this run",
            result.criterion
        ));
    }
    let decided = matches!(
        result.status,
        CriterionStatus::Satisfied | CriterionStatus::Contradicted
    );
    if decided && result.evidence.is_empty() {
        problems.push(format!(
            "criterion {}: a satisfied or contradicted criterion must cite the evidence that shows it",
            result.criterion
        ));
    }
    if result.explanation.trim().is_empty() {
        problems.push(format!(
            "criterion {}: explain the result",
            result.criterion
        ));
    }
}

/// The outcome follows from the criterion results.
fn check_consistency(assessment: &Assessment, problems: &mut Vec<String>) {
    let any = |status| assessment.criteria.iter().any(|r| r.status == status);
    let contradicted = any(CriterionStatus::Contradicted);
    let undetermined = any(CriterionStatus::Undetermined);
    let problem = match assessment.outcome {
        Outcome::Passed if contradicted || undetermined => {
            Some("passed requires every criterion to be satisfied by cited evidence")
        }
        Outcome::Failed if !contradicted => {
            Some("failed requires at least one criterion contradicted by cited evidence")
        }
        Outcome::Blocked | Outcome::Inconclusive if contradicted => {
            Some("a contradicted criterion means the outcome is failed")
        }
        Outcome::Blocked | Outcome::Inconclusive if !undetermined => {
            Some("blocked or inconclusive requires at least one undetermined criterion")
        }
        _ => None,
    };
    problems.extend(problem.map(str::to_owned));
}

/// The outcome of a finished attempt: an execution error wins over any
/// assessment, and an attempt with neither is an error.
#[must_use]
pub const fn attempt_outcome(assessment: Option<&Assessment>, error: bool) -> Outcome {
    match (assessment, error) {
        (Some(assessment), false) => assessment.outcome,
        _ => Outcome::Error,
    }
}

/// JSON schema of the step-assessment tool for a unit with
/// `criteria` expected results.
#[must_use]
pub fn schema(criteria: usize) -> Value {
    let last = criteria.saturating_sub(1);
    json!({
        "type": "object",
        "properties": {
            "outcome": {"type": "string", "enum": ["passed", "failed", "blocked", "inconclusive"]},
            "criteria": {
                "type": "array",
                "description": format!("One result per expected result, criterion 0 to {last}"),
                "items": {
                    "type": "object",
                    "properties": {
                        "criterion": {"type": "integer", "minimum": 0, "maximum": last},
                        "status": {"type": "string", "enum": ["satisfied", "contradicted", "undetermined"]},
                        "basis": {"type": "string", "enum": ["mechanical", "model_judgment"]},
                        "evidence": {"type": "array", "items": {"type": "string", "pattern": "^ev-[0-9]+$"}},
                        "explanation": {"type": "string"}
                    },
                    "required": ["criterion", "status", "basis", "evidence", "explanation"]
                }
            },
            "explanation": {"type": "string"}
        },
        "required": ["outcome", "criteria", "explanation"]
    })
}

#[cfg(test)]
mod tests;
