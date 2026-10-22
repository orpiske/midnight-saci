//! Runs one script with a fixed executable, a validated argument list and
//! bounded resources. This module does not know about MCP.
//!
//! The script starts in a new process group with a cleared environment,
//! `/dev/null` as standard input and a fixed working directory. Standard
//! output and standard error are read at the same time, and only the first
//! `max_output_bytes` of each are kept; the rest is read and discarded so the
//! script never blocks on a full pipe.
//!
//! When the script exits, every process still left in its group gets
//! `SIGKILL`. On timeout or cancellation the group gets `SIGTERM`, then
//! `SIGKILL` after [`KILL_GRACE`]. Output collection then stops after at most
//! [`DRAIN_GRACE`], even when a process that left the group keeps a pipe
//! open.

use std::collections::BTreeMap;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Time between `SIGTERM` and `SIGKILL` when a script is stopped.
pub const KILL_GRACE: Duration = Duration::from_secs(2);
/// Longest wait for the output pipes to close after the script ends.
pub const DRAIN_GRACE: Duration = Duration::from_secs(1);

/// What to run.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The script.
    pub executable: &'a Path,
    /// The argument list, without the executable.
    pub arguments: &'a [String],
    /// The working directory.
    pub working_directory: &'a Path,
    /// The complete environment.
    pub environment: &'a BTreeMap<String, String>,
    /// The deadline, from spawn to exit.
    pub timeout: Duration,
    /// Bytes kept from each output stream.
    pub max_output_bytes: usize,
}

/// How an execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The script exited with code 0.
    Completed,
    /// The script exited with another code, or a signal stopped it.
    Failed,
    /// The deadline expired and the script was stopped.
    TimedOut,
    /// The call was cancelled and the script was stopped.
    Cancelled,
    /// The script could not be started.
    SpawnFailed,
}

impl Status {
    /// The name used in results and audit events.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::SpawnFailed => "spawn_failed",
        }
    }
}

/// The result of one execution.
#[derive(Debug, Clone, Serialize)]
pub struct Execution {
    /// How the execution ended.
    pub status: Status,
    /// The exit code, when the script exited normally.
    pub exit_code: Option<i32>,
    /// The signal that stopped the script, if any.
    pub signal: Option<i32>,
    /// Time from spawn to the end of output collection.
    pub duration_ms: u64,
    /// Standard output, as lossy UTF-8.
    pub stdout: String,
    /// Standard error, as lossy UTF-8.
    pub stderr: String,
    /// Standard output was longer than the limit.
    pub stdout_truncated: bool,
    /// Standard error was longer than the limit.
    pub stderr_truncated: bool,
    /// Why the script could not start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct Captured {
    bytes: Vec<u8>,
    truncated: bool,
}

enum End {
    Exited(Option<ExitStatus>),
    TimedOut,
    Cancelled,
}

/// Runs the script until it exits, the deadline expires or `cancelled`
/// completes. If the returned future is dropped, the process group is
/// killed.
pub async fn run(request: &Request<'_>, cancelled: impl Future<Output = ()>) -> Execution {
    let started = Instant::now();
    match command(request).spawn() {
        Ok(child) => Box::pin(supervise(child, request, cancelled, started)).await,
        Err(error) => not_started(started, &error),
    }
}

async fn supervise(
    mut child: Child,
    request: &Request<'_>,
    cancelled: impl Future<Output = ()>,
    started: Instant,
) -> Execution {
    let mut group = ProcessGroup::new(child.id());
    let stop = CancellationToken::new();
    let _stop_readers = stop.clone().drop_guard();
    let limit = request.max_output_bytes;
    let stdout = tokio::spawn(capture(child.stdout.take(), limit, stop.clone()));
    let stderr = tokio::spawn(capture(child.stderr.take(), limit, stop.clone()));

    let end = tokio::select! {
        biased;
        status = child.wait() => End::Exited(status.ok()),
        () = tokio::time::sleep_until(started + request.timeout) => End::TimedOut,
        () = cancelled => End::Cancelled,
    };
    let exit = match end {
        End::Exited(status) => {
            group.signal(Signal::SIGKILL);
            status
        }
        End::TimedOut | End::Cancelled => terminate(&mut child, &group).await,
    };
    group.disarm();
    let (stdout, stderr) = drain(stdout, stderr, &stop).await;
    finish(&end, exit, started, &stdout, &stderr)
}

