//! `saci run`: validates a plan bundle locally and submits it.

use std::path::PathBuf;

use reqwest::{Method, StatusCode};
use saci_core::api::{IDEMPOTENCY_KEY, RunSummary, SubmitRun};
use saci_core::bundle;
use uuid::Uuid;

use crate::client::{CliError, Client, exit};

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
    /// Key that makes the submission safe to repeat; generated when omitted.
    #[arg(long)]
    pub idempotency_key: Option<String>,
    /// Print the accepted run as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Submits the plan and prints the accepted run.
///
/// # Errors
///
/// A [`CliError`] for invalid plans, unreachable or refusing backends, and
/// lost replies; the latter names the key that makes a retry safe.
pub async fn submit(client: &Client, args: RunArgs) -> Result<(), CliError> {
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
    };
    let headers = [(IDEMPOTENCY_KEY, key.clone())];
    let result = client
        .request::<_, RunSummary>(Method::POST, "/runs", Some(&request), &headers)
        .await;
    let (status, run) = result.map_err(|error| retry_hint(error, &key))?;
    print(&run, status, args.json)
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
