//! Turning an investigation request into a stored investigation run
//! (`docs/investigations.md`): the failed unit and its evidence come from
//! the stored report of the investigated run, everything else from the
//! `investigations` configuration.

use std::collections::{BTreeMap, BTreeSet};

use saci_core::api::ErrorCode;
use saci_core::build_info::BuildInfo;
use saci_core::bundle::expand;
use saci_core::config::{Config, ConfigError, DEFAULT_POLICY};
use saci_core::evidence::Evidence;
use saci_core::ids::{EvidenceId, RunId};
use saci_core::investigation::{
    INVESTIGATED_OUTCOMES, InvestigationRequest, InvestigationSpec, InvestigationsConfig,
    Repository, is_git_value,
};
use saci_core::run::{ReportHeader, RunReport, UnitReport};
use saci_core::spec::{EnvironmentSpec, ExecutionLimits, RunSpec, SCHEMA_VERSION};
use saci_core::status::Lifecycle;
use time::OffsetDateTime;

use super::submit::sha256_hex;
use super::{Backend, Failure};
use crate::store::{NewRun, RunRecord, StoreError};

/// Observations copied from an attempt that ended without an assessment,
/// counted from its end.
const UNASSESSED_EVIDENCE: usize = 3;

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new(ErrorCode::ValidationFailed, message)
}

/// The `investigations` configuration.
///
/// # Errors
///
/// `not_found` when investigations are not configured.
pub fn configured(config: &Config) -> Result<&InvestigationsConfig, Failure> {
    config.investigations.as_deref().ok_or_else(|| {
        Failure::new(
            ErrorCode::NotFound,
            "investigations are not configured; add an `investigations` section to the \
             backend configuration",
        )
    })
}

/// Validates `request` and prepares the investigation run. Nothing is
/// stored or launched when this fails.
///
/// # Errors
///
/// `not_found` when investigations are not configured or the run does not
/// exist, and `validation_failed` when the run or unit cannot be
/// investigated or a source is unknown.
pub async fn prepare(
    backend: &Backend,
    request: &InvestigationRequest,
    idempotency_key: Option<String>,
) -> Result<Box<NewRun>, Failure> {
    let config = configured(&backend.config)?;
    let parent = Box::pin(backend.store.run(&request.run_id)).await?;
    let unit = failed_unit(&parent, request)?;
    let repositories = repositories(config, request)?;
    let (environment, limits) = backend
        .config
        .environment(&config.environment)
        .map_err(|error| config_failure(&error))?;
    let limits = config.limits.apply(limits);
    let evidence = Box::pin(evidence(backend, &parent, &unit, limits.output_limit_bytes)).await?;
    let investigation = Box::new(InvestigationSpec {
        parent_run: request.run_id.clone(),
        plan_title: parent.report.header.plan_title.clone(),
        action: action(&parent.spec, &unit),
        unit,
        evidence,
        repositories,
        prepare: config.prepare.clone(),
        instructions: config.instructions.clone(),
    });
    let hash = sha256_hex(&serde_json::to_vec(request).unwrap_or_default());
    let mut run = new_run(&backend.config, &parent, investigation, environment, limits)?;
    run.idempotency = idempotency_key.map(|key| (key, hash));
    Ok(run)
}

fn config_failure(error: &ConfigError) -> Failure {
    invalid(error.to_string())
}

/// The investigation run that executes `investigation`.
fn new_run(
    config: &Config,
    parent: &RunRecord,
    investigation: Box<InvestigationSpec>,
    environment: EnvironmentSpec,
    limits: ExecutionLimits,
) -> Result<Box<NewRun>, Failure> {
    let identity = investigation.unit_identity();
    let spec = RunSpec {
        schema_version: SCHEMA_VERSION,
        run_id: RunId::generate(),
        created_at: OffsetDateTime::now_utc(),
        bundle: None,
        investigation: Some(investigation),
        environment,
        policy: config
            .policy(DEFAULT_POLICY)
            .map_err(|error| config_failure(&error))?,
        limits,
        llm: config.llm.settings(),
        judge: None,
    };
    let mut report = RunReport::new(header(&spec, parent), vec![identity]);
    report.backend_build = Some(BuildInfo::current());
    Ok(Box::new(NewRun {
        report,
        spec,
        idempotency: None,
        actions: BTreeMap::new(),
    }))
}

fn header(spec: &RunSpec, parent: &RunRecord) -> ReportHeader {
    ReportHeader {
        run_id: spec.run_id.clone(),
        plan_title: parent.report.header.plan_title.clone(),
        entry: parent.report.header.entry.clone(),
        environment: spec.environment.name.clone(),
        policy: spec.policy.name.clone(),
        model: spec.llm.model.clone(),
        created_at: spec.created_at,
    }
}

