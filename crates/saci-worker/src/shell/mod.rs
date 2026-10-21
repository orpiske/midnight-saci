//! The run-private, persistent shell session.
//!
//! One `bash` process serves every shell call of a run, so variables,
//! functions, the working directory and files persist between calls and
//! steps. Each command is written to a file and sourced, with standard input
//! from `/dev/null`; a marker carrying `$?` then ends its output on each
//! stream. Nothing is added to the command text itself.
//!
//! On a deadline or cancellation every descendant of the shell is killed and
//! the shell gets `SIGUSR2`, whose trap unwinds the sourced command, so a
//! loop that runs inside the shell stops too and nothing after it runs. This repeats until the
//! command returns, and the session survives. When the command does not
//! return within the grace period (for example, it ignores `SIGUSR2`), the
//! shell is killed too and the loss is recorded. A session that was lost or ended is replaced on the next
//! call, and that call's evidence says so. So is a session whose command was
//! abandoned mid-way (its future dropped), since its state is then unknown.

mod process;
mod scan;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use saci_core::evidence::{ShellEvidence, Termination};
use saci_core::redaction::Redactor;
use saci_core::run::{ErrorKind, ExecutionError};
use tokio_util::sync::CancellationToken;

use process::Process;
use scan::Stream;

/// How long a command may take to return after its descendants are killed.
const GRACE: Duration = Duration::from_secs(2);

/// The shell's `SIGUSR2` trap, quoted. It returns from the current function
/// or sourced file, and a `DEBUG` trap (with `extdebug`, which makes it
/// return) then returns from every outer one until the command's source
/// returns. At the next top-level command the `DEBUG` trap removes itself,
/// so the session continues as it was.
const UNWIND: &str = r#"'builtin shopt -s extdebug; builtin trap "if (( \${#BASH_SOURCE[@]} )); then builtin return 2; fi; builtin trap - DEBUG; builtin shopt -u extdebug" DEBUG; builtin return 130 2>/dev/null'"#;

/// How often a stopped command is killed and interrupted again within
/// [`GRACE`].
const NUDGE: Duration = Duration::from_millis(100);

/// Settings of a shell session.
#[derive(Debug, Clone)]
pub struct ShellConfig {
    /// Initial working directory; files there outlive a lost session.
    pub workspace: PathBuf,
    /// The complete environment of the shell.
    pub environment: BTreeMap<String, String>,
    /// Bytes kept of each output stream.
    pub output_limit: u64,
    /// Redacts captured output and the recorded command.
    pub redactor: Redactor,
}

/// One executed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRun {
    /// Redacted evidence.
    pub evidence: ShellEvidence,
    /// Wall-clock duration.
    pub duration: Duration,
}

/// A persistent shell session.
#[derive(Debug)]
pub struct ShellSession {
    config: ShellConfig,
    process: Option<Process>,
    started: bool,
    control: tempfile::TempDir,
    nonce: String,
    commands: u64,
    /// A command was sent and its result not yet collected. Still set on
    /// the next call only if that call was abandoned (its future dropped),
    /// so the shell's state is unknown and it is replaced.
    busy: bool,
}

impl ShellSession {
    /// Prepares a session; `bash` starts on the first command.
    ///
    /// # Errors
    ///
    /// Fails when the control directory cannot be created.
    pub fn new(config: ShellConfig) -> Result<Self, ExecutionError> {
        let control = tempfile::Builder::new()
            .prefix("saci-shell-")
            .tempdir()
            .map_err(|error| {
                tool_error(format!(
                    "cannot create the shell control directory: {error}"
                ))
            })?;
        Ok(Self {
            config,
            process: None,
            started: false,
            control,
            nonce: uuid::Uuid::new_v4().simple().to_string(),
            commands: 0,
            busy: false,
        })
    }

    /// Runs `command` within `deadline`, stopping early on `cancel`.
    ///
    /// # Errors
    ///
    /// Fails when the shell cannot be started or given the command; a
    /// failing, timed-out or session-ending command is evidence, not an
    /// error.
    pub async fn run(
        &mut self,
        command: &str,
        deadline: Duration,
        cancel: &CancellationToken,
    ) -> Result<ShellRun, ExecutionError> {
        let started = Instant::now();
        self.discard_abandoned().await;
        let restarted = self.ensure_started()?;
        let script = self.write_script(command)?;
        let line = self.protocol_line(&script);
        let process = self
            .process
            .as_mut()
            .ok_or_else(|| tool_error("the shell is not running".into()))?;
        process.drain();
        self.busy = true;
        if let Err(error) = process.send(&line).await {
            self.busy = false;
            self.process = None;
            return Err(tool_error(format!(
                "cannot send the command to the shell: {error}"
            )));
        }
        let mut collector = Collector::new(&self.nonce, &self.config);
        let ending = collector.collect(process, started + deadline, cancel).await;
        self.busy = false;
        let _ = std::fs::remove_file(&script);
        let evidence = self.evidence(command, collector, ending, restarted).await;
        Ok(ShellRun {
            evidence,
            duration: started.elapsed(),
        })
    }

    /// Replaces a session whose last command was abandoned mid-way.
    async fn discard_abandoned(&mut self) {
        if std::mem::take(&mut self.busy) {
            tracing::warn!("an abandoned shell command left the session in an unknown state");
            self.close().await;
        }
    }

