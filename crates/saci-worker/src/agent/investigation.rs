//! The one attempt of an investigation run (`docs/plans/investigations.md`).
//!
//! The worker clones each repository and runs the `prepare` commands before
//! the first model request. The model then reads the code with the shell
//! and the MCP tools, and finishes with `conclude`. Every code location it
//! cites is checked against the clones before the result is accepted.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rig::completion::PromptError;
use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};
use saci_core::event::EventKind;
use saci_core::evidence::EvidenceRecord;
use saci_core::investigation::{
    CodeLocation, InvestigationOutcome, InvestigationReport, InvestigationSpec, Repository, Source,
    conclusion_schema, prepare_command,
};
use saci_core::run::{ErrorKind, ExecutionError, UnitIdentity};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::state::{AttemptState, Setup};
use super::{AttemptResult, Ending, Executor, Turn, hook, mcp, prompt, tools};

/// Name of the investigation's finishing tool.
pub const CONCLUDE: &str = "conclude";

/// The investigation to run.
#[derive(Debug, Clone, Copy)]
pub struct InvestigationRequest<'a> {
    /// What to investigate.
    pub spec: &'a InvestigationSpec,
    /// When the investigation must end.
    pub deadline: Instant,
    /// Cancels the run.
    pub cancel: &'a CancellationToken,
}

/// A cloned repository at its checked-out commit.
#[derive(Debug, Clone)]
pub struct Checkout {
    /// The repository.
    pub repository: Repository,
    /// The full commit SHA.
    pub commit: String,
    /// Where it is cloned.
    pub dir: PathBuf,
}

impl Executor {
    /// Runs the investigation's attempt and reports its start, evidence,
    /// conclusion and end. Returns the result and the accepted conclusion.
    pub async fn investigate(
        &self,
        request: InvestigationRequest<'_>,
    ) -> (AttemptResult, Option<InvestigationReport>) {
        let identity = request.spec.unit_identity();
        let events = self.recorder.events();
        events.emit(Some(&identity), Some(1), EventKind::AttemptStarted);
        let state = Arc::new(AttemptState::new(Setup {
            unit: identity.clone(),
            attempt: 1,
            criteria: Vec::new(),
            limits: self.limits,
            deadline: request.deadline,
            cancel: request.cancel.child_token(),
            recorder: self.recorder.clone(),
        }));
        let result = Box::pin(self.run_investigation(&state, request)).await;
        let result = AttemptResult {
            timing: Some(state.timing()),
            ..result
        };
        let conclusion = state.conclusion();
        if let Some(report) = &conclusion {
            let kind = EventKind::InvestigationConcluded {
                report: Box::new(report.clone()),
            };
            events.emit(Some(&identity), Some(1), kind);
        }
        finished(&identity, &result, self);
        (result, conclusion)
    }

    async fn run_investigation(
        &self,
        state: &Arc<AttemptState>,
        request: InvestigationRequest<'_>,
    ) -> AttemptResult {
        let spec = request.spec;
        let checkouts = match Box::pin(self.checkout_all(state, spec)).await {
            Ok(checkouts) => Arc::new(checkouts),
            Err(error) => return stopped(state, request.cancel, error),
        };
        let prepared = Box::pin(self.prepare_all(state, spec, &checkouts)).await;
        if let Err(reason) = state.check_interrupted() {
            let error = ExecutionError::new(ErrorKind::Tool, reason);
            return stopped(state, request.cancel, error);
        }
        let tools = match self.investigation_tools(state, &checkouts) {
            Ok(tools) => tools,
            Err(error) => return AttemptResult::failed(error),
        };
        let system = prompt::investigation_system(&self.environment);
        let turn = Turn {
            system: &system,
            feedback: hook::CONCLUDE_FEEDBACK,
            text: prompt::investigation(spec, &checkouts, &prepared),
        };
        let run = self.start(state, tools, turn);
        let ending = Box::pin(self.run_agent(state, run, request.cancel)).await;
        self.concluded(state, ending, request.cancel)
    }

