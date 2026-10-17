//! `saci actions`: outcome actions of finished runs, and decisions on the
//! ones waiting for approval (`docs/actions.md`).

use clap::{Args, Subcommand};
use reqwest::Method;
use saci_core::action::{ActionList, ActionRecord, ActionState, DecisionRequest};
use saci_core::ids::RunId;
use serde::Serialize;

use crate::client::{CliError, Client, exit};
use crate::render;

/// What `saci actions` can do.
#[derive(Debug, Subcommand)]
pub enum ActionsCommand {
    /// List a run's actions, or every action waiting for approval.
    List {
        /// The run; without it, actions waiting for approval across runs.
        #[arg(value_parser = parse_run)]
        run: Option<RunId>,
        /// Print the API's JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show one action with the exact arguments it sends and their digest.
    Show(ActionArg),
    /// Approve an action: its recorded arguments are then sent.
    ///
    /// Prints the arguments being approved. With --digest, the approval is
    /// refused unless they are the ones you reviewed earlier with `show`.
    Approve(Decide),
    /// Reject an action: it is never sent.
    Reject(Decide),
}

/// One action of one run.
#[derive(Debug, Args)]
pub struct ActionArg {
    /// Run ID.
    #[arg(value_parser = parse_run)]
    pub run: RunId,
    /// Action name.
    pub name: String,
    /// Print the API's JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

/// A decision on one action.
#[derive(Debug, Args)]
pub struct Decide {
    #[command(flatten)]
    action: ActionArg,
    /// Digest of the arguments you reviewed, as `show` prints it.
    #[arg(long)]
    digest: Option<String>,
    /// A note kept with the decision.
    #[arg(long)]
    note: Option<String>,
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

/// Runs an actions command.
///
/// # Errors
///
/// Fails when the backend refuses or cannot be reached.
pub async fn run(client: &Client, command: ActionsCommand) -> Result<u8, CliError> {
    match command {
        ActionsCommand::List { run, json } => list(client, run.as_ref(), json).await,
        ActionsCommand::Show(action) => show(client, &action).await,
        ActionsCommand::Approve(decide) => decision(client, &decide, "approval").await,
        ActionsCommand::Reject(decide) => decision(client, &decide, "rejection").await,
    }
}

async fn list(client: &Client, run: Option<&RunId>, json: bool) -> Result<u8, CliError> {
    let route = run.map_or_else(
        || format!("/actions?state={}", ActionState::PendingApproval),
        |run| format!("/runs/{run}/actions"),
    );
    let actions: ActionList = client.get(&route).await?;
    if json {
        print_json(&actions)?;
    } else if actions.items.is_empty() {
        println!("No actions.");
    } else {
        for action in &actions.items {
            let prefix = if run.is_none() {
                format!("{}  ", action.run_id)
            } else {
                String::new()
            };
            println!("{prefix}{}", render::action_line(action));
        }
    }
    Ok(exit::OK)
}

async fn fetch(client: &Client, action: &ActionArg) -> Result<ActionRecord, CliError> {
    let actions: ActionList = client.get(&format!("/runs/{}/actions", action.run)).await?;
    actions
        .items
        .into_iter()
        .find(|item| item.name == action.name)
        .ok_or_else(|| {
            CliError::new(
                exit::INVALID,
                format!("run {} has no action `{}`", action.run, action.name),
            )
        })
}

async fn show(client: &Client, action: &ActionArg) -> Result<u8, CliError> {
    let record = fetch(client, action).await?;
    if action.json {
        print_json(&record)?;
    } else {
        print!("{}", render::action(&record));
    }
    Ok(exit::OK)
}

/// Approves or rejects, naming the digest of the arguments reviewed: the
/// one given, or the one of the arguments printed here.
async fn decision(client: &Client, decide: &Decide, kind: &str) -> Result<u8, CliError> {
    let digest = match &decide.digest {
        Some(digest) => digest.clone(),
        None => {
            let record = fetch(client, &decide.action).await?;
            if !decide.action.json {
                print!("{}", render::action(&record));
            }
            record.digest
        }
    };
    let body = DecisionRequest {
        digest,
        note: decide.note.clone(),
    };
    let route = format!(
        "/runs/{}/actions/{}/{kind}",
        decide.action.run, decide.action.name
    );
    let (_, decided): (_, ActionRecord) = client
        .request(Method::POST, &route, Some(&body), &[])
        .await?;
    if decide.action.json {
        print_json(&decided)?;
    } else {
        println!("{}", render::action_line(&decided));
    }
    Ok(exit::OK)
}
