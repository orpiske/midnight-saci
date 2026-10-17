//! Durable run records in SQLite.
//!
//! A run's redacted specification is written once and never changed. Its
//! report is folded from worker events with [`RunReport::apply`], inside the
//! same transaction that stores the events, so the report, the event log and
//! the acknowledgment always agree. Every database call runs on a blocking
//! thread, off the async runtime.

mod schema;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use saci_core::api::{Page, RunSummary};
use saci_core::event::{EventKind, RunEvent};
use saci_core::ids::RunId;
use saci_core::redaction::Redactor;
use saci_core::run::{ContractError, RunReport};
use saci_core::spec::RunSpec;
use saci_core::status::Lifecycle;
use serde::de::DeserializeOwned;
use time::OffsetDateTime;

pub use schema::CURRENT as SCHEMA_VERSION;

/// Database file name inside the data directory.
pub const DATABASE_FILE: &str = "saci.db";

/// Store failures.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// No such run.
    #[error("run not found")]
    NotFound,
    /// The idempotency key was used for a different request.
    #[error("the idempotency key was already used for a different request")]
    IdempotencyConflict,
    /// The events do not fit the run's record.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// The database failed or holds unreadable data.
    #[error("database error: {0}")]
    Database(String),
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Database(format!("stored record is unreadable: {error}"))
    }
}

/// Everything stored about one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    /// The immutable, redacted specification.
    pub spec: RunSpec,
    /// The current report.
    pub report: RunReport,
    /// When cancellation was requested.
    pub cancel_requested_at: Option<OffsetDateTime>,
    /// Hash of the worker's run-scoped token, once issued.
    pub worker_token_hash: Option<String>,
}

impl RunRecord {
    /// The client-facing summary.
    #[must_use]
    pub fn summary(&self) -> RunSummary {
        let report = &self.report;
        let current_unit = report
            .units
            .iter()
            .find(|unit| unit.attempts.last().is_some_and(|a| a.outcome.is_none()))
            .map(|unit| unit.identity.key.clone());
        RunSummary {
            id: report.header.run_id.clone(),
            plan_title: report.header.plan_title.clone(),
            environment: report.header.environment.clone(),
            policy: report.header.policy.clone(),
            lifecycle: report.lifecycle,
            outcome: report.outcome,
            reason: report.reason.clone(),
            current_unit,
            counts: report.counts,
            total_units: u32::try_from(report.units.len()).unwrap_or(u32::MAX),
            last_event_seq: report.last_seq,
            created_at: report.header.created_at,
            finished_at: report.finished_at,
            cancel_requested_at: self.cancel_requested_at,
        }
    }
}

/// A run to create.
#[derive(Debug, Clone)]
pub struct NewRun {
    /// The redacted specification.
    pub spec: RunSpec,
    /// The initial report, listing every unit.
    pub report: RunReport,
    /// Client idempotency key and a hash of the request it came with.
    pub idempotency: Option<(String, String)>,
}

/// Result of [`Store::create_run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Created {
    /// A new run.
    New(Box<RunRecord>),
    /// The run created earlier with the same idempotency key and request.
    Existing(Box<RunRecord>),
}

