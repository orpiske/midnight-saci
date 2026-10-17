//! Ordered progress events reported by a worker.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::assessment::Assessment;
use crate::evidence::Evidence;
use crate::ids::{RunId, UnitKey};
use crate::plan::SourceLocation;
use crate::run::ExecutionError;
use crate::status::Outcome;

/// One progress event. `seq` starts at 1 and increases by one per event of a
/// run, so receivers can order events and discard retransmissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEvent {
    /// The run the event belongs to.
    pub run_id: RunId,
    /// Position of the event within the run.
    pub seq: u64,
    /// When the event happened.
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    /// The unit concerned, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<UnitKey>,
    /// The attempt concerned, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// Source location of the unit concerned, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
    /// What happened.
    #[serde(flatten)]
    pub kind: EventKind,
}

/// What a [`RunEvent`] reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// The worker began executing the plan.
    RunStarted {
        /// Configured model identifier.
        model: String,
    },
    /// An attempt of a unit began.
    AttemptStarted,
    /// A tool produced an observation.
    Observation {
        /// The recorded, redacted evidence.
        evidence: Box<Evidence>,
    },
    /// An attempt finished.
    AttemptFinished {
        /// The attempt outcome.
        outcome: Outcome,
        /// The validated assessment, when the agent produced one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assessment: Option<Assessment>,
        /// The execution error, when one prevented an assessment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ExecutionError>,
    },
    /// A unit reached its final outcome.
    UnitFinished {
        /// The unit outcome.
        outcome: Outcome,
    },
    /// A unit was not attempted.
    UnitSkipped {
        /// Why.
        reason: String,
    },
    /// The worker received a cancellation request.
    CancellationRequested,
    /// The run finished. Always the last event.
    RunFinished {
        /// Summary outcome.
        outcome: Outcome,
        /// Why the run ended this way.
        reason: String,
    },
}

impl EventKind {
    /// Whether this event ends the run.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::RunFinished { .. })
    }
}