/// Waits for both readers, and stops them after [`DRAIN_GRACE`].
async fn drain(
    stdout: JoinHandle<Captured>,
    stderr: JoinHandle<Captured>,
    stop: &CancellationToken,
) -> (Captured, Captured) {
    let mut output = std::pin::pin!(async {
        (
            stdout.await.unwrap_or_default(),
            stderr.await.unwrap_or_default(),
        )
    });
    if let Ok(output) = tokio::time::timeout(DRAIN_GRACE, &mut output).await {
        return output;
    }
    stop.cancel();
    output.await
}

fn command(request: &Request<'_>) -> Command {
    let mut command = Command::new(request.executable);
    command
        .args(request.arguments)
        .current_dir(request.working_directory)
        .env_clear()
        .envs(request.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    command
}

/// Sends `SIGTERM` to the group, then `SIGKILL` after [`KILL_GRACE`], and
/// reaps the script.
async fn terminate(child: &mut Child, group: &ProcessGroup) -> Option<ExitStatus> {
    group.signal(Signal::SIGTERM);
    let graceful = tokio::time::timeout(KILL_GRACE, child.wait()).await;
    group.signal(Signal::SIGKILL);
    match graceful {
        Ok(status) => status.ok(),
        Err(_) => {
            let _ = child.start_kill();
            child.wait().await.ok()
        }
    }
}

fn finish(
    end: &End,
    exit: Option<ExitStatus>,
    started: Instant,
    stdout: &Captured,
    stderr: &Captured,
) -> Execution {
    let exit_code = exit.and_then(|status| status.code());
    let status = match end {
        End::Exited(_) if exit_code == Some(0) => Status::Completed,
        End::Exited(_) => Status::Failed,
        End::TimedOut => Status::TimedOut,
        End::Cancelled => Status::Cancelled,
    };
    Execution {
        status,
        exit_code,
        signal: exit.and_then(|status| status.signal()),
        duration_ms: elapsed_ms(started),
        stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        error: None,
    }
}

fn not_started(started: Instant, error: &std::io::Error) -> Execution {
    Execution {
        status: Status::SpawnFailed,
        exit_code: None,
        signal: None,
        duration_ms: elapsed_ms(started),
        stdout: String::new(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        error: Some(format!("cannot start the script: {error}")),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Reads `pipe` to the end or until `stop`, keeping at most `limit` bytes.
async fn capture(
    pipe: Option<impl AsyncRead + Unpin>,
    limit: usize,
    stop: CancellationToken,
) -> Captured {
    let mut captured = Captured::default();
    let Some(mut pipe) = pipe else {
        return captured;
    };
    let mut chunk = vec![0_u8; 8192];
    loop {
        let read = tokio::select! {
            read = pipe.read(&mut chunk) => read,
            () = stop.cancelled() => break,
        };
        let count = match read {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let room = limit.saturating_sub(captured.bytes.len());
        captured.bytes.extend_from_slice(&chunk[..count.min(room)]);
        captured.truncated |= count > room;
    }
    captured
}

/// The script's process group. Dropping it before [`ProcessGroup::disarm`]
/// kills the whole group.
struct ProcessGroup {
    id: Option<Pid>,
}

impl ProcessGroup {
    fn new(pid: Option<u32>) -> Self {
        let id = pid
            .and_then(|pid| i32::try_from(pid).ok())
            .map(Pid::from_raw);
        Self { id }
    }

    fn signal(&self, signal: Signal) {
        if let Some(id) = self.id {
            // ESRCH means the group is already gone.
            let _ = killpg(id, signal);
        }
    }

    const fn disarm(&mut self) {
        self.id = None;
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.signal(Signal::SIGKILL);
    }
}
