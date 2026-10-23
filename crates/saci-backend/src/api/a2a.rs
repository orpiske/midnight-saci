//! A minimal A2A server for investigations (`docs/investigations.md#a2a`):
//! the agent card and the JSON-RPC 2.0 methods `message/send`, `tasks/get`
//! and `tasks/cancel`. A task is an investigation: the task ID is the
//! investigation ID and the context ID is the ID of the investigated run.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::HOST;
use axum::routing::{get, post};
use axum::{Json, Router, middleware};
use saci_core::api::ErrorCode;
use saci_core::ids::RunId;
use saci_core::investigation::{Investigation, InvestigationOutcome, InvestigationRequest};
use saci_core::status::Lifecycle;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{Backend, ClientAuth, Failure, authorize, investigation, investigations};
use crate::store::StoreError;

/// Where the agent card is served.
pub const CARD_PATH: &str = "/.well-known/agent-card.json";
/// Where the JSON-RPC methods are served.
pub const RPC_PATH: &str = "/a2a";
/// The A2A protocol version of the card and the methods.
const PROTOCOL_VERSION: &str = "0.3.0";
/// The ID of the one skill.
const SKILL: &str = "investigate-failure";
/// Prefix of the idempotency key that a message ID becomes, so that a
/// message ID never matches an `Idempotency-Key` of the REST API.
const MESSAGE_KEY: &str = "a2a:";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const TASK_NOT_FOUND: i64 = -32001;
const TASK_NOT_CANCELABLE: i64 = -32002;

/// The agent card and the JSON-RPC endpoint. The endpoint needs the
/// client token; the card does not, so that clients can discover how to
/// authenticate.
pub fn routes(backend: Backend, auth: Arc<ClientAuth>) -> Router {
    let secured = *auth != ClientAuth::Open;
    Router::new()
        .route(
            RPC_PATH,
            post(rpc).route_layer(middleware::from_fn_with_state(auth, authorize)),
        )
        .route(
            CARD_PATH,
            get(move |headers: HeaderMap| async move { Json(card(&headers, secured)) }),
        )
        .with_state(backend)
}

/// The agent card, with the endpoint URL built from the request's host.
fn card(headers: &HeaderMap, secured: bool) -> Value {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let host = header(HOST.as_str()).unwrap_or("localhost");
    let scheme = header("x-forwarded-proto").unwrap_or("http");
    let mut card = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "name": "Saci investigator",
        "description": "Investigates a failed step of a finished Saci run. The agent reads \
            the failure and the source code at the version under test, finds the probable \
            cause in the code and files a bug.",
        "url": format!("{scheme}://{host}{RPC_PATH}"),
        "preferredTransport": "JSONRPC",
        "version": env!("CARGO_PKG_VERSION"),
        "capabilities": {"streaming": false, "pushNotifications": false},
        "defaultInputModes": ["application/json"],
        "defaultOutputModes": ["application/json", "text/plain"],
        "skills": [skill()],
    });
    if secured {
        card["securitySchemes"] = json!({"bearer": {"type": "http", "scheme": "bearer"}});
        card["security"] = json!([{"bearer": []}]);
    }
    card
}

fn skill() -> Value {
    json!({
        "id": SKILL,
        "name": "Investigate a failure",
        "description": "Send a message with one data part: `run_id` (a finished run), \
            optionally `unit` (the failed unit key; the first failed unit when omitted) and \
            `sources` (configured repositories, each with a `name` and an optional `ref`). \
            The completed task has the result as a data artifact and the summary as a text \
            artifact.",
        "tags": ["testing", "investigation", "root cause"],
        "inputModes": ["application/json"],
        "outputModes": ["application/json", "text/plain"],
        "examples": [r#"{"run_id": "run-0f3c9a2b7d4e4f1a9b8c7d6e5f4a3b2c", "unit": "1.2"}"#],
    })
}

/// A JSON-RPC error.
#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn task_not_found() -> Self {
        Self::new(TASK_NOT_FOUND, "task not found")
    }
}