/// Handle to the database; cheap to clone.
#[derive(Debug, Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl Store {
    /// Opens or creates the database in `data_dir`, creating the directory
    /// and applying migrations.
    ///
    /// # Errors
    ///
    /// Fails when the directory or database cannot be opened or migrated.
    pub async fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let path = data_dir.join(DATABASE_FILE);
        let dir = data_dir.to_path_buf();
        let target = path.clone();
        let connection = blocking(move || {
            std::fs::create_dir_all(&dir).map_err(|e| {
                StoreError::Database(format!("cannot create {}: {e}", dir.display()))
            })?;
            let mut connection = Connection::open(&target)?;
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "foreign_keys", true)?;
            connection.busy_timeout(std::time::Duration::from_secs(5))?;
            schema::migrate(&mut connection).map_err(StoreError::Database)?;
            Ok(connection)
        })
        .await?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            path,
        })
    }

    /// Location of the database file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    async fn with<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, StoreError> {
        let connection = self.connection.clone();
        blocking(move || {
            let mut connection = connection.lock().unwrap_or_else(PoisonError::into_inner);
            work(&mut connection)
        })
        .await
    }

    /// Stores a new run, or returns the existing one for a repeated
    /// idempotent request.
    ///
    /// # Errors
    ///
    /// Fails on a reused idempotency key with a different request, or a
    /// database error.
    pub async fn create_run(&self, run: NewRun) -> Result<Created, StoreError> {
        self.with(move |connection| {
            let transaction = connection.transaction()?;
            if let Some((key, hash)) = &run.idempotency {
                let existing: Option<(String, Option<String>)> = transaction
                    .query_row(
                        "SELECT id, request_hash FROM runs WHERE idempotency_key = ?1",
                        [key],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                if let Some((id, stored)) = existing {
                    if stored.as_deref() != Some(hash.as_str()) {
                        return Err(StoreError::IdempotencyConflict);
                    }
                    return Ok(Created::Existing(Box::new(load(&transaction, &id)?)));
                }
            }
            insert(&transaction, &run)?;
            transaction.commit()?;
            let record = RunRecord {
                spec: run.spec,
                report: run.report,
                cancel_requested_at: None,
                worker_token_hash: None,
            };
            Ok(Created::New(Box::new(record)))
        })
        .await
    }

    /// Reads one run.
    ///
    /// # Errors
    ///
    /// Fails with [`StoreError::NotFound`] or a database error.
    pub async fn run(&self, id: &RunId) -> Result<RunRecord, StoreError> {
        let id = id.as_str().to_owned();
        self.with(move |connection| load(connection, &id)).await
    }

    /// Lists runs, newest first. `cursor` continues a previous page.
    ///
    /// # Errors
    ///
    /// Fails on an invalid cursor or a database error.
    pub async fn list(
        &self,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<Page<RunRecord>, StoreError> {
        self.with(move |connection| list(connection, cursor.as_deref(), limit))
            .await
    }

    /// Runs that have not finished, for reconciliation after a restart.
    ///
    /// # Errors
    ///
    /// Fails on a database error.
    pub async fn unfinished(&self) -> Result<Vec<RunRecord>, StoreError> {
        self.with(|connection| {
            let ids = column(
                connection,
                "SELECT id FROM runs WHERE finished = 0 ORDER BY rowid",
            )?;
            ids.iter().map(|id| load(connection, id)).collect()
        })
        .await
    }

    /// Persists consecutive worker events and folds them into the report,
    /// atomically. Events at or below the acknowledged sequence are ignored.
    /// Observations are redacted with `redactor` before they are stored.
    /// Returns the highest persisted sequence number.
    ///
    /// # Errors
    ///
    /// Fails, storing nothing, when any new event does not fit the run (a
    /// gap, another run, a finished run, an invalid unit event).
    pub async fn append_events(
        &self,
        id: &RunId,
        events: Vec<RunEvent>,
        redactor: Redactor,
    ) -> Result<u64, StoreError> {
        let id = id.as_str().to_owned();
        self.with(move |connection| {
            let transaction = connection.transaction()?;
            let mut record = load(&transaction, &id)?;
            for mut event in events {
                if event.seq <= record.report.last_seq {
                    continue;
                }
                if let EventKind::Observation { evidence } = &mut event.kind {
                    redactor.redact_evidence(&mut evidence.record);
                }
                record.report.apply(&event)?;
                store_event(&transaction, &id, &event)?;
            }
            save_report(&transaction, &id, &record.report)?;
            transaction.commit()?;
            Ok(record.report.last_seq)
        })
        .await
    }

    /// Events after `after`, at most `limit`, and whether the run finished.
    ///
    /// # Errors
    ///
    /// Fails with [`StoreError::NotFound`] or a database error.
    pub async fn events(
        &self,
        id: &RunId,
        after: u64,
        limit: u32,
    ) -> Result<(Vec<RunEvent>, bool), StoreError> {
        let id = id.as_str().to_owned();
        self.with(move |connection| {
            let finished: bool = connection
                .query_row("SELECT finished FROM runs WHERE id = ?1", [&id], |row| {
                    row.get(0)
                })
                .optional()?
                .ok_or(StoreError::NotFound)?;
            let mut statement = connection.prepare(
                "SELECT body FROM events WHERE run_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
            )?;
            let bodies = statement
                .query_map(params![id, int(after), limit], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let events = bodies
                .iter()
                .map(|body| parse(body))
                .collect::<Result<_, _>>()?;
            Ok((events, finished))
        })
        .await
    }

    /// Changes the report outside the event stream (backend transitions such
    /// as starting or finishing a run whose worker never reported). The
    /// closure sees the current report; a finished report cannot be changed.
    ///
    /// # Errors
    ///
    /// Fails when the run is finished, missing, or the closure fails.
    pub async fn update_report(
        &self,
        id: &RunId,
        change: impl FnOnce(&mut RunReport) -> Result<(), ContractError> + Send + 'static,
    ) -> Result<RunRecord, StoreError> {
        let id = id.as_str().to_owned();
        self.with(move |connection| {
            let transaction = connection.transaction()?;
            let mut record = load(&transaction, &id)?;
            if record.report.lifecycle == Lifecycle::Finished {
                return Err(ContractError::Finished.into());
            }
            change(&mut record.report)?;
            save_report(&transaction, &id, &record.report)?;
            transaction.commit()?;
            Ok(record)
        })
        .await
    }

    /// Records a cancellation request once; repeating it keeps the first
    /// time. Returns the updated record.
    ///
    /// # Errors
    ///
    /// Fails when the run is missing or a database error occurs.
    pub async fn request_cancel(
        &self,
        id: &RunId,
        at: OffsetDateTime,
    ) -> Result<RunRecord, StoreError> {
        let id = id.as_str().to_owned();
        self.with(move |connection| {
            let at = format_time(at)?;
            connection.execute(
                "UPDATE runs SET cancel_requested_at = COALESCE(cancel_requested_at, ?2) WHERE id = ?1 AND finished = 0",
                params![id, at],
            )?;
            load(connection, &id)
        })
        .await
    }

    /// Stores the hash of a newly issued worker token.
    ///
    /// # Errors
    ///
    /// Fails when the run is missing or a database error occurs.
    pub async fn set_worker_token_hash(&self, id: &RunId, hash: String) -> Result<(), StoreError> {
        let id = id.as_str().to_owned();
        self.with(move |connection| {
            let changed = connection.execute(
                "UPDATE runs SET worker_token_hash = ?2 WHERE id = ?1",
                params![id, hash],
            )?;
            if changed == 0 {
                Err(StoreError::NotFound)
            } else {
                Ok(())
            }
        })
        .await
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, StoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| StoreError::Database(format!("database task failed: {error}")))?
}

fn insert(transaction: &Transaction<'_>, run: &NewRun) -> Result<(), StoreError> {
    let (key, hash) = run.idempotency.clone().unzip();
    transaction.execute(
        "INSERT INTO runs (id, created_at, idempotency_key, request_hash, spec, report, finished, last_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            run.spec.run_id.as_str(),
            format_time(run.spec.created_at)?,
            key,
            hash,
            serde_json::to_string(&run.spec)?,
            serde_json::to_string(&run.report)?,
            run.report.lifecycle == Lifecycle::Finished,
            int(run.report.last_seq),
        ],
    )?;
    Ok(())
}

fn store_event(
    transaction: &Transaction<'_>,
    id: &str,
    event: &RunEvent,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO events (run_id, seq, body) VALUES (?1, ?2, ?3)",
        params![id, int(event.seq), serde_json::to_string(event)?],
    )?;
    if let EventKind::Observation { evidence } = &event.kind {
        transaction.execute(
            "INSERT INTO evidence (run_id, id, body) VALUES (?1, ?2, ?3)",
            params![
                id,
                evidence.id.to_string(),
                serde_json::to_string(evidence)?
            ],
        )?;
    }
    Ok(())
}

