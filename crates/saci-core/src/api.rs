//! Request and response bodies of the REST API (`docs/openapi.yaml`).

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::event::RunEvent;
use crate::ids::{RunId, UnitKey};
use crate::plan::PlanBundle;
use crate::run::OutcomeCounts;
use crate::status::{Lifecycle, Outcome};
use crate::view::PlanDiagnostic;

/// Base path of every Saci route.
pub const API_BASE: &str = "/api/v1";

/// Largest worker event batch, in serialized bytes, the backend accepts.
/// Workers split batches to stay under it.
pub const MAX_EVENT_BATCH_BYTES: usize = 16 * 1024 * 1024;

/// Header carrying a client-chosen key that makes submission retry-safe.
pub const IDEMPOTENCY_KEY: &str = "Idempotency-Key";

/// `POST /api/v1/runs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitRun {
    /// The entry document and every procedure it invokes.
    pub bundle: PlanBundle,
    /// Configured environment name.
    pub environment: String,
    /// Configured policy name; `default` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

/// A run as listed and inspected by clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    /// Run identifier.
    pub id: RunId,
    /// Plan title.
    pub plan_title: String,
    /// Environment name.
    pub environment: String,
    /// Policy name.
    pub policy: String,
    /// Lifecycle state.
    pub lifecycle: Lifecycle,
    /// Summary outcome once finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Why the run ended this way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The unit currently executing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_unit: Option<UnitKey>,
    /// Per-outcome unit counts so far.
    pub counts: OutcomeCounts,
    /// Number of units in the run.
    pub total_units: u32,
    /// Sequence number of the latest event.
    pub last_event_seq: u64,
    /// When the run was accepted.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the run finished.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    /// When cancellation was requested, if it was.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub cancel_requested_at: Option<OffsetDateTime>,
}

/// A page of a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    /// Items of this page.
    pub items: Vec<T>,
    /// Cursor for the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// `GET /api/v1/runs/{run_id}/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPage {
    /// Events with `seq` greater than the requested `after`, in order.
    pub items: Vec<RunEvent>,
    /// Pass as `after` to continue.
    pub next_after: u64,
    /// Whether the run has finished; no further events will appear.
    pub finished: bool,
}

/// `POST /api/v1/worker/runs/{run_id}/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventBatch {
    /// Consecutive events; may begin with already-acknowledged events.
    pub events: Vec<RunEvent>,
}

/// Durable acknowledgment of worker events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventAck {
    /// Highest sequence number persisted for the run.
    pub acknowledged_seq: u64,
}

/// `GET /api/v1/worker/runs/{run_id}/control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerControl {
    /// Whether the worker must stop.
    pub cancel_requested: bool,
}

/// A structured error body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// The error.
    pub error: ApiError,
}

/// Stable error description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// Stable machine-readable code.
    pub code: ErrorCode,
    /// Human-readable explanation.
    pub message: String,
    /// Individual problems, for example plan diagnostics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
    /// Plan diagnostics by document and line, when the plan is invalid.
    /// `details` holds the same problems as text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PlanDiagnostic>,
}

/// Stable error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Missing or invalid credentials (401).
    Unauthorized,
    /// Credentials do not grant access to this resource (403).
    Forbidden,
    /// No such resource (404).
    NotFound,
    /// The request body or plan is invalid (422).
    ValidationFailed,
    /// An idempotency key was reused with a different request (409).
    IdempotencyConflict,
    /// The run already finished (409).
    RunFinished,
    /// Events skipped a sequence number (409).
    SequenceGap,
    /// The action is not awaiting approval (409).
    ActionNotPending,
    /// The decision names a digest other than the recorded arguments' (409).
    DigestMismatch,
    /// The action's delivery is not unknown or failed, so it cannot be
    /// redispatched or reconciled (409).
    ActionNotResolvable,
    /// The request is malformed (400).
    BadRequest,
    /// The backend failed (500).
    Internal,
}

impl ErrorCode {
    /// HTTP status code used for this error.
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::IdempotencyConflict
            | Self::RunFinished
            | Self::SequenceGap
            | Self::ActionNotPending
            | Self::DigestMismatch
            | Self::ActionNotResolvable => 409,
            Self::ValidationFailed => 422,
            Self::Internal => 500,
        }
    }
}
