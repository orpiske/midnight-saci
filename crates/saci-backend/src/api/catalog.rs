//! Client reads that describe what a run may be submitted with, and the
//! plan a run executes (`docs/api.md#console-reads`).

use axum::Json;
use axum::extract::{Path, State};
use saci_core::api::ErrorCode;
use saci_core::view::{self, EnvironmentInfo, Items, PolicyInfo, RunPlan};

use super::runs::run_id;
use super::{Backend, Failure};

/// The configured environments, without parameters or secrets.
pub async fn environments(State(backend): State<Backend>) -> Json<Items<EnvironmentInfo>> {
    Json(Items {
        items: view::environments(&backend.config),
    })
}

/// The policies, including the implied `default`.
pub async fn policies(State(backend): State<Backend>) -> Json<Items<PolicyInfo>> {
    Json(Items {
        items: view::policies(&backend.config),
    })
}

/// The documents submitted with the run and its units in execution order.
pub async fn plan(
    State(backend): State<Backend>,
    Path(id): Path<String>,
) -> Result<Json<RunPlan>, Failure> {
    let record = backend.store.run(&run_id(&id)?).await?;
    let Some(bundle) = &record.spec.bundle else {
        return Err(Failure::new(ErrorCode::NotFound, "the run has no plan"));
    };
    let title = &record.report.header.plan_title;
    Ok(Json(view::run_plan(
        record.spec.run_id.clone(),
        title,
        bundle,
    )))
}
