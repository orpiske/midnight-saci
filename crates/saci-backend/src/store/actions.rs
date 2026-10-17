//! Outcome action records (`docs/actions.md`).
//!
//! A run's triggered actions are inserted by [`enqueue`] in the transaction
//! that makes its report final. The dispatcher claims queued actions one at
//! a time; the claim is durable before any call is made.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use saci_core::action::{
    self, ActionDefinition, ActionRecord, ActionState, Approval, Decision, Verdict,
};
use saci_core::ids::RunId;
use saci_core::run::RunReport;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use super::{Store, StoreError, format_time, parse, parse_time};

const COLUMNS: &str = "run_id, name, server, tool, approval, state, arguments, digest, attempts, \
                       created_at, updated_at, result, error, decision, decided_at, decision_note";

/// How a dispatch ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// [`ActionState::Completed`] or [`ActionState::Error`].
    pub state: ActionState,
    /// The tool's result text, when it answered.
    pub result: Option<String>,
    /// Why delivery failed.
    pub error: Option<String>,
}

/// Records the actions `report`'s outcome triggers, with their arguments
/// rendered from the report. Recording an action twice keeps the first.
pub(super) fn enqueue(
    transaction: &Transaction<'_>,
    id: &str,
    report: &RunReport,
) -> Result<(), StoreError> {
    let Some(outcome) = report.outcome else {
        return Ok(());
    };
    let stored: String =
        transaction.query_row("SELECT actions FROM runs WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
    let definitions: BTreeMap<String, ActionDefinition> = parse(&stored)?;
    let context = action::context(report);
    let now = format_time(OffsetDateTime::now_utc())?;
    for (name, definition) in definitions {
        if definition.triggered_by(outcome) {
            let arguments = action::render(&definition.arguments, &context);
            insert(transaction, (id, &name, &now), &definition, &arguments)?;
        }
    }
    Ok(())
}

/// Inserts a triggered action, unless the run already has it.
fn insert(
    transaction: &Transaction<'_>,
    (id, name, now): (&str, &str, &str),
    definition: &ActionDefinition,
    arguments: &serde_json::Value,
) -> Result<(), StoreError> {
    let text = serde_json::to_string(arguments)?;
    let state = match definition.approval {
        Approval::None => ActionState::Queued,
        Approval::Required => ActionState::PendingApproval,
    };
    {
        transaction.execute(
            "INSERT OR IGNORE INTO actions (run_id, name, server, tool, approval, state, arguments, digest, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                id,
                name,
                definition.server,
                definition.tool,
                approval_name(definition.approval),
                state.as_str(),
                text,
                digest(&text),
                now,
            ],
        )?;
    }
    Ok(())
}

/// Hex SHA-256 of the canonical (serialized) arguments.
#[must_use]
pub fn digest(arguments: &str) -> String {
    Sha256::digest(arguments.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const fn approval_name(approval: Approval) -> &'static str {
    match approval {
        Approval::None => "none",
        Approval::Required => "required",
    }
}

/// An `actions` row as stored.
struct Stored {
    run_id: String,
    name: String,
    server: String,
    tool: String,
    approval: String,
    state: String,
    arguments: String,
    digest: String,
    attempts: i64,
    created_at: String,
    updated_at: String,
    result: Option<String>,
    error: Option<String>,
    decision: Option<String>,
    decided_at: Option<String>,
    decision_note: Option<String>,
}

fn read(row: &Row<'_>) -> rusqlite::Result<Stored> {
    Ok(Stored {
        run_id: row.get(0)?,
        name: row.get(1)?,
        server: row.get(2)?,
        tool: row.get(3)?,
        approval: row.get(4)?,
        state: row.get(5)?,
        arguments: row.get(6)?,
        digest: row.get(7)?,
        attempts: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        result: row.get(11)?,
        error: row.get(12)?,
        decision: row.get(13)?,
        decided_at: row.get(14)?,
        decision_note: row.get(15)?,
    })
}

fn unreadable(what: &str) -> StoreError {
    StoreError::Database(format!("unreadable action {what}"))
}

fn decision(stored: &Stored) -> Result<Option<Decision>, StoreError> {
    let (Some(verdict), Some(at)) = (&stored.decision, &stored.decided_at) else {
        return Ok(None);
    };
    let verdict = match verdict.as_str() {
        "approved" => Verdict::Approved,
        "rejected" => Verdict::Rejected,
        _ => return Err(unreadable("decision")),
    };
    Ok(Some(Decision {
        verdict,
        at: parse_time(at)?,
        note: stored.decision_note.clone(),
    }))
}

fn record(stored: Stored) -> Result<ActionRecord, StoreError> {
    let decision = decision(&stored)?;
    Ok(ActionRecord {
        run_id: RunId::parse(&stored.run_id).map_err(|_| unreadable("run"))?,
        name: stored.name,
        server: stored.server,
        tool: stored.tool,
        approval: if stored.approval == "required" {
            Approval::Required
        } else {
            Approval::None
        },
        state: ActionState::parse(&stored.state).ok_or_else(|| unreadable("state"))?,
        arguments: parse(&stored.arguments)?,
        digest: stored.digest,
        decision,
        attempts: u32::try_from(stored.attempts).map_err(|_| unreadable("attempts"))?,
        created_at: parse_time(&stored.created_at)?,
        updated_at: parse_time(&stored.updated_at)?,
        result: stored.result,
        error: stored.error,
    })
}

fn select(
    connection: &Connection,
    filter: &str,
    values: impl rusqlite::Params,
) -> Result<Vec<ActionRecord>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM actions {filter}");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map(values, read)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().map(record).collect()
}

