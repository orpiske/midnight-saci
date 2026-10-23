//! Client routes for investigations: create one for a failed unit, list
//! all of them or a run's, and read one. An investigation's events,
//! evidence and cancellation use the routes of the run that executes it.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::header::LOCATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use saci_core::api::{API_BASE, ErrorCode, Page};
use saci_core::investigation::{Investigation, InvestigationList, InvestigationRequest};

use super::runs::{ListQuery, idempotency_key, limit, query, run_id};
use super::{Backend, Failure, investigation};
use crate::store::{Created, NewRun, RunRecord, StoreError};

type Reply<T> = Result<T, Failure>;

fn not_found() -> Failure {
    Failure::new(ErrorCode::NotFound, "investigation not found")
}

/// The investigation that `record` executes, if it executes one.
fn view(record: &RunRecord) -> Option<Investigation> {
    let spec = record.spec.investigation.as_deref()?;
    Some(Investigation::new(spec, &record.report))
}

pub async fn create(
    State(backend): State<Backend>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply<Response> {
    let key = idempotency_key(&headers)?;
    let request: InvestigationRequest = serde_json::from_slice(&body)
        .map_err(|error| Failure::bad_request(format!("invalid investigation request: {error}")))?;
    let run = Box::pin(investigation::prepare(&backend, &request, key)).await?;
    let (created, investigation) = Box::pin(store(&backend, run)).await?;
    if !created {
        return Ok((StatusCode::OK, Json(investigation)).into_response());
    }
    let (id, parent) = (&investigation.id, &investigation.parent_run);
    tracing::info!(investigation = %id, run = %parent, "investigation accepted");
    backend.supervisor.submitted(id.clone());
    let location = format!("{API_BASE}/investigations/{id}");
    Ok((
        StatusCode::ACCEPTED,
        [(LOCATION, location)],
        Json(investigation),
    )
        .into_response())
}

/// Stores `run`; returns whether it is new, and the investigation.
async fn store(backend: &Backend, run: Box<NewRun>) -> Reply<(bool, Box<Investigation>)> {
    let (created, record) = match Box::pin(backend.store.create_run(*run)).await? {
        Created::New(record) => (true, record),
        Created::Existing(record) => (false, record),
    };
    let investigation = view(&record).ok_or_else(|| {
        Failure::new(
            ErrorCode::IdempotencyConflict,
            "the idempotency key was already used for a run",
        )
    })?;
    Ok((created, Box::new(investigation)))
}

pub async fn show(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Reply<Json<Investigation>> {
    let id = run_id(&id).map_err(|_| not_found())?;
    let record = backend.store.run(&id).await.map_err(|error| match error {
        StoreError::NotFound => not_found(),
        other => other.into(),
    })?;
    view(&record).map(Json).ok_or_else(not_found)
}

pub async fn list(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Reply<Json<InvestigationList>> {
    let id = run_id(&id)?;
    backend.store.run(&id).await?;
    let records = backend.store.investigations(&id).await?;
    let items = records.iter().filter_map(view).collect();
    Ok(Json(InvestigationList { items }))
}

pub async fn list_all(
    State(backend): State<Backend>,
    params: Result<Query<ListQuery>, QueryRejection>,
) -> Reply<Json<Page<Investigation>>> {
    let params = query(params)?;
    let page = backend
        .store
        .list_investigations(params.cursor, limit(params.limit)?)
        .await?;
    Ok(Json(Page {
        items: page.items.iter().filter_map(view).collect(),
        next_cursor: page.next_cursor,
    }))
}