/// Refuses a run that is an investigation or has not finished.
fn check_parent(parent: &RunRecord, run: &RunId) -> Result<(), Failure> {
    if parent.spec.investigation.is_some() {
        return Err(invalid(format!(
            "{run} is an investigation; an investigation cannot be investigated"
        )));
    }
    if parent.report.lifecycle != Lifecycle::Finished {
        return Err(invalid(format!(
            "{run} has not finished; investigate it after it ends"
        )));
    }
    Ok(())
}

/// The unit to investigate: the requested one, or the first unit that
/// failed, ended in error or was blocked.
fn failed_unit(parent: &RunRecord, request: &InvestigationRequest) -> Result<UnitReport, Failure> {
    let run = &request.run_id;
    check_parent(parent, run)?;
    let investigable = |unit: &&UnitReport| {
        unit.outcome
            .is_some_and(|o| INVESTIGATED_OUTCOMES.contains(&o))
    };
    let units = &parent.report.units;
    let unit = match &request.unit {
        None => units.iter().find(investigable).ok_or_else(|| {
            invalid(format!(
                "{run} has no unit that failed, ended in error or was blocked"
            ))
        })?,
        Some(key) => {
            let unit = units
                .iter()
                .find(|unit| unit.identity.key == *key)
                .ok_or_else(|| invalid(format!("{run} has no unit {key}")))?;
            if !investigable(&unit) {
                return Err(invalid(format!(
                    "unit {key} of {run} did not fail, end in error or get blocked"
                )));
            }
            unit
        }
    };
    Ok(unit.clone())
}

/// The selected repositories at their refs: every configured repository
/// at its default ref when the request selects none.
fn repositories(
    config: &InvestigationsConfig,
    request: &InvestigationRequest,
) -> Result<Vec<Repository>, Failure> {
    if request.sources.is_empty() {
        return Ok(config.repositories.clone());
    }
    let mut selected = Vec::new();
    let mut names = BTreeSet::new();
    for source in &request.sources {
        let name = &source.name;
        let mut repository = config
            .repositories
            .iter()
            .find(|repository| repository.name == *name)
            .cloned()
            .ok_or_else(|| invalid(format!("sources: `{name}` is not a configured repository")))?;
        if !names.insert(name.as_str()) {
            return Err(invalid(format!("sources: `{name}` is selected twice")));
        }
        if let Some(git_ref) = &source.git_ref {
            if !is_git_value(git_ref) {
                return Err(invalid(format!(
                    "sources: the ref of `{name}` must be a branch, tag or commit"
                )));
            }
            repository.git_ref.clone_from(git_ref);
        }
        selected.push(repository);
    }
    Ok(selected)
}

/// The action text of the unit in the investigated plan.
fn action(spec: &RunSpec, unit: &UnitReport) -> String {
    let plan = spec.bundle.as_ref().and_then(|bundle| expand(bundle).ok());
    plan.and_then(|plan| {
        plan.units
            .into_iter()
            .find(|candidate| candidate.identity.key == unit.identity.key)
            .map(|candidate| candidate.unit.action)
    })
    .unwrap_or_default()
}

/// The evidence IDs the unit's attempts cite, in order and once each: the
/// observations each assessment cites, and the last observations of an
/// attempt without an assessment.
fn cited(unit: &UnitReport) -> Vec<&EvidenceId> {
    let mut ids: Vec<&EvidenceId> = Vec::new();
    for attempt in &unit.attempts {
        let cited: Vec<&EvidenceId> = match &attempt.assessment {
            Some(assessment) => assessment
                .criteria
                .iter()
                .flat_map(|criterion| &criterion.evidence)
                .collect(),
            None => {
                let skip = attempt.evidence.len().saturating_sub(UNASSESSED_EVIDENCE);
                attempt.evidence.iter().skip(skip).collect()
            }
        };
        for id in cited {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// The evidence the unit's attempts cite, with each text cut to `limit`
/// bytes.
async fn evidence(
    backend: &Backend,
    parent: &RunRecord,
    unit: &UnitReport,
    limit: u64,
) -> Result<Vec<Evidence>, Failure> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut evidence = Vec::new();
    for id in cited(unit) {
        let mut item = match backend.store.evidence(&parent.spec.run_id, id).await {
            Ok(item) => item,
            // A report cites only stored evidence; skip what is not there.
            Err(StoreError::NotFound) => continue,
            Err(error) => return Err(error.into()),
        };
        item.record.truncate_to(limit);
        evidence.push(item);
    }
    Ok(evidence)
}
