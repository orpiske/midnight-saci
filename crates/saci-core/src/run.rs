//! Attempts, unit results and the run report, built by applying events.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::assessment::{Assessment, Recorded};
use crate::event::{EventKind, RunEvent};
use crate::evidence::Evidence;
use crate::ids::{EvidenceId, RunId, UnitKey};
use crate::plan::{Criterion, SourceLocation, UnitKind};
use crate::spec::SCHEMA_VERSION;
use crate::status::{Lifecycle, Outcome, summarize};

/// Why execution could not complete reliably.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The model provider failed (authentication, quota, rate limit,
    /// malformed response, unsupported tool calling, transport).
    Provider,
    /// The agent did not produce a valid assessment.
    InvalidAssessment,
    /// A tool's infrastructure failed (not the tested target).
    Tool,
    /// An execution limit was exhausted.
    LimitExceeded,
    /// The worker could not be launched.
    LaunchFailed,
    /// The worker disappeared or the backend lost track of it.
    WorkerLost,
    /// The run specification was rejected by the worker.
    InvalidSpecification,
}

/// An execution error with an actionable, redacted message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionError {
    /// Error category.
    pub kind: ErrorKind,
    /// Human-readable explanation.
    pub message: String,
}

impl ExecutionError {
    /// Creates an error.
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// One attempt of one unit. Every attempt stays in the report, so a pass on
/// a retry remains distinguishable from a first-attempt pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    /// One-based attempt number.
    pub number: u32,
    /// When it began.
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    /// When it finished.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    /// Its outcome, once finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// The validated assessment, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<Assessment>,
    /// The execution error, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ExecutionError>,
    /// Evidence recorded during the attempt.
    #[serde(default)]
    pub evidence: Vec<EvidenceId>,
}

/// The planned identity of one execution unit, known before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitIdentity {
    /// Run-unique key.
    pub key: UnitKey,
    /// Authored ID.
    pub id: String,
    /// Test or step.
    pub kind: UnitKind,
    /// Authored title.
    pub title: String,
    /// Title of the enclosing phase.
    pub phase: String,
    /// Location of the unit heading.
    pub location: SourceLocation,
    /// Whether the unit or its phase is a cleanup/teardown section.
    pub cleanup: bool,
    /// The authored expected results an assessment must cover.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<Criterion>,
}

/// The result of one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitReport {
    /// Which unit.
    #[serde(flatten)]
    pub identity: UnitIdentity,
    /// Final outcome, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Why the unit was skipped, if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// All attempts, in order.
    #[serde(default)]
    pub attempts: Vec<Attempt>,
}

impl UnitReport {
    /// A unit that has not started.
    #[must_use]
    pub const fn pending(identity: UnitIdentity) -> Self {
        Self {
            identity,
            outcome: None,
            reason: None,
            attempts: Vec::new(),
        }
    }
}

/// Per-outcome unit counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OutcomeCounts {
    /// Passed units.
    pub passed: u32,
    /// Failed units.
    pub failed: u32,
    /// Blocked units.
    pub blocked: u32,
    /// Inconclusive units.
    pub inconclusive: u32,
    /// Units that ended in an execution error.
    pub error: u32,
    /// Units that were not attempted.
    pub skipped: u32,
    /// Cancelled units.
    pub cancelled: u32,
}

impl OutcomeCounts {
    /// Each outcome with its count, in precedence order.
    #[must_use]
    pub const fn by_outcome(&self) -> [(Outcome, u32); 7] {
        [
            (Outcome::Passed, self.passed),
            (Outcome::Failed, self.failed),
            (Outcome::Blocked, self.blocked),
            (Outcome::Inconclusive, self.inconclusive),
            (Outcome::Error, self.error),
            (Outcome::Skipped, self.skipped),
            (Outcome::Cancelled, self.cancelled),
        ]
    }

    /// Units that have an outcome.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.by_outcome().iter().map(|(_, count)| count).sum()
    }

    const fn add(&mut self, outcome: Outcome) {
        let slot = match outcome {
            Outcome::Passed => &mut self.passed,
            Outcome::Failed => &mut self.failed,
            Outcome::Blocked => &mut self.blocked,
            Outcome::Inconclusive => &mut self.inconclusive,
            Outcome::Error => &mut self.error,
            Outcome::Skipped => &mut self.skipped,
            Outcome::Cancelled => &mut self.cancelled,
        };
        *slot += 1;
    }
}

