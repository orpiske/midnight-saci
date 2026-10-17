//! Structured error responses (`docs/api.md#errors`).

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use saci_core::api::{ApiError, ErrorBody, ErrorCode};
use saci_core::run::ContractError;

use crate::store::StoreError;

/// An error answered with a stable code and the shared error body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    code: ErrorCode,
    message: String,
    details: Vec<String>,
}

impl Failure {
    /// A failure without details.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Vec::new(),
        }
    }

    /// Adds individual problems.
    #[must_use]
    pub fn with_details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }

    /// A malformed request.
    #[must_use]
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    /// No such run.
    #[must_use]
    pub fn run_not_found() -> Self {
        Self::new(ErrorCode::NotFound, "run not found")
    }

    /// The error code.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = ErrorBody {
            error: ApiError {
                code: self.code,
                message: self.message,
                details: self.details,
            },
        };
        (status, Json(body)).into_response()
    }
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound => Self::run_not_found(),
            StoreError::InvalidCursor => Self::bad_request("invalid cursor"),
            StoreError::ActionNotPending(_) => {
                Self::new(ErrorCode::ActionNotPending, error.to_string())
            }
            StoreError::DigestMismatch => Self::new(ErrorCode::DigestMismatch, error.to_string()),
            StoreError::IdempotencyConflict => {
                Self::new(ErrorCode::IdempotencyConflict, error.to_string())
            }
            StoreError::Contract(ContractError::Finished) => {
                Self::new(ErrorCode::RunFinished, "the run already finished")
            }
            StoreError::Contract(ContractError::OutOfOrder { expected, found }) => Self::new(
                ErrorCode::SequenceGap,
                format!("expected event {expected}, received {found}"),
            )
            .with_details(vec![format!("acknowledged_seq={}", expected - 1)]),
            StoreError::Contract(contract) => {
                Self::new(ErrorCode::ValidationFailed, contract.to_string())
            }
            StoreError::Database(message) => {
                tracing::error!(%message, "database failure");
                Self::new(
                    ErrorCode::Internal,
                    "the backend could not access its database",
                )
            }
        }
    }
}
