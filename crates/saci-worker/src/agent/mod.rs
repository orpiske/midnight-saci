//! One attempt of one execution unit, run by Rig's agent loop with Saci's
//! tools and controls.
//!
//! Rig owns the conversation, the model requests and tool dispatch. Saci
//! supplies the tools ([`tools`]), counts and stops every model and tool
//! operation in a hook ([`hook`]), records every observation through the
//! run's [`Recorder`], and decides the attempt's outcome from the accepted
//! assessment or the reason the attempt stopped.

mod hook;
pub mod mcp;
pub mod prompt;
mod render;
mod state;
mod tools;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rig::AgentBuilder;
use rig::completion::PromptError;
use rig::tool::DynamicTool;
use saci_core::assessment::Assessment;
use saci_core::bundle::ExecutionUnit;
use saci_core::event::EventKind;
use saci_core::evidence::{EvidenceRecord, Termination};
use saci_core::run::{AttemptTiming, ErrorKind, ExecutionError};
use saci_core::spec::ExecutionLimits;
use saci_core::status::Outcome;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub use render::evidence as render_evidence;
use state::{AttemptState, Setup};

use crate::events::Recorder;
use crate::http::HttpTool;
use crate::llm::LlmConnection;
use crate::mcp::McpServer;
use crate::shell::ShellSession;

/// How long an interrupted tool may take to stop before its future is
/// dropped. Covers the shell's kill grace period.
const TOOL_GRACE: Duration = Duration::from_secs(5);

/// Executes attempts for one run. Holds the run-scoped tools, so shell
/// state carries over from one attempt and unit to the next.
pub struct Executor {
    llm: LlmConnection,
    shell: Arc<Mutex<ShellSession>>,
    http: Arc<HttpTool>,
    mcp: Vec<Arc<mcp::McpBinding>>,
    mcp_servers: Mutex<Vec<McpServer>>,
    recorder: Recorder,
    limits: ExecutionLimits,
    /// The system prompt, the same for every attempt of the run.
    system: String,
}

impl std::fmt::Debug for Executor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Executor")
            .field("llm", &self.llm)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// The run-scoped parts an [`Executor`] is built from.
#[derive(Debug)]
pub struct Toolbox {
    /// The run's shell session.
    pub shell: ShellSession,
    /// The HTTP tool.
    pub http: HttpTool,
    /// Records observations and reports events.
    pub recorder: Recorder,
    /// What the model is told about the environment.
    pub environment: String,
}

/// One attempt to execute.
#[derive(Debug, Clone, Copy)]
pub struct AttemptRequest<'a> {
    /// The unit.
    pub unit: &'a ExecutionUnit,
    /// One-based attempt number.
    pub attempt: u32,
    /// Plan context and earlier results, see [`prompt::plan_context`].
    pub context: &'a str,
    /// The results of the procedure units the unit invoked, which ran
    /// before it.
    pub procedures: &'a [String],
    /// When the attempt must end: the step deadline, capped by the run's.
    pub deadline: Instant,
    /// Cancels the run.
    pub cancel: &'a CancellationToken,
}

/// The finished attempt, as reported in its `attempt_finished` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptResult {
    /// The attempt outcome.
    pub outcome: Outcome,
    /// The validated assessment, when one was accepted.
    pub assessment: Option<Assessment>,
    /// What prevented an assessment, when something did.
    pub error: Option<ExecutionError>,
    /// Where the attempt spent its time, once its agent ran.
    pub timing: Option<AttemptTiming>,
}

impl AttemptResult {
    const fn assessed(assessment: Assessment) -> Self {
        Self {
            outcome: assessment.outcome,
            assessment: Some(assessment),
            error: None,
            timing: None,
        }
    }

    const fn failed(error: ExecutionError) -> Self {
        Self {
            outcome: Outcome::Error,
            assessment: None,
            error: Some(error),
            timing: None,
        }
    }

    const fn cancelled() -> Self {
        Self {
            outcome: Outcome::Cancelled,
            assessment: None,
            error: None,
            timing: None,
        }
    }
}