/// Descriptive header of a report, fixed at submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportHeader {
    /// The run.
    pub run_id: RunId,
    /// Plan title.
    pub plan_title: String,
    /// Entry document path.
    pub entry: String,
    /// Environment name.
    pub environment: String,
    /// Policy name.
    pub policy: String,
    /// Configured model identifier.
    pub model: String,
    /// When the run was accepted.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// The container image that ran a run's worker, as the runtime reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerImage {
    /// Image reference as configured in `worker.image`, with its tag.
    pub reference: String,
    /// Registry digest (`sha256:<hex>`), when the image came from a registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Local image ID (`sha256:<hex>`), when the runtime reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl WorkerImage {
    /// Whether the runtime has identified the exact image.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        self.digest.is_some() || self.id.is_some()
    }
}

/// The durable, redacted report of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    /// Format version.
    pub schema_version: u32,
    /// Descriptive header.
    #[serde(flatten)]
    pub header: ReportHeader,
    /// Current lifecycle state.
    pub lifecycle: Lifecycle,
    /// Summary outcome, once finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Why the run ended this way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When the worker started executing.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    /// When the run finished.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    /// Per-outcome counts over units.
    pub counts: OutcomeCounts,
    /// Unit results in execution order.
    pub units: Vec<UnitReport>,
    /// Cleanup units that did not run.
    #[serde(default)]
    pub cleanup_not_run: Vec<UnitKey>,
    /// All recorded evidence.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// The worker's container image, once the backend has launched it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_image: Option<WorkerImage>,
    /// Sequence number of the last applied event.
    pub last_seq: u64,
}

/// An event could not be applied to a report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    /// The event belongs to another run.
    #[error("event for run {found} applied to run {expected}")]
    WrongRun {
        /// The report's run.
        expected: RunId,
        /// The event's run.
        found: RunId,
    },
    /// The event is not the next in sequence.
    #[error("expected event {expected}, received {found}")]
    OutOfOrder {
        /// The next expected sequence number.
        expected: u64,
        /// The received sequence number.
        found: u64,
    },
    /// The run already finished.
    #[error("run already finished")]
    Finished,
    /// The event names a unit that is not part of the run.
    #[error("unknown unit `{0}`")]
    UnknownUnit(UnitKey),
    /// The event is not valid in the unit's current state.
    #[error("invalid event for unit `{unit}`: {reason}")]
    InvalidUnitEvent {
        /// The unit concerned.
        unit: UnitKey,
        /// What was wrong.
        reason: String,
    },
    /// The lifecycle transition is not permitted.
    #[error("cannot move from {from} to {to}")]
    InvalidTransition {
        /// Current state.
        from: Lifecycle,
        /// Requested state.
        to: Lifecycle,
    },
}

