//! Supervision (`docs/supervision.md`): launching accepted runs, watching
//! their workers, enforcing run deadlines, stopping cancelled runs, ending
//! runs whose workers are lost, removing finished workers and recovering
//! after a backend restart.
//!
//! Every run that reaches a worker ends with a stored outcome: the
//! worker's own `run_finished`, or one the backend records when the worker
//! cannot report it. The backend never repeats a run that may already have
//! acted on its targets.

use std::time::Duration;

use saci_core::ids::RunId;
use saci_core::run::{ContractError, WorkerImage};
use saci_core::status::{Lifecycle, Outcome};
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::api::{Backend, Supervisor, backend_redactor, issue_token};
use crate::launcher::{self, LaunchError, Launcher, WorkerState};
use crate::store::RunRecord;

/// A request from the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Launch the run's worker.
    Launch(RunId),
    /// Stop the run's worker.
    Cancel(RunId),
}

/// The API's handle: forwards requests to the supervision task.
#[derive(Debug, Clone)]
pub struct Commands(mpsc::UnboundedSender<Command>);

impl Supervisor for Commands {
    fn submitted(&self, run: RunId) {
        self.send(Command::Launch(run));
    }

    fn cancel_requested(&self, run: RunId) {
        self.send(Command::Cancel(run));
    }
}

impl Commands {
    fn send(&self, command: Command) {
        if self.0.send(command).is_err() {
            tracing::error!("the supervision task has stopped; request dropped");
        }
    }
}

/// Requests waiting for the supervision task.
pub type CommandReceiver = mpsc::UnboundedReceiver<Command>;

/// Creates the API's handle and the supervision task's receiver.
#[must_use]
pub fn channel() -> (Commands, CommandReceiver) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (Commands(sender), receiver)
}

/// Supervision intervals.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How often a worker's state is checked when nothing else happens.
    pub poll: Duration,
    /// SIGTERM-to-SIGKILL grace when a worker is stopped.
    pub stop_grace: Duration,
    /// Time past `limits.run_timeout_secs` before the backend stops a worker
    /// that has not finished; the worker enforces the limit itself first.
    pub deadline_grace: Duration,
    /// How long a finished run's worker may take to exit before it is stopped.
    pub exit_grace: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(2),
            stop_grace: Duration::from_secs(30),
            deadline_grace: Duration::from_secs(60),
            exit_grace: Duration::from_secs(30),
        }
    }
}

/// Supervises runs until the API's handle is dropped: first recovers runs
/// left unfinished by an earlier backend process, then launches each
/// accepted run and watches its worker.
pub async fn run(
    backend: Backend,
    launcher: Result<Launcher, LaunchError>,
    mut commands: CommandReceiver,
    timing: Timing,
) {
    let mut recovered = match &launcher {
        Ok(launcher) => Box::pin(recover(&backend, launcher, timing)).await,
        Err(error) => {
            fail_unfinished(&backend, error).await;
            Vec::new()
        }
    };
    while let Some(command) = commands.recv().await {
        match (command, &launcher) {
            // Submitted while recovery ran, and already launched by it.
            (Command::Launch(run), Ok(_)) if recovered.contains(&run) => {
                recovered.retain(|launched| *launched != run);
            }
            (Command::Launch(run), Ok(launcher)) => supervise(&backend, launcher, run, timing),
            (Command::Launch(run), Err(error)) => launch_failed(&backend, &run, error).await,
            // A watcher sees the request in the run record and stops the worker.
            (Command::Cancel(run), _) => tracing::info!(%run, "cancellation requested"),
        }
    }
}

/// Launches `run` and watches its worker, in the background.
fn supervise(backend: &Backend, launcher: &Launcher, run: RunId, timing: Timing) {
    let (backend, launcher) = (backend.clone(), launcher.clone());
    tokio::spawn(Box::pin(async move {
        if Box::pin(start(&backend, &launcher, &run)).await {
            Box::pin(watch(&backend, &launcher, &run, timing)).await;
        }
    }));
}

/// Ends every unfinished run when no worker can be launched.
async fn fail_unfinished(backend: &Backend, error: &LaunchError) {
    for record in backend.store.unfinished().await.unwrap_or_default() {
        launch_failed(backend, &record.spec.run_id, error).await;
    }
}

async fn launch_failed(backend: &Backend, run: &RunId, error: &LaunchError) {
    let reason = format!("the worker could not be launched: {}", error.message);
    finish(backend, run, Outcome::Error, reason).await;
}