/// A running agent loop.
type AgentRun = Pin<Box<dyn Future<Output = Result<(), Box<PromptError>>> + Send>>;

/// How the agent run ended.
enum Ending {
    Finished(Result<(), Box<PromptError>>),
    Interrupted,
}

impl Executor {
    /// Creates the executor of a run.
    #[must_use]
    pub fn new(llm: LlmConnection, toolbox: Toolbox, limits: ExecutionLimits) -> Self {
        Self {
            llm,
            shell: Arc::new(Mutex::new(toolbox.shell)),
            http: Arc::new(toolbox.http),
            mcp: Vec::new(),
            mcp_servers: Mutex::new(Vec::new()),
            recorder: toolbox.recorder,
            limits,
            system: prompt::system(&toolbox.environment),
        }
    }

    /// The run's recorder.
    #[must_use]
    pub const fn recorder(&self) -> &Recorder {
        &self.recorder
    }

    /// Exposes the selected tools of the run's connected MCP servers to
    /// every attempt; [`Executor::close`] shuts the servers down.
    pub fn attach_mcp(&mut self, servers: Vec<McpServer>) {
        self.mcp = mcp::bindings(&servers).into_iter().map(Arc::new).collect();
        self.mcp_servers = Mutex::new(servers);
    }

    /// The names the model calls the MCP tools by.
    #[must_use]
    pub fn mcp_tools(&self) -> Vec<String> {
        self.mcp
            .iter()
            .map(|binding| binding.exposed().to_owned())
            .collect()
    }

    /// Runs one attempt and reports its start, observations and result.
    pub async fn attempt(&self, request: AttemptRequest<'_>) -> AttemptResult {
        let identity = &request.unit.identity;
        let events = self.recorder.events();
        events.emit(
            Some(identity),
            Some(request.attempt),
            EventKind::AttemptStarted,
        );
        let result = Box::pin(self.execute(request)).await;
        let kind = EventKind::AttemptFinished {
            outcome: result.outcome,
            assessment: result.assessment.clone(),
            error: result.error.clone(),
            timing: result.timing,
        };
        events.emit(Some(identity), Some(request.attempt), kind);
        result
    }

    async fn execute(&self, request: AttemptRequest<'_>) -> AttemptResult {
        let state = Arc::new(AttemptState::new(Setup {
            unit: request.unit.identity.clone(),
            attempt: request.attempt,
            criteria: request.unit.unit.expected.clone(),
            limits: self.limits,
            deadline: request.deadline,
            cancel: request.cancel.child_token(),
            recorder: self.recorder.clone(),
        }));
        let tools = match self.tools(&state) {
            Ok(tools) => tools,
            Err(error) => return AttemptResult::failed(error),
        };
        let mut ran = Box::pin(self.run_marked(&state, request.unit)).await;
        ran.procedures = request.procedures.to_vec();
        let text = prompt::unit(request.unit, request.attempt, request.context, &ran);
        let run = self.start(&state, tools, text);
        let ending = Box::pin(self.run_agent(&state, run, request.cancel)).await;
        let mut result = self.conclude(&state, ending, request.cancel);
        result.timing = Some(state.timing());
        result
    }

    /// Runs the unit's `run` blocks as written, in order, before the model is
    /// asked. Stops at a block that did not exit normally, or when the
    /// attempt is interrupted or out of tool calls; the model then sees what
    /// ran and what did not.
    async fn run_marked(&self, state: &AttemptState, unit: &ExecutionUnit) -> prompt::Ran {
        let marked: Vec<_> = unit
            .unit
            .commands
            .iter()
            .filter(|block| block.run)
            .collect();
        let mut ran = prompt::Ran::default();
        for block in &marked {
            if state.check_interrupted().is_err() || state.count_tool_call().is_err() {
                break;
            }
            let _running = state.running();
            let mut session = self.shell.lock().await;
            let deadline = state.operation_deadline();
            let result = session.run(&block.code, deadline, &state.cancel).await;
            let Ok(run) = result.map_err(|error| state.halt(error)) else {
                break;
            };
            let exited = run.evidence.termination == Termination::Exited;
            let evidence = state.record(run.duration, EvidenceRecord::Shell(run.evidence));
            ran.evidence.push(render::evidence(&evidence));
            if !exited {
                break;
            }
        }
        ran.not_run = marked.len() - ran.evidence.len();
        ran
    }

