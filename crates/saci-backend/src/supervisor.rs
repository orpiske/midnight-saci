//! Hands accepted and cancelled runs from the API to the supervision task,
//! which launches and watches workers.

use saci_core::ids::RunId;
use saci_core::run::ContractError;
use saci_core::status::Outcome;
use time::OffsetDateTime;
use tokio::sync::mpsc;

use std::time::Duration;

use crate::api::{Backend, Supervisor, issue_token};
use crate::launcher::{self, LaunchError, Launcher};

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

/// How long a stopped worker gets to report its end before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(30);

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
    let assignment = launcher::assignment(&backend.config, &record.spec, &token, |name| {
        std::env::var(name).ok()
    })?;
    launcher.launch(&assignment).await?;
    tracing::info!(%run, "worker started");
    Ok(())
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
