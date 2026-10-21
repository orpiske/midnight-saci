//! The optional System One judge: a TypeSafe Noul evaluation that decides
//! whether the evidence of a step whose blocks all ran satisfies each
//! expected result, without a model request.
//!
//! The judge only passes steps. When a result scores below the threshold,
//! or the judge fails, the model assesses the step as usual.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use rig::http_client::ReqwestClient;
use rig::typesafeai::types::{Answer, Question};
use rig::typesafeai::{DynamicQuery, Evaluate, Jev, JevConfig};
use saci_core::assessment::{self, Assessment, Basis, CriterionResult, CriterionStatus};
use saci_core::bundle::ExecutionUnit;
use saci_core::plan::Criterion;
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::{ExecutionLimits, JudgeSettings};
use saci_core::status::Outcome;
use serde_json::{Value, json};

use super::prompt::Ran;
use super::state::AttemptState;

/// What the judge is told a high probability means.
const SATISFIED: &str =
    "The recorded evidence of the commands that ran shows that the expected result holds.";

/// What the judge is told a low probability means.
const NOT_SATISFIED: &str =
    "The recorded evidence does not show the expected result, contradicts it, or is missing.";

/// A connection to the configured judge.
pub struct Judge {
    jev: Jev,
    model: String,
    threshold_percent: u8,
}

impl std::fmt::Debug for Judge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Judge")
            .field("model", &self.model)
            .field("threshold_percent", &self.threshold_percent)
            .finish_non_exhaustive()
    }
}

impl Judge {
    /// Connects to the judge with the credential `key`.
    ///
    /// # Errors
    ///
    /// Fails when the HTTP client cannot be built.
    pub fn new(
        settings: &JudgeSettings,
        key: &str,
        limits: &ExecutionLimits,
    ) -> Result<Self, ExecutionError> {
        let http = rig::rig_reqwest::reqwest::Client::builder()
            .timeout(Duration::from_secs(limits.model_request_timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| {
                ExecutionError::new(
                    ErrorKind::InvalidSpecification,
                    crate::llm::client_build_error(&error),
                )
            })?;
        let jev = JevConfig::new(key)
            .model(settings.model.clone())
            .with_endpoint(settings.endpoint.clone())
            .connect(ReqwestClient::from(http));
        Ok(Self {
            jev,
            model: settings.model.clone(),
            threshold_percent: settings.threshold_percent,
        })
    }

    /// The passed assessment of a step whose blocks all ran, when the judge
    /// finds every expected result satisfied. `None` when the step is not
    /// eligible, the attempt stops, or the model must assess the step.
    pub async fn assess(
        &self,
        state: &AttemptState,
        unit: &ExecutionUnit,
        environment: &str,
        ran: &Ran,
    ) -> Option<Assessment> {
        if !eligible(unit, ran) || state.check_interrupted().is_err() {
            return None;
        }
        let started = Instant::now();
        let deadline = tokio::time::sleep_until(state.deadline.into());
        let scores = tokio::select! {
            scores = Box::pin(self.scores(unit, environment, ran)) => scores,
            () = state.cancel.cancelled() => return None,
            () = deadline => return None,
        };
        let assessment = scores
            .and_then(|scores| self.assessment(&unit.unit.expected, ran, &scores, started))
            .and_then(|assessment| {
                let recorded = state.recorder.recorded();
                assessment::validate(&assessment, &state.criteria, &recorded)
                    .map(|()| assessment)
                    .map_err(|problems| problems.join("; "))
            });
        assessment
            .inspect_err(|reason| {
                let unit = unit.identity.key.as_str();
                tracing::info!(unit, "the model assesses the step, since {reason}");
            })
            .ok()
    }

    /// The probability that the evidence satisfies each expected result of
    /// `unit`, in criterion order.
    async fn scores(
        &self,
        unit: &ExecutionUnit,
        environment: &str,
        ran: &Ran,
    ) -> Result<Vec<f64>, String> {
        let expected = &unit.unit.expected;
        let query = questions(expected)?;
        let state = json!({
            "step": unit.unit.title,
            "action": unit.unit.action,
            "environment": environment,
            "procedures": ran.procedures,
            "evidence": ran.evidence,
        });
        let result = self
            .jev
            .evaluation()
            .evaluate(&state, query)
            .await
            .map_err(|error| format!("the judge failed: {error}"))?;
        expected
            .iter()
            .map(
                |criterion| match result.answers.get(&question_id(criterion.index)) {
                    Some(Answer::Noul { noul }) => Ok(*noul),
                    _ => Err(format!(
                        "the judge gave no probability for criterion {}",
                        criterion.index
                    )),
                },
            )
            .collect()
    }

    /// The passed assessment, or why the scores do not pass the step.
    fn assessment(
        &self,
        expected: &[Criterion],
        ran: &Ran,
        scores: &[f64],
        started: Instant,
    ) -> Result<Assessment, String> {
        let threshold = f64::from(self.threshold_percent) / 100.0;
        if scores.iter().any(|score| *score < threshold) {
            return Err(format!(
                "the judge's probabilities {scores:.2?} are not all at or above {threshold:.2}"
            ));
        }
        let model = &self.model;
        let criteria = expected
            .iter()
            .zip(scores)
            .map(|(criterion, score)| CriterionResult {
                criterion: criterion.index,
                status: CriterionStatus::Satisfied,
                basis: Basis::ModelJudgment,
                evidence: ran.ids.clone(),
                explanation: format!(
                    "The judge {model} gives a probability of {score:.2} that the evidence shows this result."
                ),
            })
            .collect();
        Ok(Assessment {
            outcome: Outcome::Passed,
            criteria,
            explanation: format!(
                "The System One judge {model} passed every expected result from the evidence of \
                 the blocks Saci ran (threshold {threshold:.2}, {} ms). The model was not asked.",
                started.elapsed().as_millis()
            ),
        })
    }
}

/// One Noul question per expected result.
fn questions(expected: &[Criterion]) -> Result<DynamicQuery, String> {
    let questions = expected
        .iter()
        .map(|criterion| {
            let question = Question::Noul {
                instructions: Value::String(format!(
                    "Does the recorded evidence show that this expected result holds: {}",
                    criterion.text
                )),
                criteria: Some(BTreeMap::from([
                    ("true".to_owned(), Value::String(SATISFIED.to_owned())),
                    ("false".to_owned(), Value::String(NOT_SATISFIED.to_owned())),
                ])),
            };
            (question_id(criterion.index), question)
        })
        .collect();
    DynamicQuery::new(questions)
        .map_err(|error| format!("the judge questions are invalid: {error}"))
}

fn question_id(criterion: usize) -> String {
    format!("criterion_{criterion}")
}

/// Whether the evidence covers the whole action: the unit has expected
/// results, and every code block of it is marked `run`, ran and exited
/// normally.
fn eligible(unit: &ExecutionUnit, ran: &Ran) -> bool {
    let commands = &unit.unit.commands;
    let all_run = !commands.is_empty() && commands.iter().all(|block| block.run);
    all_run && ran.complete && !unit.unit.expected.is_empty()
}