impl RunReport {
    /// Creates the report of an accepted run with every unit pending.
    #[must_use]
    pub fn new(header: ReportHeader, units: Vec<UnitIdentity>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            header,
            lifecycle: Lifecycle::Accepted,
            outcome: None,
            reason: None,
            started_at: None,
            finished_at: None,
            counts: OutcomeCounts::default(),
            units: units.into_iter().map(UnitReport::pending).collect(),
            cleanup_not_run: Vec::new(),
            evidence: Vec::new(),
            worker_image: None,
            last_seq: 0,
        }
    }

    /// Moves the run to another lifecycle state.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidTransition`] for a backward or
    /// post-terminal move.
    pub const fn transition(&mut self, to: Lifecycle) -> Result<(), ContractError> {
        if !self.lifecycle.can_transition_to(to) {
            return Err(ContractError::InvalidTransition {
                from: self.lifecycle,
                to,
            });
        }
        self.lifecycle = to;
        Ok(())
    }

    /// Applies the next worker event.
    ///
    /// # Errors
    ///
    /// Returns a [`ContractError`] when the event belongs to another run, is
    /// out of order, arrives after the run finished, or is inconsistent with
    /// the unit's state. The report is unchanged on error.
    pub fn apply(&mut self, event: &RunEvent) -> Result<(), ContractError> {
        self.check_sequence(event)?;
        // Every handler validates before it mutates, so an error leaves the
        // report unchanged.
        self.apply_kind(event)?;
        self.last_seq = event.seq;
        Ok(())
    }

    fn check_sequence(&self, event: &RunEvent) -> Result<(), ContractError> {
        if event.run_id != self.header.run_id {
            return Err(ContractError::WrongRun {
                expected: self.header.run_id.clone(),
                found: event.run_id.clone(),
            });
        }
        if self.lifecycle.is_terminal() {
            return Err(ContractError::Finished);
        }
        if event.seq != self.last_seq + 1 {
            return Err(ContractError::OutOfOrder {
                expected: self.last_seq + 1,
                found: event.seq,
            });
        }
        Ok(())
    }

    fn apply_kind(&mut self, event: &RunEvent) -> Result<(), ContractError> {
        match &event.kind {
            EventKind::RunStarted { .. } => self.start(event.at),
            EventKind::CancellationRequested => self.stop(),
            EventKind::RunFinished { outcome, reason } => {
                self.finish(*outcome, reason, event.at);
                Ok(())
            }
            EventKind::Observation { evidence } => self.observe(event, evidence),
            EventKind::AttemptStarted
            | EventKind::AttemptFinished { .. }
            | EventKind::UnitFinished { .. }
            | EventKind::UnitSkipped { .. } => self.apply_unit_event(event),
        }
    }

    fn start(&mut self, at: OffsetDateTime) -> Result<(), ContractError> {
        if self.lifecycle == Lifecycle::Accepted {
            self.transition(Lifecycle::Starting)?;
        }
        self.transition(Lifecycle::Running)?;
        self.started_at = Some(at);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), ContractError> {
        if self.lifecycle == Lifecycle::Stopping {
            return Ok(());
        }
        self.transition(Lifecycle::Stopping)
    }

    /// Finishes the run: pending units become skipped, the summary outcome is
    /// derived from unit outcomes unless `outcome` has higher precedence, and
    /// cleanup units that did not run are listed. Does nothing if the run
    /// already finished.
    pub fn finish(&mut self, outcome: Outcome, reason: &str, at: OffsetDateTime) {
        if self.lifecycle.is_terminal() {
            return;
        }
        for unit in &mut self.units {
            close_interrupted(unit, outcome, reason, at);
        }
        self.recount();
        let derived = summarize(self.units.iter().filter_map(|unit| unit.outcome));
        let summary = if outcome.precedence() > derived.precedence() {
            outcome
        } else {
            derived
        };
        self.outcome = Some(summary);
        self.reason = Some(reason.to_owned());
        self.finished_at = Some(at);
        self.lifecycle = Lifecycle::Finished;
    }

    fn recount(&mut self) {
        self.counts = OutcomeCounts::default();
        self.cleanup_not_run.clear();
        for unit in &self.units {
            if let Some(outcome) = unit.outcome {
                self.counts.add(outcome);
            }
            let ran = unit
                .attempts
                .iter()
                .any(|attempt| attempt.outcome.is_some());
            if unit.identity.cleanup && !ran {
                self.cleanup_not_run.push(unit.identity.key.clone());
            }
        }
    }

    fn observe(&mut self, event: &RunEvent, evidence: &Evidence) -> Result<(), ContractError> {
        let unit = self.open_attempt(event)?;
        unit.evidence.push(evidence.id.clone());
        self.evidence.push(evidence.clone());
        Ok(())
    }

    fn unit_mut(&mut self, event: &RunEvent) -> Result<&mut UnitReport, ContractError> {
        let key = event
            .unit
            .clone()
            .ok_or_else(|| ContractError::InvalidUnitEvent {
                unit: UnitKey::new(""),
                reason: "event has no unit".into(),
            })?;
        self.units
            .iter_mut()
            .find(|unit| unit.identity.key == key)
            .ok_or(ContractError::UnknownUnit(key))
    }

    fn open_attempt(&mut self, event: &RunEvent) -> Result<&mut Attempt, ContractError> {
        let number = event.attempt;
        let unit = self.unit_mut(event)?;
        let key = unit.identity.key.clone();
        unit.attempts
            .last_mut()
            .filter(|attempt| attempt.outcome.is_none() && Some(attempt.number) == number)
            .ok_or_else(|| invalid(&key, "no matching attempt in progress"))
    }

    fn apply_unit_event(&mut self, event: &RunEvent) -> Result<(), ContractError> {
        match &event.kind {
            EventKind::AttemptStarted => self.start_attempt(event),
            EventKind::AttemptFinished {
                outcome,
                assessment,
                error,
            } => {
                self.check_attempt(event, *outcome, assessment.as_ref(), error.is_some())?;
                let attempt = self.open_attempt(event)?;
                attempt.outcome = Some(*outcome);
                attempt.finished_at = Some(event.at);
                attempt.assessment.clone_from(assessment);
                attempt.error.clone_from(error);
                Ok(())
            }
            EventKind::UnitFinished { outcome } => self.finish_unit(event, *outcome, None),
            EventKind::UnitSkipped { reason } => {
                self.finish_unit(event, Outcome::Skipped, Some(reason.clone()))
            }
            _ => Ok(()),
        }
    }

    /// An attempt's outcome follows its assessment, a pass needs one, and
    /// the assessment cites only evidence recorded in the run.
    fn check_attempt(
        &self,
        event: &RunEvent,
        outcome: Outcome,
        assessment: Option<&Assessment>,
        error: bool,
    ) -> Result<(), ContractError> {
        let key = event.unit.clone().unwrap_or_else(|| UnitKey::new(""));
        let expected = crate::assessment::attempt_outcome(assessment, error);
        let interrupted = matches!(outcome, Outcome::Cancelled | Outcome::Error);
        if outcome != expected && !(interrupted && assessment.is_none()) {
            let reason = format!(
                "attempt outcome {} does not follow its assessment",
                outcome.as_str()
            );
            return Err(invalid(&key, &reason));
        }
        let Some(assessment) = assessment else {
            return Ok(());
        };
        let unit = self.units.iter().find(|unit| unit.identity.key == key);
        let unit = unit.ok_or_else(|| ContractError::UnknownUnit(key.clone()))?;
        crate::assessment::validate(assessment, &unit.identity.criteria, &self.recorded()).map_err(
            |problems| {
                invalid(
                    &key,
                    &format!("invalid assessment: {}", problems.join("; ")),
                )
            },
        )
    }

    /// The run's evidence, as assessment validation sees it.
    fn recorded(&self) -> Recorded {
        self.evidence
            .iter()
            .map(|e| (e.id.clone(), e.record.unanswered()))
            .collect()
    }

    fn start_attempt(&mut self, event: &RunEvent) -> Result<(), ContractError> {
        let unit = self.unit_mut(event)?;
        let key = unit.identity.key.clone();
        let expected = u32::try_from(unit.attempts.len()).unwrap_or(u32::MAX) + 1;
        let open = unit.attempts.last().is_some_and(|a| a.outcome.is_none());
        if unit.outcome.is_some() || open || event.attempt != Some(expected) {
            return Err(invalid(&key, &format!("attempt {expected} cannot start")));
        }
        unit.attempts.push(Attempt {
            number: expected,
            started_at: event.at,
            finished_at: None,
            outcome: None,
            assessment: None,
            error: None,
            evidence: Vec::new(),
        });
        Ok(())
    }

    fn finish_unit(
        &mut self,
        event: &RunEvent,
        outcome: Outcome,
        reason: Option<String>,
    ) -> Result<(), ContractError> {
        let unit = self.unit_mut(event)?;
        let key = unit.identity.key.clone();
        let open = unit.attempts.last().is_some_and(|a| a.outcome.is_none());
        if unit.outcome.is_some() || open {
            return Err(invalid(&key, "unit already finished or attempt still open"));
        }
        if outcome == Outcome::Skipped && !unit.attempts.is_empty() {
            return Err(invalid(&key, "a unit that was attempted cannot be skipped"));
        }
        let last = unit.attempts.last().and_then(|a| a.outcome);
        if outcome != Outcome::Skipped && last != Some(outcome) {
            return Err(invalid(
                &key,
                "a unit's outcome is its last attempt's outcome",
            ));
        }
        unit.outcome = Some(outcome);
        unit.reason = reason;
        self.recount();
        Ok(())
    }
}

/// Gives an unfinished unit its final state when the run ends: an attempt
/// still in progress is interrupted (cancelled or error), and a unit that
/// never started is skipped.
fn close_interrupted(unit: &mut UnitReport, outcome: Outcome, reason: &str, at: OffsetDateTime) {
    if unit.outcome.is_some() {
        return;
    }
    let interrupted = if outcome == Outcome::Cancelled {
        Outcome::Cancelled
    } else {
        Outcome::Error
    };
    match unit.attempts.last_mut() {
        Some(attempt) if attempt.outcome.is_none() => {
            attempt.outcome = Some(interrupted);
            attempt.finished_at = Some(at);
            unit.outcome = Some(interrupted);
            unit.reason = Some(format!("interrupted: {reason}"));
        }
        Some(attempt) => unit.outcome = attempt.outcome,
        None => {
            unit.outcome = Some(Outcome::Skipped);
            unit.reason = Some(format!("not attempted: {reason}"));
        }
    }
}

fn invalid(unit: &UnitKey, reason: &str) -> ContractError {
    ContractError::InvalidUnitEvent {
        unit: unit.clone(),
        reason: reason.to_owned(),
    }
}
