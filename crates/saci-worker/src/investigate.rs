//! Runs an investigation: its one unit and attempt, then `run_finished`
//! (`docs/plans/investigations.md`).

use saci_core::event::EventKind;
use saci_core::investigation::InvestigationReport;

use crate::agent::{AttemptResult, Executor, InvestigationRequest};
use crate::environment;
use crate::sequence::RunEnding;

/// Reports `run_started`, the investigation's attempt, its unit's end and
/// `run_finished`.
pub async fn run(executor: &Executor, request: InvestigationRequest<'_>, model: &str) -> RunEnding {
    let events = executor.recorder().events();
    let model = model.to_owned();
    let environment = Some(Box::new(environment::collect().await));
    events.emit(None, None, EventKind::RunStarted { model, environment });
    let (result, conclusion) = Box::pin(executor.investigate(request)).await;
    let identity = request.spec.unit_identity();
    let outcome = result.outcome;
    events.emit(Some(&identity), None, EventKind::UnitFinished { outcome });
    let ending = RunEnding {
        outcome,
        reason: reason(&result, conclusion.as_ref()),
    };
    let kind = EventKind::RunFinished {
        outcome,
        reason: ending.reason.clone(),
    };
    events.emit(None, None, kind);
    ending
}

fn reason(result: &AttemptResult, conclusion: Option<&InvestigationReport>) -> String {
    if let Some(report) = conclusion {
        let summary = report.summary.lines().next().unwrap_or_default().trim();
        return format!(
            "the investigation ended {}: {summary}",
            report.outcome.as_str()
        );
    }
    match &result.error {
        Some(error) => format!("the investigation could not conclude: {}", error.message),
        None => format!("the investigation ended {}", result.outcome),
    }
}