/// Launches `run`'s worker. When the result of a launch is unknown, the
/// runtime is asked whether the worker exists; a launch is repeated only
/// when it certainly does not, and the stable name prevents a duplicate.
/// Returns whether a worker is there to watch; otherwise the run has ended.
async fn start(backend: &Backend, launcher: &Launcher, run: &RunId) -> bool {
    let first = Box::pin(launch(backend, launcher, run)).await;
    let Err(error) = first else { return true };
    if !error.uncertain {
        launch_failed(backend, run, &error).await;
        return false;
    }
    tracing::warn!(%run, %error, "launch outcome unknown; asking the runtime");
    let outcome = match launcher.state(run).await {
        Ok(WorkerState::Missing) => match Box::pin(launch(backend, launcher, run)).await {
            Err(error) if !error.message.contains("already in use") => Err(error),
            _ => Ok(()),
        },
        Ok(_) => Ok(()),
        Err(state) => Err(LaunchError::new(format!(
            "{}; the runtime cannot say whether it started: {}",
            error.message, state.message
        ))),
    };
    match outcome {
        Ok(()) => true,
        Err(error) => {
            launch_failed(backend, run, &error).await;
            false
        }
    }
}

/// Issues the run's worker token and starts its worker.
///
/// # Errors
///
/// Fails when the run cannot be read, a value it needs is missing, or the
/// runtime refuses.
pub async fn launch(
    backend: &Backend,
    launcher: &Launcher,
    run: &RunId,
) -> Result<(), LaunchError> {
    let record = backend
        .store
        .run(run)
        .await
        .map_err(|error| LaunchError::new(format!("cannot read the run: {error}")))?;
    let token = issue_token(backend, run).await.map_err(|error| {
        LaunchError::new(format!("cannot issue the worker token: {:?}", error.code()))
    })?;
    let lookup = |name: &str| backend.credential(name);
    let assignment = launcher::assignment(&backend.config, &record.spec, &token, lookup)?;
    launcher.launch(&assignment).await?;
    tracing::info!(%run, "worker started");
    Ok(())
}

/// What a watcher has done to the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stopping {
    No,
    Cancelled,
    Deadline,
}

/// Watches `run`'s worker until the run has ended and the worker is gone.
async fn watch(backend: &Backend, launcher: &Launcher, run: &RunId, timing: Timing) {
    let mut changes = backend.subscribe();
    let mut stopping = Stopping::No;
    let mut image = None;
    loop {
        // Before the worker can be removed, which ends what the runtime knows.
        if !image.as_ref().is_some_and(WorkerImage::is_resolved) {
            image = Box::pin(record_image(backend, launcher, run, image)).await;
        }
        let Some(progress) = Box::pin(progress(backend, run, timing)).await else {
            tracing::error!(%run, "cannot read the run; no longer watching it");
            return;
        };
        if progress == Progress::Finished {
            return Box::pin(clean_up(launcher, run, timing)).await;
        }
        if stopping == Stopping::No
            && let Progress::Stop(reason) = progress
        {
            stopping = reason;
            stop(launcher, run, timing);
        }
        if let Some(code) = Box::pin(gone(launcher, run)).await {
            return Box::pin(ended(backend, launcher, run, stopping, code)).await;
        }
        tokio::select! {
            _ = changes.changed() => {}
            () = tokio::time::sleep(timing.poll) => {}
        }
    }
}

/// Records the worker's image in the report when it differs from `known`,
/// the image recorded before. Returns the image now recorded.
async fn record_image(
    backend: &Backend,
    launcher: &Launcher,
    run: &RunId,
    known: Option<WorkerImage>,
) -> Option<WorkerImage> {
    let started = backend.store.run(run).await.ok();
    let reference =
        started.and_then(|record| launcher::worker_image(&backend.config, &record.spec));
    let image = launcher.image(run, reference.as_deref()).await;
    if known.as_ref() == Some(&image) {
        return known;
    }
    match backend.store.set_worker_image(run, image.clone()).await {
        Ok(()) => {
            backend.notify();
            Some(image)
        }
        Err(error) => {
            tracing::warn!(%run, %error, "cannot record the worker's image");
            known
        }
    }
}

/// Where a watched run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    Finished,
    Running,
    Stop(Stopping),
}

