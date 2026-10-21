//! The shell, HTTP and step-assessment tools registered with the agent.
//!
//! Every call is validated before it runs and recorded through the run's
//! [`Recorder`](crate::events::Recorder); invalid calls are recorded as
//! rejected and answered with feedback the model can act on.

use std::sync::Arc;

use rig::message::ToolName;
use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};
use saci_core::assessment::{self, Assessment};
use saci_core::evidence::EvidenceRecord;
use saci_core::run::ExecutionError;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::render;
use super::state::AttemptState;
use crate::http::{HttpRequest, HttpTool};
use crate::shell::ShellSession;

/// Name of the shell tool.
pub const SHELL: &str = "shell";
/// Name of the HTTP tool.
pub const HTTP: &str = "http";
/// Name of the step-assessment tool.
pub const ASSESS: &str = "assess";

type ToolResult = Result<ToolOutput, ToolExecutionError>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellArgs {
    command: String,
}

fn tool_name(name: &'static str) -> Result<ToolName, ExecutionError> {
    ToolName::new(name).map_err(|_| {
        ExecutionError::new(
            saci_core::run::ErrorKind::Tool,
            format!("invalid tool name `{name}`"),
        )
    })
}

/// Parses arguments; on failure records the rejected call and returns
/// feedback for the model.
fn parse<T: DeserializeOwned>(
    state: &AttemptState,
    name: &str,
    args: Value,
) -> Result<T, ToolExecutionError> {
    let raw = args.to_string();
    serde_json::from_value(args).map_err(|error| {
        let reason = format!("invalid arguments for `{name}`: {error}");
        state.reject(name, &raw, &reason);
        ToolExecutionError::invalid_args(format!(
            "{reason}. Call the tool again with valid arguments."
        ))
    })
}

/// Ends the attempt because the tool infrastructure failed.
fn infrastructure(state: &AttemptState, error: ExecutionError) -> ToolExecutionError {
    let reason = state.halt(error);
    ToolExecutionError::other(reason.clone()).with_model_feedback(reason)
}

/// The shell tool over the run's persistent session.
pub fn shell(
    state: Arc<AttemptState>,
    session: Arc<Mutex<ShellSession>>,
) -> Result<DynamicTool, ExecutionError> {
    let schema = json!({
        "type": "object",
        "properties": {"command": {"type": "string", "description": "Shell commands to run as written"}},
        "required": ["command"],
        "additionalProperties": false
    });
    let description = "Run commands in the run's persistent bash session. Variables, functions, the working \
        directory and files persist between calls and steps. Returns the exit code, stdout and stderr as evidence.";
    Ok(DynamicTool::new(
        tool_name(SHELL)?,
        description,
        schema,
        move |args| {
            let (state, session) = (state.clone(), session.clone());
            Box::pin(async move { run_shell(&state, &session, args).await })
        },
    ))
}

async fn run_shell(state: &AttemptState, session: &Mutex<ShellSession>, args: Value) -> ToolResult {
    let args: ShellArgs = parse(state, SHELL, args)?;
    let _running = state.running();
    let mut session = session.lock().await;
    let deadline = state.operation_deadline();
    match session.run(&args.command, deadline, &state.cancel).await {
        Ok(run) => {
            let evidence = state.record(run.duration, EvidenceRecord::Shell(run.evidence));
            Ok(ToolOutput::text(render::evidence(&evidence)))
        }
        Err(error) => Err(infrastructure(state, error)),
    }
}

/// The HTTP tool.
pub fn http(state: Arc<AttemptState>, tool: Arc<HttpTool>) -> Result<DynamicTool, ExecutionError> {
    let description = "Send one HTTP request to the target environment. Redirects are not followed. Any status \
        code is a response; a request that got no response reports a transport error. Returns evidence.";
    Ok(DynamicTool::new(
        tool_name(HTTP)?,
        description,
        HttpRequest::schema(),
        move |args| {
            let (state, tool) = (state.clone(), tool.clone());
            Box::pin(async move { run_http(&state, &tool, args).await })
        },
    ))
}

async fn run_http(state: &AttemptState, tool: &HttpTool, args: Value) -> ToolResult {
    let raw = args.to_string();
    let request: HttpRequest = parse(state, HTTP, args)?;
    let _running = state.running();
    match tool
        .send(&request, state.operation_deadline(), &state.cancel)
        .await
    {
        Ok(run) => {
            let evidence = state.record(run.duration, EvidenceRecord::Http(run.evidence));
            Ok(ToolOutput::text(render::evidence(&evidence)))
        }
        Err(reason) => {
            state.reject(HTTP, &raw, &reason);
            Err(ToolExecutionError::invalid_args(format!(
                "{reason}. The request was not sent."
            )))
        }
    }
}

/// The step-assessment tool: it finishes the attempt and acts on nothing.
pub fn assess(state: Arc<AttemptState>) -> Result<DynamicTool, ExecutionError> {
    let description = "Submit the final assessment of this step, once. It records the verdict and does \
        nothing to the target. Every expected result needs one entry citing evidence IDs from tool results.";
    let schema = assessment::schema();
    Ok(DynamicTool::new(
        tool_name(ASSESS)?,
        description,
        schema,
        move |args| {
            let state = state.clone();
            Box::pin(async move { submit(&state, args) })
        },
    ))
}

fn submit(state: &AttemptState, args: Value) -> ToolResult {
    let assessment: Assessment = serde_json::from_value(args).map_err(|error| {
        let problem = format!("the assessment does not match the tool schema: {error}");
        state.refuse(vec![problem.clone()]);
        ToolExecutionError::invalid_args(format!("{problem}. Call assess again."))
    })?;
    let recorded = state.recorder.recorded();
    match assessment::validate(&assessment, &state.criteria, &recorded) {
        Ok(()) => {
            state.accept(assessment);
            Ok(ToolOutput::text(
                "Assessment recorded. The step is complete.",
            ))
        }
        Err(problems) => {
            let list = problems
                .iter()
                .map(|p| format!("- {p}"))
                .collect::<Vec<_>>()
                .join("\n");
            state.refuse(problems);
            Err(ToolExecutionError::invalid_args(format!(
                "The assessment was not accepted:\n{list}\nCorrect it and call assess again."
            )))
        }
    }
}