    /// Clones every repository. The first failure stops the investigation.
    async fn checkout_all(
        &self,
        state: &AttemptState,
        spec: &InvestigationSpec,
    ) -> Result<Vec<Checkout>, ExecutionError> {
        let mut checkouts = Vec::new();
        for repository in &spec.repositories {
            checkouts.push(Box::pin(self.checkout(state, repository)).await?);
        }
        Ok(checkouts)
    }

    /// Clones one repository into `src/<name>` of the workspace.
    async fn checkout(
        &self,
        state: &AttemptState,
        repository: &Repository,
    ) -> Result<Checkout, ExecutionError> {
        let dir = self.workspace.join("src").join(&repository.name);
        let evidence = self.run_block(state, &clone_script(repository, &dir)).await;
        let commit = evidence
            .as_ref()
            .and_then(|evidence| match &evidence.record {
                EvidenceRecord::Shell(shell) if shell.exit_code == Some(0) => {
                    commit_of(&shell.stdout.text)
                }
                _ => None,
            });
        let Some(commit) = commit else {
            let reason = evidence.map_or_else(
                || "the shell failed".to_owned(),
                |evidence| format!("see evidence {}", evidence.id),
            );
            return Err(ExecutionError::new(
                ErrorKind::Tool,
                format!(
                    "cannot clone `{}` at `{}` ({reason}); the investigation needs the code",
                    repository.name, repository.git_ref
                ),
            ));
        };
        Ok(Checkout {
            repository: repository.clone(),
            commit,
            dir,
        })
    }

    /// Runs each `prepare` command in each clone and returns the rendered
    /// evidence. A failed command does not stop the investigation.
    async fn prepare_all(
        &self,
        state: &AttemptState,
        spec: &InvestigationSpec,
        checkouts: &[Checkout],
    ) -> Vec<String> {
        let values = spec.prepare_values();
        let mut rendered = Vec::new();
        for checkout in checkouts {
            for command in &spec.prepare {
                let script = prepare_script(&checkout.dir, &values, command);
                let Some(evidence) = self.run_block(state, &script).await else {
                    return rendered;
                };
                rendered.push(super::render::evidence(&evidence));
            }
        }
        rendered
    }

    fn investigation_tools(
        &self,
        state: &Arc<AttemptState>,
        checkouts: &Arc<Vec<Checkout>>,
    ) -> Result<Vec<DynamicTool>, ExecutionError> {
        let mut tools = vec![
            tools::shell(state.clone(), self.shell.clone())?,
            conclude(state.clone(), checkouts.clone())?,
        ];
        for binding in &self.mcp {
            let redactor = self.recorder.redactor().clone();
            tools.push(mcp::tool(state.clone(), binding.clone(), redactor)?);
        }
        Ok(tools)
    }

    /// Decides the result. An accepted conclusion stands, even if the run
    /// was stopped right after it.
    fn concluded(
        &self,
        state: &AttemptState,
        ending: Ending,
        cancel: &CancellationToken,
    ) -> AttemptResult {
        if let Some(report) = state.conclusion() {
            return AttemptResult {
                outcome: report.outcome.run_outcome(),
                assessment: None,
                error: None,
                timing: None,
            };
        }
        if cancel.is_cancelled() {
            return AttemptResult::cancelled();
        }
        if let Some(error) = state.halted() {
            return AttemptResult::failed(error);
        }
        let error = match ending {
            Ending::Finished(Ok(())) => unconcluded(state),
            Ending::Finished(Err(error)) if !matches!(*error, PromptError::Cancelled { .. }) => {
                self.llm.classify(&error)
            }
            _ => ExecutionError::new(
                ErrorKind::LimitExceeded,
                "the investigation did not finish within the run deadline (limits.run_timeout_secs)",
            ),
        };
        AttemptResult::failed(error)
    }
}

/// The result of an attempt that stopped before the model was asked.
fn stopped(
    state: &AttemptState,
    cancel: &CancellationToken,
    error: ExecutionError,
) -> AttemptResult {
    if cancel.is_cancelled() {
        return AttemptResult::cancelled();
    }
    AttemptResult::failed(state.halted().unwrap_or(error))
}

