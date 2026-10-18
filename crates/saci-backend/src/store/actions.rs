//! Outcome action records (`docs/actions.md`).
//!
//! A run's triggered actions are inserted by [`enqueue`] in the transaction
//! that makes its report final. The dispatcher claims queued actions one at
//! a time; the claim is durable before any call is made. Every change of
//! state is appended to the action's history in the same transaction.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use saci_core::action::{
    self, ActionDefinition, ActionEvent, ActionRecord, ActionState, Approval, Decision,
    HistoryEntry, ReconcileRequest, Verdict,
};
use saci_core::ids::RunId;
use saci_core::run::RunReport;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use super::{Store, StoreError, format_time, parse, parse_time};

const COLUMNS: &str = "run_id, name, server, tool, approval, state, arguments, digest, attempts, \
                       created_at, updated_at, result, error, decision, decided_at, decision_note, \
                       idempotency_key";

/// How a dispatch ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// [`ActionState::Completed`], [`ActionState::Error`],
    /// [`ActionState::Unknown`], or [`ActionState::Queued`] to replay the
    /// call with its idempotency key.
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
            let mut arguments = action::render(&definition.arguments, &context);
            let key = definition.idempotency_key.as_ref().map(|argument| {
                let key = action::idempotency_key(id, &name);
                if let Some(object) = arguments.as_object_mut() {
                    object.insert(argument.clone(), serde_json::Value::String(key.clone()));
                }
                key
            });
            insert(
                transaction,
                (id, &name, &now),
                &definition,
                (&arguments, key),
            )?;
        }
    }
    Ok(())
}

/// Inserts a triggered action, unless the run already has it.
fn insert(
    transaction: &Transaction<'_>,
    (id, name, now): (&str, &str, &str),
    definition: &ActionDefinition,
    (arguments, key): (&serde_json::Value, Option<String>),
) -> Result<(), StoreError> {
    let text = serde_json::to_string(arguments)?;
    let state = match definition.approval {
        Approval::None => ActionState::Queued,
        Approval::Required => ActionState::PendingApproval,
    };
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO actions (run_id, name, server, tool, approval, state, arguments, digest, created_at, updated_at, idempotency_key)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
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
            key,
        ],
    )?;
    if inserted == 1 {
        log(transaction, id, name, ActionEvent::Recorded, None)?;
    }
    Ok(())
}

/// Appends to an action's history, with its state and attempts as they
/// now are.
fn log(
    connection: &Connection,
    run: &str,
    name: &str,
    event: ActionEvent,
    detail: Option<&str>,
) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO action_history (run_id, name, seq, at, event, state, attempt, detail)
         SELECT run_id, name,
                (SELECT COALESCE(MAX(seq), 0) + 1 FROM action_history WHERE run_id = ?1 AND name = ?2),
                ?3, ?4, state, attempts, ?5
         FROM actions WHERE run_id = ?1 AND name = ?2",
        params![
            run,
            name,
            format_time(OffsetDateTime::now_utc())?,
            event.as_str(),
            detail
        ],
    )?;
    Ok(())
}

/// The event that records a dispatch ending in `state`.
const fn delivery_event(state: ActionState) -> ActionEvent {
    match state {
        ActionState::Completed => ActionEvent::Answered,
        ActionState::Unknown => ActionEvent::Uncertain,
        ActionState::Queued => ActionEvent::Replayed,
        _ => ActionEvent::Failed,
    }
}

const fn verdict_event(verdict: Verdict) -> ActionEvent {
    match verdict {
        Verdict::Approved => ActionEvent::Approved,
        Verdict::Rejected => ActionEvent::Rejected,
    }
}

/// `seq`'s fields: `at`, `event`, `state`, `attempt`, `detail`.
type StoredEntry = (String, String, String, i64, Option<String>);

fn history(
    connection: &Connection,
    run: &str,
    name: &str,
) -> Result<Vec<HistoryEntry>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT at, event, state, attempt, detail FROM action_history
         WHERE run_id = ?1 AND name = ?2 ORDER BY seq",
    )?;
    let rows = statement
        .query_map(params![run, name], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<Vec<StoredEntry>, _>>()?;
    rows.into_iter()
        .map(|(at, event, state, attempt, detail)| {
            Ok(HistoryEntry {
                at: parse_time(&at)?,
                event: ActionEvent::parse(&event).ok_or_else(|| unreadable("event"))?,
                state: ActionState::parse(&state).ok_or_else(|| unreadable("state"))?,
                attempt: u32::try_from(attempt).map_err(|_| unreadable("attempt"))?,
                detail,
            })
        })
        .collect()
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
    idempotency_key: Option<String>,
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
        idempotency_key: row.get(16)?,
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
        idempotency_key: stored.idempotency_key,
        decision,
        attempts: u32::try_from(stored.attempts).map_err(|_| unreadable("attempts"))?,
        created_at: parse_time(&stored.created_at)?,
        updated_at: parse_time(&stored.updated_at)?,
        result: stored.result,
        error: stored.error,
        history: Vec::new(),
    })
}