async fn progress(backend: &Backend, run: &RunId, timing: Timing) -> Option<Progress> {
    let record = backend.store.run(run).await.ok()?;
    if record.report.lifecycle == Lifecycle::Finished {
        return Some(Progress::Finished);
    }
    Some(match should_stop(&record, timing) {
        Stopping::No => Progress::Running,
        reason => Progress::Stop(reason),
    })
}

/// `Some(exit code)` once the worker has exited, `Some(None)` when it has
/// disappeared, `None` while it exists.
async fn gone(launcher: &Launcher, run: &RunId) -> Option<Option<i32>> {
    match launcher.state(run).await {
        Ok(WorkerState::Exited(code)) => Some(Some(code)),
        Ok(WorkerState::Missing) => Some(None),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%run, %error, "cannot read the worker's state");
            None
        }
    }
}

async fn is_finished(backend: &Backend, run: &RunId) -> bool {
    let record = backend.store.run(run).await;
    record.is_ok_and(|record| record.report.lifecycle == Lifecycle::Finished)
}

/// Whether the worker must be stopped: cancellation, or a deadline it
/// should have enforced itself. A run whose policy cleans up after a stop
/// gets its cleanup time (`limits.cleanup_timeout_secs`) after the
/// cancellation request and after the run deadline.
fn should_stop(record: &RunRecord, timing: Timing) -> Stopping {
    let now = OffsetDateTime::now_utc();
    let cleanup = record.spec.cleanup_time();
    if let Some(requested) = record.cancel_requested_at
        && (cleanup.is_zero() || now >= requested + cleanup + timing.deadline_grace)
    {
        return Stopping::Cancelled;
    }
    let limit =
        Duration::from_secs(record.spec.limits.run_timeout_secs) + cleanup + timing.deadline_grace;
    if now >= record.spec.created_at + limit {
        Stopping::Deadline
    } else {
        Stopping::No
    }
}

/// Stops the worker in the background, so the watcher keeps observing it.
fn stop(launcher: &Launcher, run: &RunId, timing: Timing) {
    let (launcher, run) = (launcher.clone(), run.clone());
    tokio::spawn(async move {
        if let Err(error) = launcher.stop(&run, timing.stop_grace).await {
            tracing::warn!(%run, %error, "cannot stop the worker");
        }
    });
}

/// The worker is gone but the run has not finished: record why, keeping
/// every event it delivered, then remove the worker.
async fn ended(
    backend: &Backend,
    launcher: &Launcher,
    run: &RunId,
    stopping: Stopping,
    code: Option<i32>,
) {
    // Events sent just before the exit may still be in flight.
    tokio::time::sleep(Duration::from_millis(200)).await;
    if !Box::pin(is_finished(backend, run)).await {
        let (outcome, reason) = loss(backend, launcher, run, stopping, code).await;
        finish(backend, run, outcome, reason).await;
    }
    if let Err(error) = launcher.remove(run).await {
        tracing::warn!(%run, %error, "cannot remove the worker");
    }
}

/// How a run whose worker ended without reporting its end is recorded.
async fn loss(
    backend: &Backend,
    launcher: &Launcher,
    run: &RunId,
    stopping: Stopping,
    code: Option<i32>,
) -> (Outcome, String) {
    match (stopping, code) {
        (Stopping::Cancelled, _) => (
            Outcome::Cancelled,
            "the run was cancelled; its worker stopped before reporting the end".to_owned(),
        ),
        (Stopping::Deadline, _) => (
            Outcome::Error,
            "the run exceeded its deadline (limits.run_timeout_secs) and its worker was stopped"
                .to_owned(),
        ),
        (Stopping::No, Some(code)) => {
            let output = last_output(backend, launcher, run).await;
            let reason =
                format!("the worker exited with code {code} before reporting the end{output}");
            (Outcome::Error, reason)
        }
        (Stopping::No, None) => (
            Outcome::Error,
            "the worker disappeared before reporting the end".to_owned(),
        ),
    }
}

/// The worker's last output lines, redacted, for a lost worker's reason.
async fn last_output(backend: &Backend, launcher: &Launcher, run: &RunId) -> String {
    let Ok(record) = backend.store.run(run).await else {
        return String::new();
    };
    let logs = launcher.logs(run).await;
    let redactor = backend_redactor(&record.spec, |name| backend.credential(name));
    let redacted = redactor.redact(&logs);
    let lines: Vec<&str> = redacted
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let tail = &lines[lines.len().saturating_sub(10)..];
    if tail.is_empty() {
        String::new()
    } else {
        format!("; its last output:\n{}", tail.join("\n"))
    }
}