fn save_report(
    transaction: &Transaction<'_>,
    id: &str,
    report: &RunReport,
) -> Result<(), StoreError> {
    transaction.execute(
        "UPDATE runs SET report = ?2, finished = ?3, last_seq = ?4 WHERE id = ?1",
        params![
            id,
            serde_json::to_string(report)?,
            report.lifecycle == Lifecycle::Finished,
            int(report.last_seq)
        ],
    )?;
    Ok(())
}

/// `spec`, `report`, `cancel_requested_at`, `worker_token_hash`.
type StoredRun = (String, String, Option<String>, Option<String>);

fn load(connection: &Connection, id: &str) -> Result<RunRecord, StoreError> {
    let row: Option<StoredRun> = connection
        .query_row(
            "SELECT spec, report, cancel_requested_at, worker_token_hash FROM runs WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let (spec, report, cancel, token) = row.ok_or(StoreError::NotFound)?;
    Ok(RunRecord {
        spec: parse(&spec)?,
        report: parse(&report)?,
        cancel_requested_at: cancel.as_deref().map(parse_time).transpose()?,
        worker_token_hash: token,
    })
}

fn list(
    connection: &Connection,
    cursor: Option<&str>,
    limit: u32,
) -> Result<Page<RunRecord>, StoreError> {
    let before = match cursor {
        Some(cursor) => cursor
            .parse::<i64>()
            .map_err(|_| StoreError::Database("invalid cursor".into()))?,
        None => i64::MAX,
    };
    let limit = limit.clamp(1, 200);
    let mut statement = connection
        .prepare("SELECT rowid, id FROM runs WHERE rowid < ?1 ORDER BY rowid DESC LIMIT ?2")?;
    let rows = statement
        .query_map(params![before, limit + 1], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let more = rows.len() > limit as usize;
    let page: Vec<_> = rows.into_iter().take(limit as usize).collect();
    let next_cursor = page
        .last()
        .filter(|_| more)
        .map(|(rowid, _)| rowid.to_string());
    let items = page
        .iter()
        .map(|(_, id)| load(connection, id))
        .collect::<Result<_, _>>()?;
    Ok(Page { items, next_cursor })
}

fn column(connection: &Connection, sql: &str) -> Result<Vec<String>, StoreError> {
    let mut statement = connection.prepare(sql)?;
    let values = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(values)
}

/// SQLite integers are signed; sequence numbers never get near the limit.
fn int(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    Ok(serde_json::from_str(text)?)
}

fn format_time(at: OffsetDateTime) -> Result<String, StoreError> {
    at.format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| StoreError::Database(error.to_string()))
}

fn parse_time(text: &str) -> Result<OffsetDateTime, StoreError> {
    OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
        .map_err(|error| StoreError::Database(error.to_string()))
}

#[cfg(test)]
mod tests;