/// Refuses a decision unless the action awaits approval and `digest` is
/// its recorded arguments' digest.
fn check_pending(current: &ActionRecord, digest: &str) -> Result<(), StoreError> {
    if current.state != ActionState::PendingApproval {
        return Err(StoreError::ActionNotPending(current.state));
    }
    if current.digest != digest {
        return Err(StoreError::DigestMismatch);
    }
    Ok(())
}

fn one(connection: &Connection, run: &str, name: &str) -> Result<ActionRecord, StoreError> {
    select(
        connection,
        "WHERE run_id = ?1 AND name = ?2",
        params![run, name],
    )?
    .pop()
    .ok_or(StoreError::NotFound)
}

impl Store {
    /// The actions of a run, in name order.
    ///
    /// # Errors
    ///
    /// Fails with [`StoreError::NotFound`] when the run does not exist.
    pub async fn actions(&self, run: &RunId) -> Result<Vec<ActionRecord>, StoreError> {
        let run = run.as_str().to_owned();
        self.with(move |connection| {
            let exists: Option<i64> = connection
                .query_row("SELECT 1 FROM runs WHERE id = ?1", [&run], |row| row.get(0))
                .optional()?;
            exists.ok_or(StoreError::NotFound)?;
            select(connection, "WHERE run_id = ?1 ORDER BY name", [&run])
        })
        .await
    }

    /// Claims the oldest queued action for dispatch: it becomes `executing`
    /// with one more attempt, durably, before it is returned.
    ///
    /// # Errors
    ///
    /// Fails on a database error.
    pub async fn claim_action(&self) -> Result<Option<ActionRecord>, StoreError> {
        self.with(|connection| {
            let transaction = connection.transaction()?;
            let next: Option<(String, String)> = transaction
                .query_row(
                    "SELECT run_id, name FROM actions WHERE state = 'queued' ORDER BY updated_at, rowid LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((run, name)) = next else {
                return Ok(None);
            };
            transaction.execute(
                "UPDATE actions SET state = 'executing', attempts = attempts + 1, updated_at = ?3, error = NULL
                 WHERE run_id = ?1 AND name = ?2",
                params![run, name, format_time(OffsetDateTime::now_utc())?],
            )?;
            let claimed = one(&transaction, &run, &name)?;
            transaction.commit()?;
            Ok(Some(claimed))
        })
        .await
    }

    /// Records how a claimed action's dispatch ended. Only an `executing`
    /// action changes.
    ///
    /// # Errors
    ///
    /// Fails with [`StoreError::NotFound`] when the action is not executing.
    pub async fn finish_action(
        &self,
        run: &RunId,
        name: &str,
        delivery: Delivery,
    ) -> Result<ActionRecord, StoreError> {
        let (run, name) = (run.as_str().to_owned(), name.to_owned());
        self.with(move |connection| {
            let changed = connection.execute(
                "UPDATE actions SET state = ?3, result = ?4, error = ?5, updated_at = ?6
                 WHERE run_id = ?1 AND name = ?2 AND state = 'executing'",
                params![
                    run,
                    name,
                    delivery.state.as_str(),
                    delivery.result,
                    delivery.error,
                    format_time(OffsetDateTime::now_utc())?
                ],
            )?;
            if changed == 0 {
                return Err(StoreError::NotFound);
            }
            one(connection, &run, &name)
        })
        .await
    }

    /// Records a decision on an action awaiting approval. `digest` must be
    /// the recorded arguments' digest: the reviewer decides on exactly
    /// that payload. Approval queues it for dispatch; rejection is final.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] without such an action,
    /// [`StoreError::ActionNotPending`] when it is not awaiting approval,
    /// [`StoreError::DigestMismatch`] for another digest.
    pub async fn decide_action(
        &self,
        run: &RunId,
        name: &str,
        verdict: Verdict,
        request: action::DecisionRequest,
    ) -> Result<ActionRecord, StoreError> {
        let (run, name) = (run.as_str().to_owned(), name.to_owned());
        self.with(move |connection| {
            let transaction = connection.transaction()?;
            let current = one(&transaction, &run, &name)?;
            check_pending(&current, &request.digest)?;
            let state = match verdict {
                Verdict::Approved => ActionState::Queued,
                Verdict::Rejected => ActionState::Rejected,
            };
            transaction.execute(
                "UPDATE actions SET state = ?3, decision = ?4, decided_at = ?5, decision_note = ?6, updated_at = ?5
                 WHERE run_id = ?1 AND name = ?2 AND state = 'pending_approval'",
                params![
                    run,
                    name,
                    state.as_str(),
                    verdict.as_str(),
                    format_time(OffsetDateTime::now_utc())?,
                    request.note
                ],
            )?;
            let decided = one(&transaction, &run, &name)?;
            transaction.commit()?;
            Ok(decided)
        })
        .await
    }

    /// Actions in `state` across runs, oldest first, at most `limit`.
    ///
    /// # Errors
    ///
    /// Fails on a database error.
    pub async fn actions_in_state(
        &self,
        state: ActionState,
        limit: u32,
    ) -> Result<Vec<ActionRecord>, StoreError> {
        self.with(move |connection| {
            select(
                connection,
                "WHERE state = ?1 ORDER BY created_at, rowid LIMIT ?2",
                params![state.as_str(), limit],
            )
        })
        .await
    }

    /// Actions left `executing` by a backend that stopped, oldest first.
    ///
    /// # Errors
    ///
    /// Fails on a database error.
    pub async fn executing_actions(&self) -> Result<Vec<ActionRecord>, StoreError> {
        self.with(|connection| {
            select(
                connection,
                "WHERE state = 'executing' ORDER BY updated_at, rowid",
                [],
            )
        })
        .await
    }
}