/// The ordering position of the action a page cursor names.
fn cursor_position(connection: &Connection, cursor: &str) -> Result<(String, i64), StoreError> {
    let rowid: i64 = cursor.parse().map_err(|_| StoreError::InvalidCursor)?;
    connection
        .query_row(
            "SELECT created_at FROM actions WHERE rowid = ?1",
            [rowid],
            |row| row.get(0),
        )
        .optional()?
        .map(|created_at| (created_at, rowid))
        .ok_or(StoreError::InvalidCursor)
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

/// Moves an action from `from` to `state` at a person's request, and
/// records `event` with the note.
fn resolve(
    transaction: &Transaction<'_>,
    (run, name): (&str, &str),
    from: ActionState,
    (state, event): (ActionState, ActionEvent),
    note: Option<&str>,
) -> Result<(), StoreError> {
    transaction.execute(
        "UPDATE actions SET state = ?4, updated_at = ?5 WHERE run_id = ?1 AND name = ?2 AND state = ?3",
        params![
            run,
            name,
            from.as_str(),
            state.as_str(),
            format_time(OffsetDateTime::now_utc())?
        ],
    )?;
    log(transaction, run, name, event, note)
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
            log(&transaction, &run, &name, ActionEvent::Claimed, None)?;
            let claimed = one(&transaction, &run, &name)?;
            transaction.commit()?;
            Ok(Some(claimed))
        })
        .await
    }

    /// Records how a claimed action's dispatch ended, or how a backend
    /// restart left it. Only an `executing` action changes.
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
            let transaction = connection.transaction()?;
            let changed = transaction.execute(
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
            let event = delivery_event(delivery.state);
            log(&transaction, &run, &name, event, delivery.error.as_deref())?;
            let finished = one(&transaction, &run, &name)?;
            transaction.commit()?;
            Ok(finished)
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
            log(&transaction, &run, &name, verdict_event(verdict), request.note.as_deref())?;
            let decided = one(&transaction, &run, &name)?;
            transaction.commit()?;
            Ok(decided)
        })
        .await
    }

    /// One action of a run, with its history.
    ///
    /// # Errors
    ///
    /// Fails with [`StoreError::NotFound`] without such an action.
    pub async fn action(&self, run: &RunId, name: &str) -> Result<ActionRecord, StoreError> {
        let (run, name) = (run.as_str().to_owned(), name.to_owned());
        self.with(move |connection| {
            let mut record = one(connection, &run, &name)?;
            record.history = history(connection, &run, &name)?;
            Ok(record)
        })
        .await
    }

    /// Queues an action whose delivery is unknown or failed again, at a
    /// person's request. `digest` must be the recorded arguments' digest:
    /// the same payload is sent again.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] without such an action,
    /// [`StoreError::ActionNotResolvable`] in another state,
    /// [`StoreError::DigestMismatch`] for another digest.
    pub async fn redispatch_action(
        &self,
        run: &RunId,
        name: &str,
        request: action::DecisionRequest,
    ) -> Result<ActionRecord, StoreError> {
        let (run, name) = (run.as_str().to_owned(), name.to_owned());
        self.with(move |connection| {
            let transaction = connection.transaction()?;
            let current = one(&transaction, &run, &name)?;
            if !matches!(current.state, ActionState::Unknown | ActionState::Error) {
                return Err(StoreError::ActionNotResolvable(
                    current.state,
                    "only unknown or failed deliveries can be redispatched",
                ));
            }
            if current.digest != request.digest {
                return Err(StoreError::DigestMismatch);
            }
            let note = request.note.as_deref();
            let after = (ActionState::Queued, ActionEvent::Redispatched);
            resolve(&transaction, (&run, &name), current.state, after, note)?;
            let queued = one(&transaction, &run, &name)?;
            transaction.commit()?;
            Ok(queued)
        })
        .await
    }

    /// Records what a person found an unknown delivery to be: `completed`
    /// or `error`. Nothing is sent.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] without such an action,
    /// [`StoreError::ActionNotResolvable`] when it is not unknown.
    pub async fn reconcile_action(
        &self,
        run: &RunId,
        name: &str,
        request: ReconcileRequest,
    ) -> Result<ActionRecord, StoreError> {
        let (run, name) = (run.as_str().to_owned(), name.to_owned());
        self.with(move |connection| {
            let transaction = connection.transaction()?;
            let current = one(&transaction, &run, &name)?;
            if current.state != ActionState::Unknown {
                return Err(StoreError::ActionNotResolvable(
                    current.state,
                    "only unknown deliveries can be reconciled",
                ));
            }
            let note = request.note.as_deref();
            let after = (request.state, ActionEvent::Reconciled);
            resolve(&transaction, (&run, &name), current.state, after, note)?;
            let reconciled = one(&transaction, &run, &name)?;
            transaction.commit()?;
            Ok(reconciled)
        })
        .await
    }

    /// Actions in `state` across runs, oldest first, at most `limit`, and
    /// the cursor of the next page when there is one. `cursor` continues a
    /// previous page; the order (creation time, then insertion) is stable
    /// while actions change state.
    ///
    /// # Errors
    ///
    /// Fails on a cursor this store did not issue or a database error.
    pub async fn actions_in_state(
        &self,
        state: ActionState,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<(Vec<ActionRecord>, Option<String>), StoreError> {
        self.with(move |connection| {
            let after = cursor
                .map(|cursor| cursor_position(connection, &cursor))
                .transpose()?;
            let (created_at, rowid) = after.unwrap_or_default();
            let sql = format!(
                "SELECT {COLUMNS}, rowid FROM actions WHERE state = ?1 \
                 AND (created_at, rowid) > (?2, ?3) ORDER BY created_at, rowid LIMIT ?4"
            );
            let mut statement = connection.prepare(&sql)?;
            let rows = statement
                .query_map(
                    params![state.as_str(), created_at, rowid, limit + 1],
                    |row| Ok((read(row)?, row.get::<_, i64>(17)?)),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let more = rows.len() > limit as usize;
            let mut items = Vec::with_capacity(rows.len());
            let mut last = None;
            for (stored, rowid) in rows.into_iter().take(limit as usize) {
                items.push(record(stored)?);
                last = Some(rowid);
            }
            Ok((items, last.filter(|_| more).map(|rowid| rowid.to_string())))
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
