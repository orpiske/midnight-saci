//! `saci list`, `status`, `follow`, `report` and `cancel`.
//!
//! Commands about one run exit with the run's outcome (`exit::for_outcome`)
//! once it has finished and with `exit::RUNNING` before; see
//! `docs/cli.md#exit-codes`.

use std::time::Duration;

use reqwest::Method;
use saci_core::api::{EventPage, Page, RunSummary};
use saci_core::ids::RunId;
use saci_core::run::RunReport;
use saci_core::status::Lifecycle;
use serde::Serialize;

use crate::client::{CliError, Client, exit};
use crate::render;

/// Seconds each long-poll waits for new events.
const WAIT_SECS: u64 = 30;
/// How long `follow` keeps reconnecting before it gives up.
const RECONNECT_FOR: Duration = Duration::from_secs(300);

/// The run named on the command line.
#[derive(Debug, Clone, clap::Args)]
pub struct RunArg {
    /// Run ID, as printed by `saci run`.
    #[arg(value_parser = parse_run)]
    pub run: RunId,
    /// Print the API's JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

fn parse_run(value: &str) -> Result<RunId, String> {
    RunId::parse(value).map_err(|error| error.to_string())
}

fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::new(exit::BACKEND, error.to_string()))?;
    println!("{text}");
    Ok(())
}

/// The exit code that reflects a run's state.
const fn state_code(run: &RunSummary) -> u8 {
    match run.outcome {
        Some(outcome) if matches!(run.lifecycle, Lifecycle::Finished) => exit::for_outcome(outcome),
        _ => exit::RUNNING,
    }
}

/// `saci list`.
///
/// # Errors
///
/// See [`Client::request`].
pub async fn list(client: &Client, limit: u32, json: bool) -> Result<u8, CliError> {
    let page: Page<RunSummary> = client.get(&format!("/runs?limit={limit}")).await?;
    if json {
        print_json(&page)?;
        return Ok(exit::OK);
    }
    if page.items.is_empty() {
        eprintln!("No runs yet. Submit one with `saci run <plan.md> --environment <name>`.");
    }
    for run in &page.items {
        let state = run
            .outcome
            .map_or_else(|| run.lifecycle.to_string(), |o| o.to_string());
        println!(
            "{}  {state:<12}  {}  {}",
            run.id, run.environment, run.plan_title
        );
    }
    Ok(exit::OK)
}

/// `saci status`.
///
/// # Errors
///
/// See [`Client::request`].
pub async fn status(client: &Client, args: &RunArg) -> Result<u8, CliError> {
    let run: RunSummary = client.get(&format!("/runs/{}", args.run)).await?;
    if args.json {
        print_json(&run)?;
    } else {
        print!("{}", render::summary(&run));
    }
    Ok(state_code(&run))
}

/// `saci report`.
///
/// # Errors
///
/// See [`Client::request`].
pub async fn report(client: &Client, args: &RunArg) -> Result<u8, CliError> {
    let report: RunReport = client.get(&format!("/runs/{}/report", args.run)).await?;
    if args.json {
        print_json(&report)?;
    } else {
        print!("{}", render::report(&report));
    }
    Ok(match report.outcome {
        Some(outcome) if report.lifecycle == Lifecycle::Finished => exit::for_outcome(outcome),
        _ => exit::RUNNING,
    })
}

/// How to print followed events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// One line of text per event.
    Text,
    /// One JSON object per event.
    Json,
    /// Nothing.
    Quiet,
}

/// Prints every event after `after` until the run finishes, reconnecting
/// after connection failures without skipping or repeating events, and
/// returns the finished run.
///
/// # Errors
///
/// Fails when the backend refuses, or stays unreachable for five minutes;
/// the message says how to resume.
pub async fn follow(
    client: &Client,
    run: &RunId,
    mut after: u64,
    output: Output,
) -> Result<RunSummary, CliError> {
    let mut lost: Option<tokio::time::Instant> = None;
    loop {
        let route = format!("/runs/{run}/events?after={after}&wait={WAIT_SECS}");
        match client.get::<EventPage>(&route).await {
            Ok(page) => {
                lost = None;
                for event in &page.items {
                    print_event(event, output)?;
                }
                after = page.next_after;
                if page.finished {
                    return client.get(&format!("/runs/{run}")).await;
                }
            }
            Err(error) if error.retryable => {
                let since = *lost.get_or_insert_with(tokio::time::Instant::now);
                reconnect(error, since, run, after).await?;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Waits before reconnecting, or gives up with how to resume.
async fn reconnect(
    error: CliError,
    since: tokio::time::Instant,
    run: &RunId,
    after: u64,
) -> Result<(), CliError> {
    if since.elapsed() >= RECONNECT_FOR {
        let message = format!(
            "{}\nResume with `saci follow {run} --after {after}`.",
            error.message
        );
        return Err(CliError::new(error.code, message));
    }
    eprintln!("connection lost; reconnecting after event {after}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}

fn print_event(event: &saci_core::event::RunEvent, output: Output) -> Result<(), CliError> {
    match output {
        Output::Text => println!("{}", render::event(event)),
        Output::Json => {
            let line = serde_json::to_string(event)
                .map_err(|error| CliError::new(exit::BACKEND, error.to_string()))?;
            println!("{line}");
        }
        Output::Quiet => {}
    }
    Ok(())
}

/// `saci follow`: prints progress and exits with the run's outcome.
///
/// # Errors
///
/// See [`follow`].
pub async fn follow_command(client: &Client, args: &RunArg, after: u64) -> Result<u8, CliError> {
    let output = if args.json {
        Output::Json
    } else {
        Output::Text
    };
    let run = Box::pin(follow(client, &args.run, after, output)).await?;
    if !args.json {
        eprint!("{}", render::summary(&run));
    }
    Ok(state_code(&run))
}

/// Reports a recorded cancellation request without waiting.
fn requested(run: &RunSummary, json: bool) -> Result<u8, CliError> {
    if json {
        print_json(run)?;
    } else {
        println!(
            "Cancellation requested for {}; the run is {} and stops after the current action. \
             Wait for it with `saci cancel {} --wait` or `saci follow {}`.",
            run.id, run.lifecycle, run.id, run.id
        );
    }
    Ok(exit::OK)
}

/// `saci cancel`: requests cancellation and, with `wait`, waits until the
/// run has stopped.
///
/// # Errors
///
/// See [`Client::request`]; a lost reply exits `exit::UNCERTAIN` and is
/// safe to repeat.
pub async fn cancel(client: &Client, args: &RunArg, wait: bool) -> Result<u8, CliError> {
    let route = format!("/runs/{}/cancellation", args.run);
    let (_, run) = client
        .request::<(), RunSummary>(Method::POST, &route, None, &[])
        .await
        .map_err(|mut error| {
            if error.code == exit::UNCERTAIN {
                error.message.push_str("\nRepeating `saci cancel` is safe.");
            }
            error
        })?;
    if !wait {
        return requested(&run, args.json);
    }
    eprintln!(
        "Cancellation requested for {}; waiting for the run to stop…",
        run.id
    );
    let run = Box::pin(follow(client, &args.run, run.last_event_seq, Output::Quiet)).await?;
    if args.json {
        print_json(&run)?;
    } else {
        print!("{}", render::summary(&run));
    }
    Ok(state_code(&run))
}