fn finished(identity: &UnitIdentity, result: &AttemptResult, executor: &Executor) {
    let kind = EventKind::AttemptFinished {
        outcome: result.outcome,
        assessment: None,
        error: result.error.clone(),
        timing: result.timing,
    };
    executor
        .recorder
        .events()
        .emit(Some(identity), Some(1), kind);
}

fn unconcluded(state: &AttemptState) -> ExecutionError {
    let problems = state.problems();
    let detail = if problems.is_empty() {
        String::new()
    } else {
        format!("; the last conclusion was refused: {}", problems.join("; "))
    };
    ExecutionError::new(
        ErrorKind::InvalidAssessment,
        format!(
            "the model ended the investigation without a valid conclusion after {} model requests{detail}",
            state.model_calls()
        ),
    )
}

/// Quotes `value` for bash.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Clones `repository` into `dir`, checks out its ref and prints the commit.
/// A branch name resolves to the remote branch when no local one exists.
fn clone_script(repository: &Repository, dir: &Path) -> String {
    let (url, git_ref) = (quote(&repository.url), quote(&repository.git_ref));
    let dir = quote(&dir.display().to_string());
    format!(
        "git clone --quiet -- {url} {dir} && \
         commit=$(git -C {dir} rev-parse --verify --quiet {git_ref}^{{commit}} \
           || git -C {dir} rev-parse --verify --quiet origin/{git_ref}^{{commit}}) && \
         git -C {dir} checkout --quiet --detach \"$commit\" && echo \"$commit\""
    )
}

/// The last line of `stdout` when it is a full commit SHA.
fn commit_of(stdout: &str) -> Option<String> {
    let line = stdout.lines().last()?.trim();
    let hex = line
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    (hex && matches!(line.len(), 40 | 64)).then(|| line.to_owned())
}

/// Runs a `prepare` command in `dir`, in a subshell so it leaves the
/// session as it was, with the template values in environment variables.
fn prepare_script(dir: &Path, values: &[(&str, String)], command: &str) -> String {
    let mut script = format!("( cd {} || exit; ", quote(&dir.display().to_string()));
    for (variable, value) in values {
        script.push_str(&format!("export {variable}={}; ", quote(value)));
    }
    script.push_str(&prepare_command(command));
    script.push_str("\n)");
    script
}

/// The investigation's finishing tool.
fn conclude(
    state: Arc<AttemptState>,
    checkouts: Arc<Vec<Checkout>>,
) -> Result<DynamicTool, ExecutionError> {
    let description = "Submit the result of the investigation, once. `concluded` needs at least one \
        hypothesis with code locations in the cloned commits; otherwise use `inconclusive`. Cite only \
        evidence IDs from this investigation's tool results.";
    Ok(DynamicTool::new(
        tools::tool_name(CONCLUDE)?,
        description,
        conclusion_schema(),
        move |args| {
            let (state, checkouts) = (state.clone(), checkouts.clone());
            Box::pin(async move { submit(&state, &checkouts, args).await })
        },
    ))
}

async fn submit(
    state: &AttemptState,
    checkouts: &[Checkout],
    args: Value,
) -> Result<ToolOutput, ToolExecutionError> {
    let mut report: InvestigationReport = serde_json::from_value(args).map_err(|error| {
        let problem = format!("the conclusion does not match the tool schema: {error}");
        state.refuse(vec![problem.clone()]);
        ToolExecutionError::invalid_args(format!("{problem}. Call conclude again."))
    })?;
    report.sources = checkouts
        .iter()
        .map(|checkout| Source {
            name: checkout.repository.name.clone(),
            commit: checkout.commit.clone(),
        })
        .collect();
    let problems = Box::pin(problems(state, checkouts, &report)).await;
    if problems.is_empty() {
        state.accept_conclusion(report);
        return Ok(ToolOutput::text(
            "Conclusion recorded. The investigation is complete.",
        ));
    }
    let list: Vec<String> = problems.iter().map(|p| format!("- {p}")).collect();
    state.refuse(problems);
    Err(ToolExecutionError::invalid_args(format!(
        "The conclusion was not accepted:\n{}\nCorrect it and call conclude again.",
        list.join("\n")
    )))
}

