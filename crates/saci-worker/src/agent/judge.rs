//! The optional System One judge: a Jev Noul evaluation, from TypeSafe or
//! through OpenRouter, that decides whether the evidence of a step whose
//! blocks all ran satisfies each expected result, without a model request.
//!
//! The judge only passes steps. When a result scores below the threshold,
//! or the judge fails, the model assesses the step as usual.
//!
//! Both services take `{model, state, questions}` and answer with
//! `answers.<question>.noul`. Saci reads only the answers, so the rest of
//! each service's reply (its `usage` and cost fields differ) does not matter.

use std::time::{Duration, Instant};

use rig::rig_reqwest::reqwest::Client;
use rig::rig_reqwest::reqwest::header::CONTENT_TYPE;
use saci_core::assessment::{self, Assessment, Basis, CriterionResult, CriterionStatus};
use saci_core::bundle::ExecutionUnit;
use saci_core::plan::Criterion;
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::{ExecutionLimits, JudgeSettings};
use saci_core::status::Outcome;
use serde_json::{Map, Value, json};

use super::prompt::Ran;
use super::state::AttemptState;

/// What the judge is told a high probability means.
const SATISFIED: &str =
    "The recorded evidence of the commands that ran shows that the expected result holds.";

/// What the judge is told a low probability means.
const NOT_SATISFIED: &str =
    "The recorded evidence does not show the expected result, contradicts it, or is missing.";

/// Longest part of a failed judge reply quoted in the log.
const BODY_EXCERPT: usize = 300;

/// A connection to the configured judge.
pub struct Judge {
    http: Client,
    endpoint: String,
    key: String,
    model: String,
    threshold_percent: u8,
}

impl std::fmt::Debug for Judge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Judge")
            .field("endpoint", &self.endpoint)
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
        let http = Client::builder()
            .timeout(Duration::from_secs(limits.model_request_timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| {
                ExecutionError::new(
                    ErrorKind::InvalidSpecification,
                    crate::llm::client_build_error(&error),
                )
            })?;
        Ok(Self {
            http,
            endpoint: settings.endpoint.clone(),
            key: key.to_owned(),
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
        let body = json!({
            "model": self.model,
            "state": {
                "step": unit.unit.title,
                "action": unit.unit.action,
                "environment": environment,
                "procedures": ran.procedures,
                "evidence": ran.evidence,
            },
            "questions": questions(expected),
        });
        let reply = self.ask(&body).await?;
        expected
            .iter()
            .map(|criterion| {
                reply["answers"][question_id(criterion.index)]["noul"]
                    .as_f64()
                    .filter(|noul| (0.0..=1.0).contains(noul))
                    .ok_or_else(|| {
                        format!(
                            "the judge gave no probability for criterion {}",
                            criterion.index
                        )
                    })
            })
            .collect()
    }

    /// Sends one evaluation request and returns the JSON reply.
    async fn ask(&self, body: &Value) -> Result<Value, String> {
        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.key)
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| format!("the judge request failed: {error}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("the judge reply could not be read: {error}"))?;
        if !status.is_success() {
            let excerpt: String = text.chars().take(BODY_EXCERPT).collect();
            return Err(format!("the judge answered HTTP {status}: {excerpt}"));
        }
        serde_json::from_str(&text).map_err(|error| format!("the judge reply is not JSON: {error}"))
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
fn questions(expected: &[Criterion]) -> Map<String, Value> {
    expected
        .iter()
        .map(|criterion| {
            let question = json!({
                "type": "noul",
                "instructions": format!(
                    "Does the recorded evidence show that this expected result holds: {}",
                    criterion.text
                ),
                "criteria": {"true": SATISFIED, "false": NOT_SATISFIED},
            });
            (question_id(criterion.index), question)
        })
        .collect()
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
