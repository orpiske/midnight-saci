# Operating the backend

## Data directory

The backend keeps everything durable in `backend.data_dir` (default
`saci-data`, relative to its working directory), created on start:

| File | Contents |
|------|----------|
| `saci.db` (with `saci.db-wal`, `saci.db-shm`) | SQLite database: runs, their event logs, evidence and reports. |

Each run stores:

- its **specification**, written once at submission and never changed: the
  plan bundle, environment (parameters and secret *references*), policy,
  limits and LLM settings. No secret value is stored.
- its **event log**, in sequence order. A worker batch is stored in one
  transaction together with the updated report, and only then acknowledged;
  resent events at or below the acknowledged sequence are ignored.
- its **report**, folded from the events. Once the run has finished, no
  event, transition or cancellation can change it.
- its **evidence**, inside observation events. Observations are redacted
  again with the run's known secret values before they are stored.
- the hash of its worker token, never the token.

Runs are not deleted automatically. To remove old runs, stop the backend and
delete or archive the data directory, or delete rows with the `sqlite3`
tool.

Back up by stopping the backend and copying the directory, or while it runs
with `sqlite3 saci-data/saci.db ".backup saci-backup.db"`.

The database carries a schema version. A backend upgrades older databases on
start and refuses to open one written by a newer Saci.

All database work runs on blocking threads, off the backend's async
runtime.
