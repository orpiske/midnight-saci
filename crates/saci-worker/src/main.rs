#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `saci-worker` entry point (`docs/worker.md`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use clap::Parser;
use saci_core::ids::RunId;
use saci_core::run::ExecutionError;
use saci_core::spec::RunSpec;
use saci_worker::bootstrap::{self, BACKEND_URL_ENV, RUN_ID_ENV, WORKSPACE_ENV};
use saci_worker::credentials::WORKER_TOKEN_ENV;
use saci_worker::events::EventReceiver;
use saci_worker::init;
use saci_worker::llm::Backoff;
use saci_worker::report::{self, BackendClient};
use saci_worker::sequence::Stops;
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

/// Results were delivered, whatever the run's outcome.
const DELIVERED: u8 = 0;
/// The worker could not set up its tools.
const SETUP: u8 = 1;
/// The bootstrap input was rejected; nothing was executed.
const REJECTED: u8 = 2;
/// The results could not be delivered.
const UNDELIVERED: u8 = 3;

/// Saci per-run worker.
#[derive(Debug, Parser)]
#[command(
    name = "saci-worker",
    version,
    about = "Execute one Saci run specification",
    long_about = "Execute one Saci run specification.\n\n\
        Started by the backend with SACI_BACKEND_URL, SACI_RUN_ID and SACI_WORKER_TOKEN, the worker \
        fetches its specification, runs it and reports every event. With --spec it runs a \
        specification file instead and writes the events as JSON lines."
)]
struct Args {
    /// Run this specification file instead of fetching one from the backend.
    #[arg(long)]
    spec: Option<PathBuf>,
    /// With --spec: where to write the events as JSON lines (`-` for standard output).
    #[arg(long, default_value = "-", requires = "spec")]
    events: PathBuf,
    /// Working directory of the run's shell; the current directory by default.
    #[arg(long, env = WORKSPACE_ENV)]
    workspace: Option<PathBuf>,
}

fn main() -> ExitCode {
    if init::is_pid_one() {
        return ExitCode::from(as_init());
    }
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return fail(SETUP, &format!("cannot start the runtime: {error}")),
    };
    let code = runtime.block_on(async move {
        let workspace = match args.workspace.clone() {
            Some(workspace) => workspace,
            None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        let process: BTreeMap<String, String> = std::env::vars().collect();
        match &args.spec {
            Some(path) => Box::pin(local(path, &args.events, &process, &workspace)).await,
            None => Box::pin(assigned(&process, &workspace)).await,
        }
    });
    ExitCode::from(code)
}

/// As PID 1, runs the worker again as a child and reaps every process that
/// ends while it runs (`docs/worker.md#process-1`).
fn as_init() -> u8 {
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            eprintln!("error: cannot find the worker executable: {error}");
            return SETUP;
        }
    };
    let mut command = Command::new(program);
    command.args(std::env::args_os().skip(1));
    init::supervise(command).unwrap_or_else(|error| {
        eprintln!("error: cannot start the worker under its init: {error}");
        SETUP
    })
}

fn fail(code: u8, message: &str) -> ExitCode {
    eprintln!("error: {message}");
    ExitCode::from(code)
}

/// Runs the specification the backend assigned and reports to it.
async fn assigned(process: &BTreeMap<String, String>, workspace: &Path) -> u8 {
    let (client, spec) = match assignment(process).await {
        Ok(assigned) => assigned,
        Err(code) => return code,
    };
    let stops = Stops::new();
    let stop = CancellationToken::new();
    let watcher = spawn_watcher(&client, &stops.cancel, &stop);
    let assignment = Assignment {
        expected: Some(client.run().clone()),
        process,
        workspace,
    };
    let deliver =
        |events| async move { report::deliver(&client, events, Backoff::default()).await };
    let code = Box::pin(execute(spec, assignment, deliver, &stops)).await;
    stop.cancel();
    let _ = watcher.await;
    code
}

/// Connects to the backend and fetches the assigned specification.
async fn assignment(process: &BTreeMap<String, String>) -> Result<(BackendClient, RunSpec), u8> {
    let client = connect(process).map_err(|message| {
        eprintln!("error: {message}");
        REJECTED
    })?;
    match report::fetch_specification(&client, Backoff::default()).await {
        Ok(spec) => Ok((client, spec)),
        Err(error) => {
            eprintln!(
                "error: cannot fetch the run specification: {}",
                error.message
            );
            Err(UNDELIVERED)
        }
    }
}

fn spawn_watcher(
    client: &BackendClient,
    cancel: &CancellationToken,
    stop: &CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let (client, cancel, stop) = (client.clone(), cancel.clone(), stop.clone());
    tokio::spawn(async move {
        report::watch_cancellation(&client, &cancel, &stop, Duration::from_secs(2)).await;
    })
}

fn connect(process: &BTreeMap<String, String>) -> Result<BackendClient, String> {
    let variable = |name: &str| {
        process
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| {
                format!("{name} is not set; the backend's launcher sets it (or pass --spec)")
            })
    };
    let url = variable(BACKEND_URL_ENV)?;
    let run =
        RunId::parse(&variable(RUN_ID_ENV)?).map_err(|error| format!("{RUN_ID_ENV}: {error}"))?;
    let token = variable(WORKER_TOKEN_ENV)?;
    BackendClient::new(&url, run, token).map_err(|error| error.message)
}

