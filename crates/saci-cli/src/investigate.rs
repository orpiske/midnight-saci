//! `saci investigate` and `saci investigations`: start an investigation of
//! a failed unit and show it (`docs/investigations.md`).

use std::fmt::Write as _;

use clap::{Args, Subcommand};
use reqwest::{Method, StatusCode};
use saci_core::api::IDEMPOTENCY_KEY;
use saci_core::ids::{EvidenceId, RunId, UnitKey};
use saci_core::investigation::{
    Hypothesis, Investigation, InvestigationReport, InvestigationRequest, SourceSelection,
};
use saci_core::status::Lifecycle;
use serde::Serialize;
use uuid::Uuid;

use crate::client::{CliError, Client, exit};

/// Arguments of `saci investigate`.
#[derive(Debug, Clone, Args)]
pub struct InvestigateArgs {
    /// The finished run, as printed by `saci run`.
    #[arg(value_parser = parse_run)]
    pub run: RunId,
    /// The failed unit; the first unit that failed, ended in error or was
    /// blocked when omitted.
    #[arg(long)]
    pub unit: Option<String>,
    /// A configured repository to clone, as NAME or NAME=REF; all of them
    /// at their configured refs when omitted. Repeat for more.
    #[arg(long = "source", value_name = "NAME[=REF]", value_parser = parse_source)]
    pub sources: Vec<SourceSelection>,
    /// Key that makes the request safe to repeat; generated when omitted.
    #[arg(long)]
    pub idempotency_key: Option<String>,
    /// Print the investigation as JSON.
    #[arg(long)]
    pub json: bool,
}

/// What `saci investigations` can do.
#[derive(Debug, Subcommand)]
pub enum InvestigationsCommand {
    /// Show an investigation: its state and, once it concluded, the
    /// hypotheses, the cited code and the bug. Exits with its outcome, or
    /// 8 while it runs.
    Show {
        /// Investigation ID, as printed by `saci investigate`.
        #[arg(value_parser = parse_run)]
        id: RunId,
        /// Print the API's JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

fn parse_run(value: &str) -> Result<RunId, String> {
    RunId::parse(value).map_err(|error| error.to_string())
}

fn parse_source(value: &str) -> Result<SourceSelection, String> {
    let (name, git_ref) = match value.split_once('=') {
        Some((name, git_ref)) => (name, Some(git_ref.to_owned())),
        None => (value, None),
    };
    if name.is_empty() {
        return Err("give a repository name, as NAME or NAME=REF".to_owned());
    }
    Ok(SourceSelection {
        name: name.to_owned(),
        git_ref,
    })
}

fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::new(exit::BACKEND, error.to_string()))?;
    println!("{text}");
    Ok(())
}

/// Requests the investigation and prints its ID.
///
/// # Errors
///
/// A [`CliError`] when the backend refuses or cannot be reached; a lost
/// reply names the key that makes a retry safe.
pub async fn start(client: &Client, args: InvestigateArgs) -> Result<u8, CliError> {
    let key = args
        .idempotency_key
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let request = InvestigationRequest {
        run_id: args.run,
        unit: args.unit.map(UnitKey::new),
        sources: args.sources,
    };
    let headers = [(IDEMPOTENCY_KEY, key.clone())];
    let (status, investigation): (_, Investigation) = client
        .request(Method::POST, "/investigations", Some(&request), &headers)
        .await
        .map_err(|error| retry_hint(error, &key))?;
    if args.json {
        print_json(&investigation)?;
    } else {
        print_accepted(&investigation, status);
    }
    Ok(exit::OK)
}

fn print_accepted(investigation: &Investigation, status: StatusCode) {
    let Investigation {
        id,
        unit,
        parent_run,
        ..
    } = investigation;
    println!("{id}");
    let note = if status == StatusCode::OK {
        "already exists"
    } else {
        "accepted"
    };
    eprintln!(
        "Investigation {id} of unit {unit} of {parent_run} {note}. Follow it with `saci follow {id}`."
    );
}

/// Adds the safe retry to a failure whose effect is unknown.
fn retry_hint(mut error: CliError, key: &str) -> CliError {
    if error.code == exit::UNCERTAIN {
        let _ = write!(
            error.message,
            "\nRepeat the same command with --idempotency-key {key}: the backend returns the \
             investigation it already accepted instead of starting a second one."
        );
    }
    error
}

/// Runs an investigations command.
///
/// # Errors
///
/// Fails when the backend refuses or cannot be reached.
pub async fn run(client: &Client, command: InvestigationsCommand) -> Result<u8, CliError> {
    let InvestigationsCommand::Show { id, json } = command;
    let investigation: Investigation = client.get(&format!("/investigations/{id}")).await?;
    if json {
        print_json(&investigation)?;
    } else {
        print!("{}", render(&investigation));
    }
    Ok(match investigation.outcome {
        Some(outcome) => exit::for_outcome(outcome.run_outcome()),
        None => exit::RUNNING,
    })
}

