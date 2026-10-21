//! Executes a run's units in document order under its policy.
//!
//! Each unit gets up to `policy.retry.max_attempts` attempts, retried only
//! when its outcome is listed in `policy.retry.on`; every attempt stays in
//! the report. After an unsuccessful unit the default policy stops and the
//! remaining units, cleanup included, are skipped with the reason.
//! `on_failure: continue` goes on with the next unit. Cancellation and the
//! run deadline stop execution the same way.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use saci_core::bundle::{ExecutionUnit, ExpandedPlan};
use saci_core::event::EventKind;
use saci_core::ids::UnitKey;
use saci_core::spec::{FailureAction, RunPolicy};
use saci_core::status::{Outcome, summarize};
use tokio_util::sync::CancellationToken;

use crate::agent::{AttemptRequest, AttemptResult, Executor, prompt};
use crate::environment;

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEnding {
    /// Summary outcome reported in `run_finished`.
    pub outcome: Outcome,
    /// Why the run ended this way.
    pub reason: String,
}

/// What a run needs besides its executor.
#[derive(Debug, Clone, Copy)]
pub struct RunPlan<'a> {
    /// The expanded plan.
    pub plan: &'a ExpandedPlan,
    /// The run policy.
    pub policy: &'a RunPolicy,
    /// Deadline of one attempt.
    pub step_timeout: Duration,
    /// When the whole run must end.
    pub deadline: Instant,
    /// The configured model, reported in `run_started`.
    pub model: &'a str,
}

/// Why the remaining units are not attempted.
enum Stop {
    Cancelled,
    Deadline,
    Policy(String),
}

impl Stop {
    fn reason(&self) -> String {
        match self {
            Self::Cancelled => "the run was cancelled".to_owned(),
            Self::Deadline => "the run deadline (limits.run_timeout_secs) passed".to_owned(),
            Self::Policy(reason) => reason.clone(),
        }
    }
}

/// Runs every unit and reports `run_started`, the unit events and
/// `run_finished`.
pub async fn run(executor: &Executor, plan: RunPlan<'_>, cancel: &CancellationToken) -> RunEnding {
    let events = executor.recorder().events();
    let model = plan.model.to_owned();
    let environment = Some(Box::new(environment::collect().await));
    events.emit(None, None, EventKind::RunStarted { model, environment });
    let mut progress = Progress {
        context: prompt::plan_context(&plan.plan.plan),
        done: Vec::new(),
        outcomes: Vec::new(),
        stop: None,
    };
    for unit in &plan.plan.units {
        Box::pin(progress.unit(executor, &plan, unit, cancel)).await;
    }
    let ending = ending(&progress.outcomes, progress.stop.as_ref());
    let kind = EventKind::RunFinished {
        outcome: ending.outcome,
        reason: ending.reason.clone(),
    };
    events.emit(None, None, kind);
    ending
}

/// What the run has done so far.
struct Progress {
    context: String,
    /// Each attempted unit's invoking units and its line in the earlier
    /// results.
    done: Vec<(Vec<UnitKey>, String)>,
    outcomes: Vec<Outcome>,
    stop: Option<Stop>,
}

impl Progress {
    /// Attempts `unit`, or skips it once the run has stopped.
    async fn unit(
        &mut self,
        executor: &Executor,
        plan: &RunPlan<'_>,
        unit: &ExecutionUnit,
        cancel: &CancellationToken,
    ) {
        let events = executor.recorder().events();
        if self.stop.is_none() {
            self.stop = interrupted(plan, cancel);
        }
        if let Some(stop) = &self.stop {
            let reason = format!("not attempted: {}", stop.reason());
            events.emit(
                Some(&unit.identity),
                None,
                EventKind::UnitSkipped { reason },
            );
            self.outcomes.push(Outcome::Skipped);
            return;
        }
        let procedures = self.procedures(unit);
        let context = (self.context.as_str(), procedures.as_slice());
        let result = Box::pin(attempts(executor, plan, unit, context, cancel)).await;
        let outcome = result.outcome;
        events.emit(
            Some(&unit.identity),
            None,
            EventKind::UnitFinished { outcome },
        );
        self.remember(unit, &result);
        self.outcomes.push(outcome);
        self.stop = after(plan, unit, outcome, cancel);
    }