impl From<Failure> for RpcError {
    fn from(failure: Failure) -> Self {
        let code = match failure.code() {
            ErrorCode::IdempotencyConflict => INVALID_PARAMS,
            _ => INTERNAL_ERROR,
        };
        Self::new(code, failure.message())
    }
}

impl From<StoreError> for RpcError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound => Self::task_not_found(),
            other => Failure::from(other).into(),
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Answers one JSON-RPC request. Every answer has HTTP status 200, also
/// an error.
async fn rpc(State(backend): State<Backend>, body: Bytes) -> Json<Value> {
    let (id, answer) = match envelope(&body) {
        Ok(envelope) => {
            let answer = Box::pin(call(&backend, &envelope.method, envelope.params)).await;
            (envelope.id, answer)
        }
        Err((id, error)) => (id, Err(error)),
    };
    Json(match answer {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": error.code, "message": error.message},
        }),
    })
}

/// The request, or the ID to answer with and the error.
fn envelope(body: &[u8]) -> Result<Envelope, (Value, RpcError)> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| (Value::Null, RpcError::new(PARSE_ERROR, error.to_string())))?;
    let id = match value.get("id") {
        Some(id @ (Value::String(_) | Value::Number(_))) => id.clone(),
        _ => Value::Null,
    };
    let invalid = |message: String| (id.clone(), RpcError::new(INVALID_REQUEST, message));
    let envelope: Envelope =
        serde_json::from_value(value).map_err(|error| invalid(error.to_string()))?;
    if envelope.jsonrpc != "2.0" {
        return Err(invalid("`jsonrpc` must be \"2.0\"".into()));
    }
    if id.is_null() {
        return Err(invalid("`id` must be a string or a number".into()));
    }
    Ok(envelope)
}

async fn call(backend: &Backend, method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
        "message/send" => Box::pin(send(backend, params)).await,
        "tasks/get" => get_task(backend, params).await,
        "tasks/cancel" => cancel_task(backend, params).await,
        other => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!(
                "method `{other}` is not supported; use message/send, tasks/get or tasks/cancel"
            ),
        )),
    }
}

fn params<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|error| RpcError::new(INVALID_PARAMS, format!("invalid params: {error}")))
}

