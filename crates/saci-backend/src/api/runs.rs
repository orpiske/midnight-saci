//! Client routes: submission, listing, inspection, progress, reports,
//! evidence and cancellation.

use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::header::LOCATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use saci_core::api::{
    API_BASE, ErrorCode, EventPage, IDEMPOTENCY_KEY, Page, RunSummary, SubmitRun,
};
use saci_core::evidence::Evidence;
use saci_core::ids::{EvidenceId, RunId};
use saci_core::run::RunReport;
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::{Backend, Failure, submit};
use crate::store::Created;

/// Longest accepted `Idempotency-Key`.
const MAX_KEY_LEN: usize = 200;
/// Longest long-poll wait, in seconds.
const MAX_WAIT_SECS: u64 = 60;

type Reply<T> = Result<T, Failure>;

pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

pub(super) fn run_id(raw: &str) -> Reply<RunId> {
    RunId::parse(raw).map_err(|_| Failure::run_not_found())
}

pub(super) fn idempotency_key(headers: &HeaderMap) -> Reply<Option<String>> {
    let Some(value) = headers.get(IDEMPOTENCY_KEY) else {
        return Ok(None);
    };
    let key = value
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|key| !key.is_empty() && key.len() <= MAX_KEY_LEN)
        .ok_or_else(|| {
            Failure::bad_request(format!(
                "{IDEMPOTENCY_KEY} must be 1 to {MAX_KEY_LEN} visible ASCII characters"
            ))
        })?;
    Ok(Some(key.to_owned()))
}

pub async fn submit(
    State(backend): State<Backend>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply<Response> {
    let key = idempotency_key(&headers)?;
    let request: SubmitRun = serde_json::from_slice(&body)
        .map_err(|error| Failure::bad_request(format!("invalid submission: {error}")))?;
    let run = submit::prepare(&request, &backend.config, key)?;
    match Box::pin(backend.store.create_run(run)).await? {
        Created::New(record) => {
            let summary = record.summary();
            tracing::info!(run = %summary.id, "run accepted");
            backend.supervisor.submitted(summary.id.clone());
            let location = format!("{API_BASE}/runs/{}", summary.id);
            Ok((StatusCode::ACCEPTED, [(LOCATION, location)], Json(summary)).into_response())
        }
        Created::Existing(record) => Ok((StatusCode::OK, Json(record.summary())).into_response()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

pub(super) fn limit(value: Option<u32>) -> Reply<u32> {
    match value.unwrap_or(50) {
        limit @ 1..=200 => Ok(limit),
        _ => Err(Failure::bad_request("limit must be between 1 and 200")),
    }
}

pub(super) fn query<T>(query: Result<Query<T>, QueryRejection>) -> Reply<T> {
    query
        .map(|Query(query)| query)
        .map_err(|rejection| Failure::bad_request(rejection.body_text()))
}

pub async fn list(
    State(backend): State<Backend>,
    params: Result<Query<ListQuery>, QueryRejection>,
) -> Reply<Json<Page<RunSummary>>> {
    let params = query(params)?;
    let page = backend
        .store
        .list(params.cursor, limit(params.limit)?)
        .await?;
    Ok(Json(Page {
        items: page
            .items
            .iter()
            .map(crate::store::RunRecord::summary)
            .collect(),
        next_cursor: page.next_cursor,
    }))
}

pub async fn inspect(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Reply<Json<RunSummary>> {
    let record = backend.store.run(&run_id(&id)?).await?;
    Ok(Json(record.summary()))
}

pub async fn report(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Reply<Json<RunReport>> {
    let record = backend.store.run(&run_id(&id)?).await?;
    Ok(Json(record.report))
}

pub async fn evidence(
    State(backend): State<Backend>,
    Path((id, evidence)): Path<(String, String)>,
) -> Reply<Json<Evidence>> {
    let id = run_id(&id)?;
    let not_found = || Failure::new(ErrorCode::NotFound, "evidence not found");
    let evidence = EvidenceId::parse(&evidence).map_err(|_| not_found())?;
    backend
        .store
        .evidence(&id, &evidence)
        .await
        .map(Json)
        .map_err(|error| match error {
            crate::store::StoreError::NotFound => not_found(),
            other => other.into(),
        })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsQuery {
    #[serde(default)]
    after: u64,
    pub limit: Option<u32>,
    #[serde(default)]
    wait: u64,
}

/// Returns events after `after`. With `wait`, holds the request until there
/// is at least one, the run finishes, or the wait ends.
pub async fn events(
    State(backend): State<Backend>,
    Path(id): Path<String>,
    params: Result<Query<EventsQuery>, QueryRejection>,
) -> Reply<Json<EventPage>> {
    let id = run_id(&id)?;
    let params = query(params)?;
    let limit = limit(params.limit)?;
    if params.wait > MAX_WAIT_SECS {
        return Err(Failure::bad_request(
            "wait must be between 0 and 60 seconds",
        ));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(params.wait);
    let mut changes = backend.subscribe();
    loop {
        changes.borrow_and_update();
        let (items, finished) = backend.store.events(&id, params.after, limit).await?;
        let waited_out = tokio::time::Instant::now() >= deadline;
        if !items.is_empty() || finished || waited_out {
            let next_after = items.last().map_or(params.after, |event| event.seq);
            return Ok(Json(EventPage {
                items,
                next_after,
                finished,
            }));
        }
        // Either a change arrives or the deadline passes; both re-check.
        let _ = tokio::time::timeout_at(deadline, changes.changed()).await;
    }
}

/// Records a cancellation request and tells the supervisor.
pub async fn cancel(State(backend): State<Backend>, Path(id): Path<String>) -> Reply<Response> {
    let id = run_id(&id)?;
    let record = backend
        .store
        .request_cancel(&id, OffsetDateTime::now_utc())
        .await?;
    if record.report.lifecycle.is_terminal() {
        return Err(Failure::new(
            ErrorCode::RunFinished,
            "the run already finished",
        ));
    }
    backend.supervisor.cancel_requested(id);
    backend.notify();
    Ok((StatusCode::ACCEPTED, Json(record.summary())).into_response())
}