    fn tools(&self, state: &Arc<AttemptState>) -> Result<Vec<DynamicTool>, ExecutionError> {
        let mut tools = vec![
            tools::shell(state.clone(), self.shell.clone())?,
            tools::http(state.clone(), self.http.clone())?,
            tools::assess(state.clone())?,
        ];
        for binding in &self.mcp {
            let redactor = self.recorder.redactor().clone();
            tools.push(mcp::tool(state.clone(), binding.clone(), redactor)?);
        }
        Ok(tools)
    }

    /// Builds the attempt's agent and starts its run.
    fn start(&self, state: &Arc<AttemptState>, tools: Vec<DynamicTool>, text: String) -> AgentRun {
        let agent = AgentBuilder::new(self.llm.model())
            .preamble(&self.system)
            .dynamic_tools(tools)
            .build();
        // The hook stops the run at the model-call limit; Rig's own turn
        // limit is a backstop one request later.
        let max_turns = self.llm.max_turns().saturating_add(1);
        let controls = hook::Controls::new(state.clone());
        Box::pin(async move {
            agent
                .prompt(text)
                .max_turns(max_turns)
                .tool_concurrency(1)
                .add_hook(controls)
                .await
                .map(|_| ())
                .map_err(Box::new)
        })
    }

    /// Runs the agent until it ends, the run is cancelled or the attempt
    /// deadline passes. An interrupted tool gets [`TOOL_GRACE`] to stop and
    /// record its evidence; a pending model request is dropped at once.
    async fn run_agent(
        &self,
        state: &AttemptState,
        mut run: AgentRun,
        cancel: &CancellationToken,
    ) -> Ending {
        let deadline = tokio::time::sleep_until(state.deadline.into());
        tokio::select! {
            result = &mut run => return Ending::Finished(result),
            () = cancel.cancelled() => {}
            () = deadline => {}
        }
        state.cancel.cancel();
        if state.tool_running()
            && let Ok(result) = tokio::time::timeout(TOOL_GRACE, &mut run).await
        {
            return Ending::Finished(result);
        }
        Ending::Interrupted
    }

    /// Decides the attempt's result. An accepted assessment stands, even if
    /// the run was stopped right after it.
    fn conclude(
        &self,
        state: &AttemptState,
        ending: Ending,
        cancel: &CancellationToken,
    ) -> AttemptResult {
        if let Some(assessment) = state.assessment() {
            return AttemptResult::assessed(assessment);
        }
        if cancel.is_cancelled() {
            return AttemptResult::cancelled();
        }
        if let Some(error) = state.halted() {
            return AttemptResult::failed(error);
        }
        let error = match ending {
            Ending::Finished(Ok(())) => self.unassessed(state),
            Ending::Finished(Err(error)) if !matches!(*error, PromptError::Cancelled { .. }) => {
                self.llm.classify(&error)
            }
            _ => ExecutionError::new(
                ErrorKind::LimitExceeded,
                format!(
                    "the attempt did not finish within its deadline ({}s per step, capped by the run deadline)",
                    self.limits.step_timeout_secs
                ),
            ),
        };
        AttemptResult::failed(error)
    }

    fn unassessed(&self, state: &AttemptState) -> ExecutionError {
        let problems = state.problems();
        let detail = if problems.is_empty() {
            String::new()
        } else {
            format!("; the last submission was refused: {}", problems.join("; "))
        };
        ExecutionError::new(
            ErrorKind::InvalidAssessment,
            format!(
                "the model ended the step without a valid assessment after {} model requests{detail}",
                state.model_calls().min(self.limits.model_calls_per_step)
            ),
        )
    }

    /// Stops the shell session and everything it started, and shuts the
    /// MCP servers down.
    pub async fn close(&self) {
        self.shell.lock().await.close().await;
        let servers = std::mem::take(&mut *self.mcp_servers.lock().await);
        crate::mcp::shutdown_all(servers).await;
    }
}
