//! Run-scoped worker routes: the assigned specification, event ingestion
//! with durable acknowledgment, and cancellation control.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use saci_core::api::{ErrorCode, EventAck, EventBatch, WorkerControl};
use saci_core::ids::RunId;
use saci_core::redaction::Redactor;
use saci_core::spec::{RunSpec, SecretRef};

use super::submit::sha256_hex;
use super::{Backend, Failure};

/// Issues a new worker token for `run`, storing only its hash. The launcher
/// passes the returned value to the worker.
///
/// # Errors
///
/// Fails when the run does not exist or the database fails.
pub async fn issue_token(backend: &Backend, run: &RunId) -> Result<String, Failure> {
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    backend
        .store
        .set_worker_token_hash(run, sha256_hex(token.as_bytes()))
        .await?;
    Ok(token)
}

fn unauthorized() -> Failure {
    Failure::new(
        ErrorCode::Unauthorized,
        "send the run-scoped worker token as a bearer token",
    )
}

/// Admits a request only with the token issued for the run in its path.
pub async fn authorize(
    State(backend): State<Backend>,
    Path(params): Path<Vec<(String, String)>>,
    request: Request,
    next: Next,
) -> Result<Response, Failure> {
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(unauthorized)?;
    let owner = backend
        .store
        .run_for_token_hash(&sha256_hex(token.as_bytes()))
        .await?
        .ok_or_else(unauthorized)?;
    let requested = params
        .iter()
        .find(|(name, _)| name == "run_id")
        .map(|(_, value)| value.as_str());
    if requested != Some(owner.as_str()) {
        return Err(Failure::new(
            ErrorCode::Forbidden,
            "this worker token belongs to another run",
        ));
    }
    Ok(next.run(request).await)
}

fn run_id(raw: &str) -> Result<RunId, Failure> {
    RunId::parse(raw).map_err(|_| Failure::run_not_found())
}

pub async fn specification(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Result<Json<RunSpec>, Failure> {
    let record = backend.store.run(&run_id(&id)?).await?;
    Ok(Json(record.spec))
}

/// Redacts the secret values the backend itself knows (environment
/// secrets read from its own variables and the LLM key), as a second line
/// of defense behind the worker's redaction.
/// Redacts the secrets the backend can resolve for `spec`: the target
/// secrets from its environment and the LLM key.
#[must_use]
pub fn backend_redactor(spec: &RunSpec, lookup: impl Fn(&str) -> Option<String>) -> Redactor {
    let mut redactor = Redactor::new();
    for (name, secret) in &spec.environment.secrets {
        if let SecretRef::Env { env } = secret
            && let Some(value) = lookup(env)
        {
            redactor.add(name, &value);
        }
    }
    if let Some(value) = spec.llm.api_key_env.as_deref().and_then(&lookup) {
        redactor.add("LLM_API_KEY", &value);
    }
    redactor
}

/// Persists a batch and acknowledges the highest stored sequence number.
pub async fn events(
    State(backend): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<EventAck>, Failure> {
    let id = run_id(&id)?;
    let batch: EventBatch = serde_json::from_slice(&body)
        .map_err(|error| Failure::bad_request(format!("invalid event batch: {error}")))?;
    let record = backend.store.run(&id).await?;
    let redactor = backend_redactor(&record.spec, |name| std::env::var(name).ok());
    let acknowledged = Box::pin(backend.store.append_events(&id, batch.events, redactor)).await;
    backend.notify();
    Ok(Json(EventAck {
        acknowledged_seq: acknowledged?,
    }))
}

pub async fn control(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Result<Json<WorkerControl>, Failure> {
    let record = backend.store.run(&run_id(&id)?).await?;
    Ok(Json(WorkerControl {
        cancel_requested: record.cancel_requested_at.is_some(),
    }))
}