/// Runs a specification file and writes its events as JSON lines.
async fn local(
    path: &Path,
    output: &Path,
    process: &BTreeMap<String, String>,
    workspace: &Path,
) -> u8 {
    let spec = match read_spec(path) {
        Ok(spec) => spec,
        Err(message) => {
            eprintln!("error: {message}");
            return REJECTED;
        }
    };
    let writer = match open_events(output).await {
        Ok(writer) => writer,
        Err(error) => {
            eprintln!("error: {}", error.message);
            return UNDELIVERED;
        }
    };
    let output = output.to_owned();
    let deliver = |events| async move { write_events(&output, writer, events).await };
    let assignment = Assignment {
        expected: None,
        process,
        workspace,
    };
    Box::pin(execute(spec, assignment, deliver, &Stops::new())).await
}

fn read_spec(path: &Path) -> Result<RunSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("{} is not a run specification: {error}", path.display()))
}

type EventWriter = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;

fn write_failure(output: &Path, error: &std::io::Error) -> ExecutionError {
    ExecutionError::new(
        saci_core::run::ErrorKind::WorkerLost,
        format!("cannot write events to {}: {error}", output.display()),
    )
}

/// Opens the events destination before the run starts, so a run never
/// acts on the target while its evidence has nowhere to go.
async fn open_events(output: &Path) -> Result<EventWriter, ExecutionError> {
    if output == Path::new("-") {
        return Ok(Box::new(tokio::io::stdout()));
    }
    let file = tokio::fs::File::create(output)
        .await
        .map_err(|error| write_failure(output, &error))?;
    Ok(Box::new(file))
}

async fn write_events(
    output: &Path,
    mut writer: EventWriter,
    mut events: EventReceiver,
) -> Result<u64, ExecutionError> {
    let failed = |error: std::io::Error| write_failure(output, &error);
    let mut last = 0;
    while let Some(event) = events.recv().await {
        last = event.seq;
        let mut line = serde_json::to_vec(&event).unwrap_or_default();
        line.push(b'\n');
        writer.write_all(&line).await.map_err(failed)?;
    }
    writer.flush().await.map_err(failed)?;
    Ok(last)
}

/// What the launcher handed this worker besides the specification.
struct Assignment<'a> {
    expected: Option<RunId>,
    process: &'a BTreeMap<String, String>,
    workspace: &'a Path,
}

fn start(
    spec: RunSpec,
    assignment: &Assignment<'_>,
) -> Result<(Box<bootstrap::Running>, EventReceiver), ExecutionError> {
    let prepared = bootstrap::prepare(
        spec,
        assignment.expected.as_ref(),
        assignment.process,
        assignment.workspace,
    )?;
    bootstrap::executor(prepared)
}

/// Validates and runs `spec`, delivering its events with `deliver`.
async fn execute<D, F>(spec: RunSpec, assignment: Assignment<'_>, deliver: D, stops: &Stops) -> u8
where
    D: FnOnce(EventReceiver) -> F,
    F: Future<Output = Result<u64, ExecutionError>> + Send + 'static,
{
    let run_id = spec.run_id.clone();
    let (running, events) = match start(spec, &assignment) {
        Ok(started) => started,
        Err(error) => {
            eprintln!("error: {}", error.message);
            let code = Box::pin(deliver(bootstrap::reject(run_id, &error))).await;
            return if code.is_ok() { REJECTED } else { UNDELIVERED };
        }
    };
    let delivery = tokio::spawn(stop_on_failure(deliver(events), stops.abort.clone()));
    let signals = tokio::spawn(cancel_on_signal(stops.abort.clone()));
    let ending = Box::pin(bootstrap::execute(running, stops)).await;
    signals.abort();
    eprintln!("run {run_id} ended {}: {}", ending.outcome, ending.reason);
    match delivery.await {
        Ok(Ok(_)) => DELIVERED,
        Ok(Err(_)) => UNDELIVERED,
        Err(error) => {
            eprintln!("error: event delivery stopped: {error}");
            UNDELIVERED
        }
    }
}

/// Evidence that cannot be delivered must not keep the run acting on the
/// target: a failed delivery stops it at once, cleanup included.
async fn stop_on_failure<F>(delivering: F, cancel: CancellationToken) -> Result<u64, ExecutionError>
where
    F: Future<Output = Result<u64, ExecutionError>>,
{
    let delivered = delivering.await;
    if let Err(error) = &delivered {
        eprintln!(
            "error: event delivery failed; stopping the run: {}",
            error.message
        );
        cancel.cancel();
    }
    delivered
}

/// Stopping the container stops the run at once, cleanup included, so it
/// still reports its end before the stop grace runs out.
async fn cancel_on_signal(cancel: CancellationToken) {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        else {
            let _ = interrupt.await;
            cancel.cancel();
            return;
        };
        tokio::select! {
            _ = interrupt => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = interrupt.await;
    }
    tracing::info!("signal received; cancelling the run");
    cancel.cancel();
}