/// Everything wrong with a submitted conclusion. Code locations are looked
/// up in the clones only when the rest is valid.
async fn problems(
    state: &AttemptState,
    checkouts: &[Checkout],
    report: &InvestigationReport,
) -> Vec<String> {
    let mut problems = report.check().err().unwrap_or_default();
    if !matches!(
        report.outcome,
        InvestigationOutcome::Concluded | InvestigationOutcome::Inconclusive
    ) {
        problems.push("outcome must be `concluded` or `inconclusive`".to_owned());
    }
    let recorded = state.recorder.recorded();
    for id in report.hypotheses.iter().flat_map(|h| &h.evidence) {
        if !recorded.contains_key(id) {
            problems.push(format!("{id} is not evidence of this investigation"));
        }
    }
    if !problems.is_empty() {
        return problems;
    }
    for location in report.hypotheses.iter().flat_map(|h| &h.code) {
        if let Some(problem) = Box::pin(locate(state, checkouts, location)).await {
            problems.push(problem);
        }
    }
    problems
}

/// Why `location` does not exist in its clone, if it does not.
async fn locate(
    state: &AttemptState,
    checkouts: &[Checkout],
    location: &CodeLocation,
) -> Option<String> {
    let checkout = checkouts
        .iter()
        .find(|checkout| checkout.repository.name == location.repository)?;
    let object = format!("{}:{}", location.commit, location.path);
    let show = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&checkout.dir)
        .args(["show", "--no-textconv", &object])
        .kill_on_drop(true)
        .output();
    let missing = || {
        format!(
            "`{}` does not exist in `{}` at {}",
            location.path, location.repository, location.commit
        )
    };
    let output = match tokio::time::timeout(state.operation_deadline(), show).await {
        Ok(Ok(output)) if output.status.success() => output.stdout,
        Ok(_) => return Some(missing()),
        Err(_) => return Some(format!("looking up `{}` took too long", location.path)),
    };
    let lines = line_count(&output);
    (u64::from(location.end_line) > lines).then(|| {
        format!(
            "`{}` has {lines} lines at {}, so lines {}-{} do not exist",
            location.path, location.commit, location.start_line, location.end_line
        )
    })
}

fn line_count(bytes: &[u8]) -> u64 {
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    let unterminated = bytes.last().is_some_and(|byte| *byte != b'\n');
    u64::try_from(newlines + usize::from(unterminated)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_quoted_for_bash() {
        assert_eq!(quote("it's $(id)"), r"'it'\''s $(id)'");
    }

    #[test]
    fn only_a_full_sha_is_a_commit() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(commit_of(&format!("noise\n{sha}\n")), Some(sha.to_owned()));
        assert_eq!(commit_of("0123abc"), None);
        assert_eq!(commit_of(&sha.to_uppercase()), None);
        assert_eq!(commit_of(""), None);
    }

    #[test]
    fn lines_are_counted_with_or_without_a_final_newline() {
        assert_eq!(line_count(b""), 0);
        assert_eq!(line_count(b"a\nb\n"), 2);
        assert_eq!(line_count(b"a\nb"), 2);
    }

    #[test]
    fn prepare_runs_in_a_subshell_with_quoted_values() {
        let values = [("SACI_FAILURE_TITLE", "it's \"x\"".to_owned())];
        let script = prepare_script(Path::new("/w/src/a"), &values, "echo \"{{failure.title}}\"");
        assert_eq!(
            script,
            "( cd '/w/src/a' || exit; export SACI_FAILURE_TITLE='it'\\''s \"x\"'; \
             echo \"${SACI_FAILURE_TITLE}\"\n)"
        );
    }
}
