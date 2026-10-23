#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `saci` command-line entry point.

mod actions;
mod check;
mod client;
mod inspect;
mod investigate;
mod new;
mod render;
mod run;
mod skills;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::client::{CliError, Client, Connection, exit};

/// Saci, an AI test engineer that executes Markdown test plans.
#[derive(Debug, Parser)]
#[command(name = "saci", version, about = "Saci, an AI test engineer")]
struct Cli {
    #[command(flatten)]
    connection: Connection,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create files from built-in templates. Works offline.
    #[command(subcommand)]
    New(NewCommand),
    /// Validate a plan bundle offline, without submitting it.
    ///
    /// Reads the plan and every procedure it invokes, reports every problem
    /// `saci run` would refuse, and warns about content that would never
    /// run, such as a phase without tests or steps. Exits 2 when the plan is
    /// invalid, or with --strict when there are warnings.
    Check {
        /// Entry document of the plan bundle.
        plan: PathBuf,
        /// Treat warnings as errors.
        #[arg(long)]
        strict: bool,
        /// Directory holding the documents the plan was converted from, laid
        /// out like the plan's. Exits 2 unless every code block in the
        /// bundle is identical to one there and none is missing.
        #[arg(long, value_name = "DIR")]
        original: Option<PathBuf>,
    },
    /// List and install the agent skills that ship with Saci. Works offline.
    ///
    /// Skills teach coding agents such as Claude Code, Codex, Cursor,
    /// Copilot, Gemini CLI or OpenCode to do Saci tasks, for example to
    /// convert existing test plans.
    #[command(subcommand)]
    Skill(skills::SkillCommand),
    /// Run the backend.
    #[command(subcommand)]
    Backend(BackendCommand),
    /// Validate a plan bundle locally and submit it to the backend.
    ///
    /// Prints the accepted run ID on standard output. A submission whose
    /// reply was lost exits 4 and names the --idempotency-key that makes
    /// repeating it safe.
    Run(run::RunArgs),
    /// List recent runs, newest first.
    List {
        /// How many runs to show (1-200).
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Print the API's JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show a run's state. Exits with its outcome, or 8 while it runs.
    Status(inspect::RunArg),
    /// Print a run's progress until it finishes, then exit with its outcome.
    ///
    /// Waits on the backend 30 seconds at a time and reconnects after
    /// connection failures for up to five minutes without skipping or
    /// repeating events. Ctrl-C stops following, not the run.
    Follow {
        #[command(flatten)]
        run: inspect::RunArg,
        /// Start after this event sequence number (resume an earlier follow).
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
    /// Print a run's report: units, attempts, criteria and evidence.
    Report(inspect::RunArg),
    /// Outcome actions of finished runs: list, show, approve, reject,
    /// redispatch and reconcile.
    ///
    /// Actions run after a run's report is final and never change its
    /// outcome. Approval-required actions wait for `approve`.
    #[command(subcommand)]
    Actions(actions::ActionsCommand),
    /// Investigate a failed unit of a finished run.
    ///
    /// An agent clones the configured source code, finds the probable cause
    /// of the failure and reports a bug. Prints the investigation ID; follow
    /// it with `saci follow <id>` and show its result with
    /// `saci investigations show <id>`.
    Investigate(investigate::InvestigateArgs),
    /// Show investigations.
    #[command(subcommand)]
    Investigations(investigate::InvestigationsCommand),
    /// Request cancellation of a run.
    ///
    /// The worker stops after its current action; the remaining units are
    /// skipped. Without --wait the command returns once the request is
    /// recorded.
    Cancel {
        #[command(flatten)]
        run: inspect::RunArg,
        /// Wait until the run has stopped and exit with its outcome.
        #[arg(long)]
        wait: bool,
    },
}

#[derive(Debug, Subcommand)]
enum NewCommand {
    /// Create an editable Markdown test plan from the starter template.
    ///
    /// The title is derived from the file name. The plan follows the
    /// conventions in docs/test-plans.md: `## Phase <id>: <title>`,
    /// `### Test <id>: <title>` or `### Step <id>: <title>`, and an
    /// `**Expected result:**` list for every unit. Existing files are never
    /// overwritten.
    TestPlan {
        /// Destination file, for example saci-on-the-cloud.md.
        file: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum BackendCommand {
    /// Start the backend in the foreground; stop it with Ctrl-C or SIGTERM.
    Start {
        /// Configuration file (docs/configuration.md).
        #[arg(long, short, default_value = "saci.yaml")]
        config: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::New(NewCommand::TestPlan { file }) => new::test_plan(&file)
            .map(|()| exit::OK)
            .map_err(CliError::from),
        Command::Check {
            plan,
            strict,
            original,
        } => check::plan(&plan, strict, original.as_deref()),
        Command::Skill(command) => skills::run(command)
            .map(|()| exit::OK)
            .map_err(CliError::from),
        Command::Backend(BackendCommand::Start { config }) => start_backend(config),
        command => {
            let connection = cli.connection;
            block_on(async move { Box::pin(remote(&Client::new(&connection)?, command)).await })
        }
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {}", error.message);
            error.exit_code()
        }
    }
}

/// Commands that talk to a backend.
async fn remote(client: &Client, command: Command) -> Result<u8, CliError> {
    match command {
        Command::Run(args) => Box::pin(run::submit(client, args)).await,
        Command::List { limit, json } => Box::pin(inspect::list(client, limit, json)).await,
        Command::Status(run) => Box::pin(inspect::status(client, &run)).await,
        Command::Follow { run, after } => {
            Box::pin(inspect::follow_command(client, &run, after)).await
        }
        Command::Report(run) => Box::pin(inspect::report(client, &run)).await,
        Command::Cancel { run, wait } => Box::pin(inspect::cancel(client, &run, wait)).await,
        Command::Actions(command) => Box::pin(actions::run(client, command)).await,
        Command::Investigate(args) => Box::pin(investigate::start(client, args)).await,
        Command::Investigations(command) => Box::pin(investigate::run(client, command)).await,
        Command::New(_) | Command::Check { .. } | Command::Skill(_) | Command::Backend(_) => {
            Ok(exit::OK)
        }
    }
}

fn start_backend(config: PathBuf) -> Result<u8, CliError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    block_on(async move {
        Box::pin(saci_backend::server::run(&config))
            .await
            .map(|()| exit::OK)
            .map_err(|error| CliError::new(exit::INVALID, format!("{error:#}")))
    })
}

fn block_on(future: impl Future<Output = Result<u8, CliError>>) -> Result<u8, CliError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            CliError::new(exit::BACKEND, format!("cannot start the runtime: {error}"))
        })?;
    runtime.block_on(future)
}
