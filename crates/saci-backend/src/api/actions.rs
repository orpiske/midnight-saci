//! Client routes for outcome actions (`docs/actions.md`): listing them,
//! and approving or rejecting the ones that wait for a person.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use saci_core::action::{ActionList, ActionRecord, ActionState, DecisionRequest, Verdict};
use saci_core::api::ErrorCode;
use serde::Deserialize;

use super::runs::run_id;
use super::{Backend, Failure};
use crate::store::StoreError;

/// A run's actions and their delivery state.
pub async fn list(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Result<Json<ActionList>, Failure> {
    let items = backend.store.actions(&run_id(&id)?).await?;
    Ok(Json(ActionList { items }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateQuery {
    state: Option<String>,
    limit: Option<u32>,
}

/// Actions across runs in one state, oldest first: by default the ones
/// waiting for approval.
pub async fn by_state(
    State(backend): State<Backend>,
    params: Result<Query<StateQuery>, QueryRejection>,
) -> Result<Json<ActionList>, Failure> {
    let Query(params) = params.map_err(|rejection| Failure::bad_request(rejection.body_text()))?;
    let state = match params.state.as_deref() {
        None => ActionState::PendingApproval,
        Some(text) => ActionState::parse(text)
            .ok_or_else(|| Failure::bad_request(format!("unknown action state `{text}`")))?,
    };
    let limit = params.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(Failure::bad_request("limit must be between 1 and 200"));
    }
    let items = backend.store.actions_in_state(state, limit).await?;
    Ok(Json(ActionList { items }))
}

/// Approves the recorded arguments named by their digest; the action is
/// then dispatched.
pub async fn approve(
    State(backend): State<Backend>,
    Path((id, name)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<ActionRecord>, Failure> {
    decide(&backend, &id, &name, Verdict::Approved, &body).await
}

/// Rejects the action; it is never dispatched.
pub async fn reject(
    State(backend): State<Backend>,
    Path((id, name)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<ActionRecord>, Failure> {
    decide(&backend, &id, &name, Verdict::Rejected, &body).await
}

async fn decide(
    backend: &Backend,
    id: &str,
    name: &str,
    verdict: Verdict,
    body: &[u8],
) -> Result<Json<ActionRecord>, Failure> {
    let run = run_id(id)?;
    let request: DecisionRequest = serde_json::from_slice(body)
        .map_err(|error| Failure::bad_request(format!("invalid decision: {error}")))?;
    let decided = backend
        .store
        .decide_action(&run, name, verdict, request)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => Failure::new(ErrorCode::NotFound, "action not found"),
            other => other.into(),
        })?;
    tracing::info!(%run, action = %name, verdict = verdict.as_str(), "action decided");
    backend.notify();
    Ok(Json(decided))
}