    /// Adds a unit's result to the context later units see.
    fn remember(&mut self, unit: &ExecutionUnit, result: &AttemptResult) {
        let line = summary(unit, result);
        if self.done.is_empty() {
            self.context.push_str("\nEarlier results:\n");
        }
        let _ = writeln!(self.context, "{line}");
        self.done.push((unit.invoked_by.clone(), line));
    }

    /// The results of the procedure units that `unit` invoked, directly or
    /// through other procedures. They ran before it.
    fn procedures(&self, unit: &ExecutionUnit) -> Vec<String> {
        let key = &unit.identity.key;
        self.done
            .iter()
            .filter(|(invoked_by, _)| invoked_by.contains(key))
            .map(|(_, line)| line.clone())
            .collect()
    }
}

fn interrupted(plan: &RunPlan<'_>, cancel: &CancellationToken) -> Option<Stop> {
    if cancel.is_cancelled() {
        Some(Stop::Cancelled)
    } else if Instant::now() >= plan.deadline {
        Some(Stop::Deadline)
    } else {
        None
    }
}

/// Decides whether to go on after `unit` ended with `outcome`.
fn after(
    plan: &RunPlan<'_>,
    unit: &ExecutionUnit,
    outcome: Outcome,
    cancel: &CancellationToken,
) -> Option<Stop> {
    if outcome == Outcome::Cancelled || cancel.is_cancelled() {
        return Some(Stop::Cancelled);
    }
    let stops = outcome.is_unsuccessful() && plan.policy.on_failure == FailureAction::Stop;
    stops.then(|| {
        Stop::Policy(format!(
            "stopped after {} ended {outcome} (policy `{}` stops on failure)",
            unit.identity.key, plan.policy.name
        ))
    })
}

/// Attempts `unit` until an outcome the policy does not retry, or the
/// attempts allowed run out. `context` is the run context and the results of
/// the unit's procedures.
async fn attempts(
    executor: &Executor,
    plan: &RunPlan<'_>,
    unit: &ExecutionUnit,
    (context, procedures): (&str, &[String]),
    cancel: &CancellationToken,
) -> AttemptResult {
    let retry = &plan.policy.retry;
    let mut attempt = 1;
    loop {
        let deadline = (Instant::now() + plan.step_timeout).min(plan.deadline);
        let request = AttemptRequest {
            unit,
            attempt,
            context,
            procedures,
            deadline,
            cancel,
        };
        let result = Box::pin(executor.attempt(request)).await;
        let retried = retry.on.contains(&result.outcome)
            && result.outcome != Outcome::Cancelled
            && attempt < retry.max_attempts
            && interrupted(plan, cancel).is_none();
        if !retried {
            return result;
        }
        tracing::info!(unit = %unit.identity.key, attempt, outcome = %result.outcome, "retrying under the run policy");
        attempt += 1;
    }
}

/// A unit's result as later units see it, with the evidence its assessment
/// cites so they can cite it too.
fn summary(unit: &ExecutionUnit, result: &AttemptResult) -> String {
    let mut detail = result
        .assessment
        .as_ref()
        .map(|a| a.explanation.clone())
        .or_else(|| result.error.as_ref().map(|e| e.message.clone()))
        .unwrap_or_default();
    let mut cited: Vec<String> = Vec::new();
    for criterion in result.assessment.iter().flat_map(|a| &a.criteria) {
        for id in &criterion.evidence {
            let id = id.to_string();
            if !cited.contains(&id) {
                cited.push(id);
            }
        }
    }
    if !cited.is_empty() {
        let _ = write!(detail, "; evidence {}", cited.join(", "));
    }
    let identity = &unit.identity;
    format!(
        "- {} \"{}\": {} ({detail})",
        identity.key, identity.title, result.outcome
    )
}

fn ending(outcomes: &[Outcome], stop: Option<&Stop>) -> RunEnding {
    let derived = summarize(outcomes.iter().copied());
    match stop {
        Some(Stop::Cancelled) => RunEnding {
            outcome: Outcome::Cancelled,
            reason: Stop::Cancelled.reason(),
        },
        Some(Stop::Deadline) => RunEnding {
            outcome: if Outcome::Error.precedence() > derived.precedence() {
                Outcome::Error
            } else {
                derived
            },
            reason: Stop::Deadline.reason(),
        },
        Some(stop @ Stop::Policy(_)) => RunEnding {
            outcome: derived,
            reason: stop.reason(),
        },
        None => RunEnding {
            outcome: derived,
            reason: format!(
                "all {} units ran; the summary outcome is {derived}",
                outcomes.len()
            ),
        },
    }
}
