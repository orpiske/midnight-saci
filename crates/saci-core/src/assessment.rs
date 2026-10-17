//! The structured verdict the agent submits for one attempt.

use serde::{Deserialize, Serialize};

use crate::ids::EvidenceId;
use crate::status::Outcome;

/// A verdict for one attempt of one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