/// An investigation as text.
#[must_use]
pub fn render(investigation: &Investigation) -> String {
    let state = match (investigation.lifecycle, investigation.outcome) {
        (Lifecycle::Finished, Some(outcome)) => outcome.as_str().to_owned(),
        (lifecycle, _) => format!("{lifecycle:?}").to_lowercase(),
    };
    let mut text = format!(
        "Investigation {} of unit {} of {}: {state}\n",
        investigation.id, investigation.unit, investigation.parent_run
    );
    if let Some(reason) = &investigation.reason {
        let _ = writeln!(text, "Reason: {reason}");
    }
    for repository in &investigation.repositories {
        let _ = writeln!(
            text,
            "Source: {} at {} ({})",
            repository.name, repository.git_ref, repository.url
        );
    }
    if let Some(result) = &investigation.result {
        text.push_str(&result_text(result));
    }
    text
}

fn result_text(result: &InvestigationReport) -> String {
    let mut text = String::new();
    for source in &result.sources {
        let _ = writeln!(text, "Cloned: {} at commit {}", source.name, source.commit);
    }
    let _ = writeln!(text, "\n{}", result.summary.trim());
    for (number, hypothesis) in (1..).zip(&result.hypotheses) {
        text.push_str(&hypothesis_text(number, hypothesis));
    }
    match (&result.bug, &result.no_bug_reason) {
        (Some(bug), _) => {
            let _ = writeln!(text, "\nBug: {} <{}>", bug.title, bug.url);
        }
        (None, Some(reason)) => {
            let _ = writeln!(text, "\nNo bug filed: {reason}");
        }
        (None, None) => {}
    }
    text
}

fn hypothesis_text(number: u32, hypothesis: &Hypothesis) -> String {
    let confidence = format!("{:?}", hypothesis.confidence).to_lowercase();
    let mut text = format!(
        "\n{number}. [{confidence}] {}\n",
        hypothesis.statement.trim()
    );
    for code in &hypothesis.code {
        let _ = writeln!(
            text,
            "   {} {}:{}-{} at {}",
            code.repository, code.path, code.start_line, code.end_line, code.commit
        );
    }
    if !hypothesis.evidence.is_empty() {
        let ids: Vec<&str> = hypothesis.evidence.iter().map(EvidenceId::as_str).collect();
        let _ = writeln!(text, "   Evidence: {}", ids.join(", "));
    }
    text
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn sources_are_names_with_an_optional_ref() {
        let source = parse_source("orders=v2.4.1").unwrap();
        assert_eq!(source.name, "orders");
        assert_eq!(source.git_ref.as_deref(), Some("v2.4.1"));
        assert_eq!(parse_source("orders").unwrap().git_ref, None);
        assert!(parse_source("=v2").is_err());
    }

    #[test]
    fn a_concluded_investigation_shows_the_cited_code_and_the_bug() {
        let investigation: Investigation = serde_json::from_value(serde_json::json!({
            "id": "run-9", "parent_run": "run-2b", "unit": "1.2", "plan_title": "Orders", "unit_title": "Total",
            "lifecycle": "finished",
            "outcome": "concluded", "reason": "the investigation ended concluded",
            "repositories": [{"name": "orders", "url": "https://e.com/o.git", "ref": "v2"}],
            "created_at": "2026-10-23T10:00:00Z",
            "result": {
                "outcome": "concluded", "summary": "The total drops the currency.",
                "sources": [{"name": "orders", "commit": "abc"}],
                "hypotheses": [{"statement": "Line 3 drops it.", "confidence": "high",
                    "evidence": ["ev-3"], "code": [{"repository": "orders", "commit": "abc",
                    "path": "src/total.rs", "start_line": 3, "end_line": 5}]}],
                "bug": {"url": "https://e.com/o/issues/7", "title": "Total without currency"}
            }
        }))
        .unwrap();
        let text = render(&investigation);
        assert!(text.starts_with("Investigation run-9 of unit 1.2 of run-2b: concluded\n"));
        assert!(
            text.contains("Source: orders at v2 (https://e.com/o.git)"),
            "{text}"
        );
        assert!(text.contains("1. [high] Line 3 drops it."), "{text}");
        assert!(text.contains("   orders src/total.rs:3-5 at abc"), "{text}");
        assert!(text.contains("   Evidence: ev-3"), "{text}");
        assert!(
            text.contains("Bug: Total without currency <https://e.com/o/issues/7>"),
            "{text}"
        );
    }
}