/// Removes a finished run's worker once it has exited, stopping it if it
/// does not exit by itself.
async fn clean_up(launcher: &Launcher, run: &RunId, timing: Timing) {
    let waited = tokio::time::Instant::now();
    let mut exited = false;
    while !exited && waited.elapsed() < timing.exit_grace {
        exited = matches!(
            launcher.state(run).await,
            Ok(WorkerState::Exited(_) | WorkerState::Missing)
        );
        if !exited {
            tokio::time::sleep(timing.poll).await;
        }
    }
    if !exited {
        let _ = launcher.stop(run, timing.stop_grace).await;
    }
    match launcher.remove(run).await {
        Ok(()) => tracing::info!(%run, "worker removed"),
        Err(error) => tracing::warn!(%run, %error, "cannot remove the worker"),
    }
}

/// Resumes supervision of runs an earlier backend process left unfinished.
///
/// - A run whose worker still exists is watched again; the worker keeps
///   reporting once the backend is back.
/// - A run that was never handed to a worker (accepted, no worker token)
///   has not acted on anything and is launched now.
/// - Any other run may already have executed actions and is not repeated:
///   it ends `error` with the reason.
///
/// Workers left over from runs that had already finished are removed.
///
/// Returns the runs it launched.
async fn recover(backend: &Backend, launcher: &Launcher, timing: Timing) -> Vec<RunId> {
    let records = match backend.store.unfinished().await {
        Ok(records) => records,
        Err(error) => {
            tracing::error!(%error, "cannot list unfinished runs; none recovered");
            return Vec::new();
        }
    };
    let existing = launcher.workers().await.unwrap_or_else(|error| {
        tracing::warn!(%error, "cannot list workers; treating them as gone");
        Vec::new()
    });
    let mut launched = Vec::new();
    for record in &records {
        let exists = existing.contains(&record.spec.run_id);
        if recover_run(backend, launcher, record, exists, timing).await {
            launched.push(record.spec.run_id.clone());
        }
    }
    let unfinished: Vec<&RunId> = records.iter().map(|record| &record.spec.run_id).collect();
    for run in existing.iter().filter(|run| !unfinished.contains(run)) {
        remove_leftover(backend, launcher, run, timing).await;
    }
    launched
}

/// Removes the worker of a finished run whose cleanup an earlier backend
/// process did not complete. Workers of runs this backend does not know
/// are left alone: another backend may own them.
async fn remove_leftover(backend: &Backend, launcher: &Launcher, run: &RunId, timing: Timing) {
    if !is_finished(backend, run).await {
        return;
    }
    tracing::info!(%run, "removing the worker of a run that finished before the restart");
    let (launcher, run) = (launcher.clone(), run.clone());
    tokio::spawn(Box::pin(async move {
        clean_up(&launcher, &run, timing).await;
    }));
}

async fn recover_run(
    backend: &Backend,
    launcher: &Launcher,
    record: &RunRecord,
    worker_exists: bool,
    timing: Timing,
) -> bool {
    let run = record.spec.run_id.clone();
    if worker_exists {
        tracing::info!(%run, "watching a worker started before the restart");
        let (backend, launcher) = (backend.clone(), launcher.clone());
        tokio::spawn(Box::pin(async move {
            watch(&backend, &launcher, &run, timing).await;
        }));
        false
    } else if never_started(record) {
        tracing::info!(%run, "launching a run accepted before the restart");
        supervise(backend, launcher, run, timing);
        true
    } else {
        let reason = "the backend restarted and the run's worker is gone; the run is not \
                      repeated because its actions may already have run";
        finish(backend, &run, Outcome::Error, reason.to_owned()).await;
        false
    }
}

/// A run no worker was ever started for: accepted, without a worker token.
fn never_started(record: &RunRecord) -> bool {
    record.report.lifecycle == Lifecycle::Accepted && record.worker_token_hash.is_none()
}

/// Finishes an unfinished run from the backend side, for example when its
/// worker could not be launched or was lost. A run that already finished is
/// left alone.
pub async fn finish(backend: &Backend, run: &RunId, outcome: Outcome, reason: String) {
    let result = backend
        .store
        .update_report(run, move |report| {
            report.finish(outcome, &reason, OffsetDateTime::now_utc());
            Ok(())
        })
        .await;
    match result {
        Ok(_) | Err(crate::store::StoreError::Contract(ContractError::Finished)) => {}
        Err(error) => tracing::error!(%run, %error, "cannot finish the run"),
    }
    backend.notify();
}
