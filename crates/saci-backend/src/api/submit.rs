//! Turning a submission into a stored run: validation, the immutable run
//! specification and the initial report.

use saci_core::api::{ErrorCode, SubmitRun};
use saci_core::bundle::expand;
use saci_core::config::{Config, DEFAULT_POLICY};
use saci_core::ids::RunId;
use saci_core::run::{ReportHeader, RunReport};
use saci_core::spec::{RunSpec, SCHEMA_VERSION};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use super::Failure;
use crate::store::NewRun;

/// Hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Validates `request` against the configuration and prepares the run.
/// Nothing is stored or launched when this fails.
///
/// # Errors
///
/// Returns `validation_failed` with every plan diagnostic, or naming the
/// unknown environment or policy.
pub fn prepare(
    request: &SubmitRun,
    config: &Config,
    idempotency_key: Option<String>,
) -> Result<NewRun, Failure> {
    let expanded = expand(&request.bundle).map_err(plan_invalid)?;
    let invalid = |error: saci_core::config::ConfigError| {
        Failure::new(ErrorCode::ValidationFailed, error.to_string())
    };
    let (environment, limits) = config.environment(&request.environment).map_err(invalid)?;
    let policy_name = request.policy.as_deref().unwrap_or(DEFAULT_POLICY);
    let policy = config.policy(policy_name).map_err(invalid)?;
    let actions = config.policy_actions(policy_name).map_err(invalid)?;
    let spec = RunSpec {
        schema_version: SCHEMA_VERSION,
        run_id: RunId::generate(),
        created_at: OffsetDateTime::now_utc(),
        bundle: request.bundle.clone(),
        environment,
        policy,
        limits,
        llm: config.llm.clone(),
    };
    let header = header(&spec, &expanded.plan.title);
    let units = expanded.units.into_iter().map(|u| u.identity).collect();
    let idempotency = idempotency_key.map(|key| (key, request_hash(request)));
    Ok(NewRun {
        report: RunReport::new(header, units),
        spec,
        idempotency,
        actions,
    })
}

fn plan_invalid(error: saci_core::parser::PlanError) -> Failure {
    let details = error
        .diagnostics
        .into_iter()
        .map(|d| d.to_string())
        .collect();
    Failure::new(ErrorCode::ValidationFailed, "the plan is invalid").with_details(details)
}

fn header(spec: &RunSpec, plan_title: &str) -> ReportHeader {
    ReportHeader {
        run_id: spec.run_id.clone(),
        plan_title: plan_title.to_owned(),
        entry: spec.bundle.entry.clone(),
        environment: spec.environment.name.clone(),
        policy: spec.policy.name.clone(),
        model: spec.llm.model.clone(),
        created_at: spec.created_at,
    }
}

/// Identifies a submission's content, so a reused idempotency key with a
/// different request is detected.
fn request_hash(request: &SubmitRun) -> String {
    let canonical = serde_json::to_vec(request).unwrap_or_default();
    sha256_hex(&canonical)
}
