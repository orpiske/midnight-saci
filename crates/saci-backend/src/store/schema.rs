//! Database schema migrations, applied in order and tracked with
//! `PRAGMA user_version`.

use rusqlite::Connection;

/// Each entry upgrades the schema by one version.
const MIGRATIONS: [&str; 6] = [
    r"
CREATE TABLE runs (
    id                  TEXT PRIMARY KEY,
    created_at          TEXT NOT NULL,
    idempotency_key     TEXT UNIQUE,
    request_hash        TEXT,
    spec                TEXT NOT NULL,
    report              TEXT NOT NULL,
    finished            INTEGER NOT NULL DEFAULT 0,
    last_seq            INTEGER NOT NULL DEFAULT 0,
    cancel_requested_at TEXT,
    worker_token_hash   TEXT
);
CREATE TABLE events (
    run_id TEXT NOT NULL REFERENCES runs (id),
    seq    INTEGER NOT NULL,
    body   TEXT NOT NULL,
    PRIMARY KEY (run_id, seq)
);
CREATE TABLE evidence (
    run_id TEXT NOT NULL REFERENCES runs (id),
    id     TEXT NOT NULL,
    body   TEXT NOT NULL,
    PRIMARY KEY (run_id, id)
);
CREATE INDEX runs_unfinished ON runs (finished);
",
    // Outcome actions (docs/actions.md): the definitions selected when the
    // run was accepted, and one row per triggered action.
    r"
ALTER TABLE runs ADD COLUMN actions TEXT NOT NULL DEFAULT '{}';
CREATE TABLE actions (
    run_id     TEXT NOT NULL REFERENCES runs (id),
    name       TEXT NOT NULL,
    server     TEXT NOT NULL,
    tool       TEXT NOT NULL,
    approval   TEXT NOT NULL,
    state      TEXT NOT NULL,
    arguments  TEXT NOT NULL,
    digest     TEXT NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    result     TEXT,
    error      TEXT,
    PRIMARY KEY (run_id, name)
);
CREATE INDEX actions_state ON actions (state);
",
    // Approval decisions on outcome actions.
    r"
ALTER TABLE actions ADD COLUMN decision TEXT;
ALTER TABLE actions ADD COLUMN decided_at TEXT;
ALTER TABLE actions ADD COLUMN decision_note TEXT;
",
    // Idempotency keys and the history of each action's delivery.
    r"
ALTER TABLE actions ADD COLUMN idempotency_key TEXT;
CREATE TABLE action_history (
    run_id  TEXT NOT NULL,
    name    TEXT NOT NULL,
    seq     INTEGER NOT NULL,
    at      TEXT NOT NULL,
    event   TEXT NOT NULL,
    state   TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    detail  TEXT,
    PRIMARY KEY (run_id, name, seq),
    FOREIGN KEY (run_id, name) REFERENCES actions (run_id, name)
);
",
    // Investigations (docs/plans/investigations.md): the investigated run
    // and unit of an investigation run.
    r"
ALTER TABLE runs ADD COLUMN parent_run TEXT;
ALTER TABLE runs ADD COLUMN parent_unit TEXT;
CREATE UNIQUE INDEX runs_investigated ON runs (parent_run, parent_unit)
    WHERE parent_run IS NOT NULL;
",
    // An investigation that ended in error or was cancelled can be
    // replaced (#126), so the index no longer keeps one for each unit.
    r"
DROP INDEX runs_investigated;
CREATE INDEX runs_investigated ON runs (parent_run, parent_unit)
    WHERE parent_run IS NOT NULL;
",
];

/// The schema version this build writes.
pub const CURRENT: u32 = 6;

const _: () = assert!(MIGRATIONS.len() == CURRENT as usize);

/// Brings the database to the current schema.
///
/// # Errors
///
/// Fails on a database error or when the database was written by a newer
/// Saci.
pub fn migrate(connection: &mut Connection) -> Result<(), String> {
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version > CURRENT {
        return Err(format!(
            "the database has schema version {version}, newer than this backend's {CURRENT}; upgrade Saci"
        ));
    }
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        transaction
            .execute_batch(migration)
            .map_err(|error| format!("migration {}: {error}", index + 1))?;
    }
    transaction
        .pragma_update(None, "user_version", CURRENT)
        .map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())
}
