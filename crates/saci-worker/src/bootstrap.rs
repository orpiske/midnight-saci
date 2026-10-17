//! Starting a worker (`docs/worker.md`): validating the assigned
//! specification and its environment before any action can run, then
//! executing the plan and handing every event to a sink.
//!
//! A specification is rejected when its format version is unknown, it names
//! another run, its plan does not validate, a configured secret or the LLM
//! key is missing from the worker's environment, or the workspace is not a
//! writable directory. Nothing is executed for a rejected specification.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use saci_core::bundle::{ExpandedPlan, expand};
use saci_core::event::EventKind;
use saci_core::ids::RunId;
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::{RunSpec, SCHEMA_VERSION};
use saci_core::status::Outcome;
use tokio_util::sync::CancellationToken;

use crate::agent::{Executor, Toolbox};
use crate::credentials::Credentials;
use crate::events::{EventReceiver, EventSink, Recorder};
use crate::http::HttpTool;
use crate::llm::LlmConnection;
use crate::sequence::{self, RunEnding, RunPlan};
use crate::shell::{ShellConfig, ShellSession};

/// Backend base URL, set by the launcher.
pub const BACKEND_URL_ENV: &str = "SACI_BACKEND_URL";
/// The run this worker executes, set by the launcher.
pub const RUN_ID_ENV: &str = "SACI_RUN_ID";
/// The worker's workspace directory.
pub const WORKSPACE_ENV: &str = "SACI_WORKSPACE";

/// A specification that passed validation, with what executing it needs.
pub struct Prepared {
    /// The specification.
    pub spec: RunSpec,
    /// Its expanded plan.
    pub plan: ExpandedPlan,
    credentials: Credentials,
    llm: LlmConnection,
    workspace: PathBuf,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prepared")
            .field("run_id", &self.spec.run_id)
            .field("workspace", &self.workspace)
            .finish_non_exhaustive()
    }
}

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::new(ErrorKind::InvalidSpecification, message)
}

/// Validates `spec` for this worker.
///
/// `expected` is the run the launcher assigned, `process` the worker's
/// environment variables.
///
/// # Errors
///
/// An [`ErrorKind::InvalidSpecification`] error naming the problem.
pub fn prepare(
    spec: RunSpec,
    expected: Option<&RunId>,
    process: &BTreeMap<String, String>,
    workspace: &Path,
) -> Result<Box<Prepared>, ExecutionError> {
    if spec.schema_version != SCHEMA_VERSION {
        return Err(invalid(format!(
            "specification format {} is not supported (this worker reads format {SCHEMA_VERSION}); \
             use a worker image built from the same release as the backend",
            spec.schema_version
        )));
    }
    if let Some(expected) = expected.filter(|run| **run != spec.run_id) {
        return Err(invalid(format!(
            "the specification is for {}, but this worker was assigned {expected}",
            spec.run_id
        )));
    }
    let plan = expand(&spec.bundle).map_err(|error| invalid(format!("invalid plan:\n{error}")))?;
    let credentials = Credentials::project(&spec.environment, &spec.llm, process.clone())
        .map_err(|error| invalid(error.message))?;
    let llm = LlmConnection::new(&spec.llm, &spec.limits, |name| process.get(name).cloned())
        .map_err(|error| invalid(error.message))?;
    check_workspace(workspace)?;
    Ok(Box::new(Prepared {
        spec,
        plan,
        credentials,
        llm,
        workspace: workspace.to_owned(),
    }))
}

fn check_workspace(workspace: &Path) -> Result<(), ExecutionError> {
    let probe = tempfile::Builder::new()
        .prefix(".saci-probe")
        .tempfile_in(workspace);
    probe.map(drop).map_err(|error| {
        invalid(format!(
            "the workspace {} is not a writable directory: {error}",
            workspace.display()
        ))
    })
}

/// Reports a rejected specification as the run's end, without running
/// anything.
pub fn reject(run_id: RunId, error: &ExecutionError) -> EventReceiver {
    let (sink, events) = EventSink::new(run_id);
    let kind = EventKind::RunFinished {
        outcome: Outcome::Error,
        reason: format!(
            "the worker rejected the run specification: {}",
            error.message
        ),
    };
    sink.emit(None, None, kind);
    events
}

/// Builds the run's executor; its events arrive on the receiver, which
/// closes once [`execute`] returns.
///
/// # Errors
///
/// Fails when a tool cannot be set up.
pub fn executor(prepared: Box<Prepared>) -> Result<(Box<Running>, EventReceiver), ExecutionError> {
    let Prepared {
        spec,
        plan,
        credentials,
        llm,
        workspace,
    } = *prepared;
    let limits = spec.limits;
    let shell = ShellSession::new(ShellConfig {
        workspace,
        environment: credentials.shell_environment().clone(),
        output_limit: limits.output_limit_bytes,
        redactor: credentials.redactor().clone(),
    })?;
    let (sink, events) = EventSink::new(spec.run_id.clone());
    let toolbox = Toolbox {
        shell,
        http: HttpTool::new(credentials.clone(), limits.output_limit_bytes)?,
        recorder: Recorder::new(Arc::new(sink), credentials.redactor().clone()),
        environment: credentials.model_context(),
    };
    let executor = Executor::new(llm, toolbox, limits);
    Ok((
        Box::new(Running {
            spec,
            plan,
            executor,
        }),
        events,
    ))
}

/// A run ready to execute.
pub struct Running {
    spec: RunSpec,
    plan: ExpandedPlan,
    executor: Executor,
}

impl std::fmt::Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running")
            .field("run_id", &self.spec.run_id)
            .finish_non_exhaustive()
    }
}

/// Executes every unit until the run ends or `cancel` fires, then closes the
/// tools and the event stream.
pub async fn execute(running: Box<Running>, cancel: &CancellationToken) -> RunEnding {
    let Running {
        spec,
        plan,
        executor,
    } = *running;
    let run_plan = RunPlan {
        plan: &plan,
        policy: &spec.policy,
        step_timeout: Duration::from_secs(spec.limits.step_timeout_secs),
        deadline: Instant::now() + Duration::from_secs(spec.limits.run_timeout_secs),
        model: &spec.llm.model,
    };
    let ending = Box::pin(sequence::run(&executor, run_plan, cancel)).await;
    executor.close().await;
    ending
}
