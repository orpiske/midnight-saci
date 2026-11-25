//! `saci run`: validates a plan bundle locally and submits it.

use std::path::PathBuf;

use reqwest::{Method, StatusCode};
use saci_core::api::{IDEMPOTENCY_KEY, RunSummary, SubmitRun};
use saci_core::bundle;
use saci_core::ids::UnitKey;
use uuid::Uuid;

use crate::client::{CliError, Client, exit};
use crate::inspect::{self, RunArg};

/// Arguments of `saci run`.
#[derive(Debug, Clone, clap::Args)]
pub struct RunArgs {
    /// Entry document of the plan bundle.
    pub plan: PathBuf,
    /// Configured environment to test.
    #[arg(long, short)]
    pub environment: String,
    /// Configured run policy; the backend uses `default` when omitted.
    #[arg(long)]
    pub policy: Option<String>,
    /// Start at this unit key, including its invoked procedures.
    #[arg(long, value_name = "UNIT_KEY")]
    pub from_step: Option<String>,
    /// Key that makes the submission safe to repeat; generated when omitted.
    #[arg(long)]
    pub idempotency_key: Option<String>,
    /// Print the accepted run as JSON (with --follow, every event as a JSON line).
    #[arg(long)]
    pub json: bool,
    /// Follow the run's progress until it finishes and exit with its outcome.
    #[arg(long)]
    pub follow: bool,
}

/// Submits the plan and prints the accepted run.
///
/// # Errors
///
/// A [`CliError`] for invalid plans, unreachable or refusing backends, and
/// lost replies; the latter names the key that makes a retry safe.
pub async fn submit(client: &Client, args: RunArgs) -> Result<u8, CliError> {
    let bundle = bundle::load(&args.plan).map_err(|error| {
        CliError::new(
            exit::INVALID,
            format!("{} is not a valid plan:\n{error}", args.plan.display()),
        )
    })?;
    let key = args
        .idempotency_key
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let request = SubmitRun {
        bundle,
        environment: args.environment,
        policy: args.policy,
        from_step: args.from_step.map(UnitKey::new),
    };
    let (status, run) = send(client, &request, &key).await?;
    if !args.follow {
        return print(&run, status, args.json).map(|()| exit::OK);
    }
    eprintln!(
        "Run {} accepted; following it (Ctrl-C stops following, not the run).",
        run.id
    );
    let followed = RunArg {
        run: run.id,
        json: args.json,
    };
    Box::pin(inspect::follow_command(client, &followed, 0)).await
}

async fn send(
    client: &Client,
    request: &SubmitRun,
    key: &str,
) -> Result<(StatusCode, RunSummary), CliError> {
    let headers = [(IDEMPOTENCY_KEY, key.to_owned())];
    client
        .request(Method::POST, "/runs", Some(request), &headers)
        .await
        .map_err(|error| retry_hint(error, key))
}

fn print(run: &RunSummary, status: StatusCode, json: bool) -> Result<(), CliError> {
    if json {
        let text = serde_json::to_string_pretty(run)
            .map_err(|error| CliError::new(exit::BACKEND, error.to_string()))?;
        println!("{text}");
    } else {
        println!("{}", run.id);
        let note = if status == StatusCode::OK {
            "already accepted for this idempotency key"
        } else {
            "accepted"
        };
        eprintln!(
            "Run {} {note}: {} units of \"{}\" in environment {}.",
            run.id, run.total_units, run.plan_title, run.environment
        );
    }
    Ok(())
}

/// Adds the safe retry to a failure whose effect is unknown.
fn retry_hint(mut error: CliError, key: &str) -> CliError {
    if error.code == exit::UNCERTAIN {
        error.message.push_str(&format!(
            "\nRepeat the same command with --idempotency-key {key}: the backend returns the run it \
             already accepted instead of starting a second one."
        ));
    }
    error
}