#[derive(Deserialize)]
struct SendParams {
    message: Message,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Message {
    message_id: String,
    #[serde(default)]
    parts: Vec<Value>,
}

/// Creates an investigation, or returns the one that the same message or
/// an earlier request for the same unit created. The task is answered at
/// once, also when the client asks to block.
async fn send(backend: &Backend, raw: Value) -> Result<Value, RpcError> {
    let SendParams { message } = params(raw)?;
    if message.message_id.is_empty() {
        return Err(RpcError::new(
            INVALID_PARAMS,
            "`messageId` must not be empty",
        ));
    }
    let request = match request(&message.parts) {
        Ok(request) => request,
        Err(reason) => return Ok(rejected(None, &reason)),
    };
    let key = format!("{MESSAGE_KEY}{}", message.message_id);
    let run = match Box::pin(investigation::prepare(backend, &request, Some(key))).await {
        Ok(run) => run,
        Err(failure) if client_error(&failure) => {
            return Ok(rejected(Some(&request.run_id), failure.message()));
        }
        Err(failure) => return Err(failure.into()),
    };
    let (created, investigation) = Box::pin(investigations::store(backend, run)).await?;
    if created {
        let (id, parent) = (&investigation.id, &investigation.parent_run);
        tracing::info!(investigation = %id, run = %parent, "investigation accepted over A2A");
        backend.supervisor.submitted(id.clone());
    }
    Ok(task(&investigation))
}

/// Whether `failure` is about the request, so the task is rejected.
const fn client_error(failure: &Failure) -> bool {
    matches!(
        failure.code(),
        ErrorCode::NotFound | ErrorCode::ValidationFailed | ErrorCode::BadRequest
    )
}

/// The investigation request in the message's one data part.
fn request(parts: &[Value]) -> Result<InvestigationRequest, String> {
    let mut data = parts
        .iter()
        .filter(|part| part.get("kind").and_then(Value::as_str) == Some("data"));
    let (Some(part), None) = (data.next(), data.next()) else {
        return Err("the message must have exactly one data part".into());
    };
    let data = part.get("data").cloned().unwrap_or(Value::Null);
    serde_json::from_value(data).map_err(|error| format!("invalid investigation request: {error}"))
}

/// A task that was not created. It is not stored, so `tasks/get` does not
/// find it.
fn rejected(run: Option<&RunId>, reason: &str) -> Value {
    let id = format!("rejected-{}", uuid::Uuid::new_v4().simple());
    let context = run.map_or_else(|| id.clone(), ToString::to_string);
    json!({
        "kind": "task",
        "id": id,
        "contextId": context,
        "status": status("rejected", OffsetDateTime::now_utc(), Some((&id, reason))),
    })
}

#[derive(Deserialize)]
struct TaskParams {
    id: String,
}

/// The investigation with the task ID in `raw`.
async fn find(backend: &Backend, raw: Value) -> Result<Investigation, RpcError> {
    let TaskParams { id } = params(raw)?;
    let id = RunId::parse(&id).map_err(|_| RpcError::task_not_found())?;
    let record = backend.store.run(&id).await?;
    investigations::view(&record).ok_or_else(RpcError::task_not_found)
}

async fn get_task(backend: &Backend, raw: Value) -> Result<Value, RpcError> {
    Ok(task(&find(backend, raw).await?))
}

/// Requests cancellation, as `POST /runs/{id}/cancellation` does.
async fn cancel_task(backend: &Backend, raw: Value) -> Result<Value, RpcError> {
    let investigation = find(backend, raw).await?;
    let record = backend
        .store
        .request_cancel(&investigation.id, OffsetDateTime::now_utc())
        .await?;
    if record.report.lifecycle.is_terminal() {
        return Err(RpcError::new(
            TASK_NOT_CANCELABLE,
            "the investigation already finished",
        ));
    }
    backend
        .supervisor
        .cancel_requested(investigation.id.clone());
    backend.notify();
    let investigation = investigations::view(&record).ok_or_else(RpcError::task_not_found)?;
    Ok(task(&investigation))
}

/// The A2A task state of an investigation.
const fn state(investigation: &Investigation) -> &'static str {
    match (investigation.lifecycle, investigation.outcome) {
        (Lifecycle::Accepted | Lifecycle::Starting, _) => "submitted",
        (Lifecycle::Running | Lifecycle::Stopping, _) => "working",
        (_, Some(InvestigationOutcome::Concluded | InvestigationOutcome::Inconclusive)) => {
            "completed"
        }
        (_, Some(InvestigationOutcome::Cancelled)) => "canceled",
        (Lifecycle::Finished, _) => "failed",
    }
}

/// The task of an investigation, with the result and summary artifacts
/// once the agent concluded.
fn task(investigation: &Investigation) -> Value {
    let id = investigation.id.as_str();
    let at = investigation
        .finished_at
        .unwrap_or(investigation.created_at);
    let reason = investigation.reason.as_deref().map(|reason| (id, reason));
    let mut task = json!({
        "kind": "task",
        "id": id,
        "contextId": investigation.parent_run,
        "status": status(state(investigation), at, reason),
        "metadata": {"unit": investigation.unit},
    });
    if let Some(result) = &investigation.result {
        task["artifacts"] = json!([
            {
                "artifactId": "result",
                "name": "Investigation result",
                "parts": [{"kind": "data", "data": result}],
            },
            {
                "artifactId": "summary",
                "name": "Summary",
                "parts": [{"kind": "text", "text": result.summary}],
            },
        ]);
    }
    task
}

/// A task status; `reason` is the task ID and the text of an agent message.
fn status(state: &str, at: OffsetDateTime, reason: Option<(&str, &str)>) -> Value {
    let mut status = Map::new();
    status.insert("state".into(), state.into());
    if let Ok(at) = at.format(&Rfc3339) {
        status.insert("timestamp".into(), at.into());
    }
    if let Some((task, text)) = reason {
        status.insert(
            "message".into(),
            json!({
                "kind": "message",
                "role": "agent",
                "messageId": format!("{task}-status"),
                "taskId": task,
                "parts": [{"kind": "text", "text": text}],
            }),
        );
    }
    Value::Object(status)
}
