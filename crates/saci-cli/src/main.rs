#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `saci` command-line entry point.

mod client;
mod new;
mod run;

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
    /// Run the backend.
    #[command(subcommand)]
    Backend(BackendCommand),
    /// Validate a plan bundle locally and submit it to the backend.
    ///
    /// Prints the accepted run ID on standard output. A submission whose
    /// reply was lost exits 4 and names the --idempotency-key that makes
    /// repeating it safe.
    Run(run::RunArgs),
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
        Command::New(NewCommand::TestPlan { file }) => {
            new::test_plan(&file).map_err(CliError::from)
        }
        Command::Backend(BackendCommand::Start { config }) => start_backend(config),
        Command::Run(args) => {
            let connection = cli.connection;
            block_on(async move { run::submit(&Client::new(&connection)?, args).await })
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {}", error.message);
            error.exit_code()
        }
    }
}

fn start_backend(config: PathBuf) -> Result<(), CliError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    block_on(async move {
        Box::pin(saci_backend::server::run(&config))
            .await
            .map_err(|error| CliError::new(exit::INVALID, format!("{error:#}")))
    })
}

fn block_on(future: impl Future<Output = Result<(), CliError>>) -> Result<(), CliError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            CliError::new(exit::BACKEND, format!("cannot start the runtime: {error}"))
        })?;
    runtime.block_on(future)
}