    /// Stops the shell and everything it started.
    pub async fn close(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.kill().await;
        }
    }

    /// Starts `bash` if needed; returns whether this replaces a session.
    fn ensure_started(&mut self) -> Result<bool, ExecutionError> {
        if self.process.is_some() {
            return Ok(false);
        }
        std::fs::create_dir_all(&self.config.workspace)
            .map_err(|error| tool_error(format!("cannot create the workspace: {error}")))?;
        let mut environment = self.config.environment.clone();
        environment
            .entry("HOME".to_owned())
            .or_insert_with(|| self.config.workspace.display().to_string());
        let process = Process::spawn(&self.config.workspace, &environment)
            .map_err(|error| tool_error(format!("cannot start bash: {error}")))?;
        self.process = Some(process);
        Ok(std::mem::replace(&mut self.started, true))
    }

    fn write_script(&mut self, command: &str) -> Result<PathBuf, ExecutionError> {
        self.commands += 1;
        let path = self
            .control
            .path()
            .join(format!("command-{}.sh", self.commands));
        std::fs::write(&path, command)
            .map_err(|error| tool_error(format!("cannot write the command file: {error}")))?;
        Ok(path)
    }

    fn protocol_line(&self, script: &std::path::Path) -> String {
        let quoted = script.display().to_string().replace('\'', r"'\''");
        let nonce = &self.nonce;
        format!(
            "builtin trap {UNWIND} USR2; builtin source '{quoted}' </dev/null; builtin printf '\\036SACI-{nonce}:%s\\036' \"$?\"; builtin printf '\\036SACI-{nonce}:\\036' >&2\n"
        )
    }

    async fn evidence(
        &mut self,
        command: &str,
        collector: Collector,
        ending: Ending,
        restarted: bool,
    ) -> ShellEvidence {
        let (termination, exit_code, lost) = self.settle(&collector, ending).await;
        if lost {
            self.process = None;
        }
        let redactor = &self.config.redactor;
        ShellEvidence {
            command: redactor.redact(command).into_owned(),
            stdout: collector.stdout.finish(redactor),
            stderr: collector.stderr.finish(redactor),
            exit_code,
            termination,
            restarted,
            session_lost: lost,
        }
    }

    /// Classifies how the command ended: termination, exit code, and
    /// whether the session was lost.
    async fn settle(
        &mut self,
        collector: &Collector,
        ending: Ending,
    ) -> (Termination, Option<i32>, bool) {
        let Some(process) = self.process.as_mut() else {
            return (Termination::SessionLost, None, true);
        };
        let status = collector.stdout.status();
        match ending {
            Ending::Returned => (Termination::Exited, status, false),
            Ending::Stopped(termination) if collector.returned() => (termination, status, false),
            Ending::Stopped(termination) => {
                process.kill().await;
                (termination, None, true)
            }
            Ending::Closed => {
                let exit = tokio::time::timeout(GRACE, process.wait())
                    .await
                    .ok()
                    .flatten();
                match exit.and_then(|status| status.code()) {
                    Some(code) => (Termination::SessionEnded, Some(code), true),
                    None => {
                        process.kill().await;
                        (Termination::SessionLost, None, true)
                    }
                }
            }
        }
    }
}

fn tool_error(message: String) -> ExecutionError {
    ExecutionError::new(ErrorKind::Tool, message)
}

/// Why collection stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// Both markers arrived.
    Returned,
    /// The deadline or cancellation stopped the command.
    Stopped(Termination),
    /// The shell's output closed: it exited or was killed.
    Closed,
}

struct Collector {
    stdout: Stream,
    stderr: Stream,
}

impl Collector {
    fn new(nonce: &str, config: &ShellConfig) -> Self {
        let marker = format!("\u{1e}SACI-{nonce}:");
        Self {
            stdout: Stream::new(marker.as_bytes(), config.output_limit, &config.redactor),
            stderr: Stream::new(marker.as_bytes(), config.output_limit, &config.redactor),
        }
    }

    const fn returned(&self) -> bool {
        self.stdout.done && self.stderr.done
    }

    async fn collect(
        &mut self,
        process: &mut Process,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Ending {
        let stopped = tokio::select! {
            ending = self.read(process) => return ending,
            () = tokio::time::sleep_until(deadline.into()) => Termination::TimedOut,
            () = cancel.cancelled() => Termination::Cancelled,
        };
        // A loop inside the shell starts new processes after each kill, so
        // kill and interrupt again until the command returns or the grace
        // period ends.
        let until = Instant::now() + GRACE;
        loop {
            process.kill_descendants();
            process.interrupt();
            let next = (Instant::now() + NUDGE).min(until);
            if tokio::time::timeout_at(next.into(), self.read(process))
                .await
                .is_ok()
                || Instant::now() >= until
            {
                break;
            }
        }
        Ending::Stopped(stopped)
    }

    async fn read(&mut self, process: &mut Process) -> Ending {
        while self.stdout.open() || self.stderr.open() {
            tokio::select! {
                chunk = process.stdout.recv(), if self.stdout.open() => self.stdout.receive(chunk),
                chunk = process.stderr.recv(), if self.stderr.open() => self.stderr.receive(chunk),
            }
        }
        if self.returned() {
            Ending::Returned
        } else {
            if process.exited().is_none() {
                tracing::debug!("shell output closed before the command returned");
            }
            Ending::Closed
        }
    }
}
