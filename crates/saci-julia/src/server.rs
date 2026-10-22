//! The HTTP judge service.
//!
//! - `POST /v1/evaluate`: an evaluation in the TypeSafe System One format.
//! - `GET /health`: `{"status": "ok"}`.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::engine::{Engine, EvaluateError};

/// The evaluation path.
pub const EVALUATE_PATH: &str = "/v1/evaluate";
/// The health path; it needs no token.
pub const HEALTH_PATH: &str = "/health";
/// Largest request body. Step evidence can be large.
const BODY_LIMIT: usize = 16 * 1024 * 1024;

/// The HTTP application.
pub fn router(engine: Arc<Engine>, token: Option<String>) -> Router {
    let token = Arc::new(token);
    Router::new()
        .route(EVALUATE_PATH, post(evaluate))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let token = token.clone();
            async move { check(token.as_deref(), request, next).await }
        }))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .route(HEALTH_PATH, get(|| async { Json(json!({"status": "ok"})) }))
        .with_state(engine)
}

async fn evaluate(State(engine): State<Arc<Engine>>, Json(request): Json<Value>) -> Response {
    let started = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(move || engine.evaluate(&request)).await;
    match result {
        Ok(Ok(answer)) => {
            tracing::debug!(elapsed_ms = started.elapsed().as_millis(), "evaluated");
            Json(answer).into_response()
        }
        Ok(Err(EvaluateError::Invalid(error))) => {
            tracing::info!(%error, "rejected an evaluation");
            failure(StatusCode::BAD_REQUEST, &error)
        }
        Ok(Err(EvaluateError::Model(error))) => {
            tracing::warn!(%error, "the model failed");
            failure(StatusCode::INTERNAL_SERVER_ERROR, &error)
        }
        Err(error) => failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("evaluation stopped: {error}"),
        ),
    }
}

fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": {"message": message}}))).into_response()
}

async fn check(token: Option<&str>, request: Request, next: Next) -> Response {
    if let Some(token) = token {
        let sent = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or_default();
        if !same(sent.as_bytes(), token.as_bytes()) {
            return failure(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
        }
    }
    next.run(request).await
}

/// Compares in time that does not depend on where the inputs differ.
fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (l, r)| difference | (l ^ r))
            == 0
}

/// Serves until `shutdown` is cancelled.
///
/// # Errors
///
/// Fails when the server fails.
pub async fn serve(
    engine: Arc<Engine>,
    token: Option<String>,
    listener: TcpListener,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let app = router(engine, token);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_exactly() {
        assert!(same(b"secret", b"secret"));
        assert!(!same(b"secret", b"secreT"));
        assert!(!same(b"secret", b"secret2"));
        assert!(!same(b"", b"x"));
    }
}
