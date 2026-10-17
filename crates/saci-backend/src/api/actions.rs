//! Client routes for outcome actions (`docs/actions.md`).

use axum::Json;
use axum::extract::{Path, State};
use saci_core::action::ActionList;

use super::runs::run_id;
use super::{Backend, Failure};

/// A run's actions and their delivery state.
pub async fn list(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Result<Json<ActionList>, Failure> {
    let items = backend.store.actions(&run_id(&id)?).await?;
    Ok(Json(ActionList { items }))
}
