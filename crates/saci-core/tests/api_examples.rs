//! The documented API examples match the Rust types, and the OpenAPI
//! description lists every implemented route.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use saci_core::api::{
    ErrorBody, ErrorCode, EventAck, EventBatch, EventPage, RunSummary, SubmitRun,
};
use serde::de::DeserializeOwned;

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

fn example<T: DeserializeOwned>(name: &str) -> T {
    let text = std::fs::read_to_string(docs().join("api-examples").join(name)).unwrap();
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name}: {error}"))
}

#[test]
fn examples_deserialize_into_api_types() {
    let submit: SubmitRun = example("submit-run.json");
    assert_eq!(submit.bundle.documents.len(), 2);
    let _: RunSummary = example("run-summary.json");
    let page: EventPage = example("event-page.json");
    assert_eq!(page.next_after, page.items.last().unwrap().seq);
    let batch: EventBatch = example("event-batch.json");
    let ack: EventAck = example("event-ack.json");
    assert_eq!(ack.acknowledged_seq, batch.events.last().unwrap().seq);
    let gap: ErrorBody = example("error-sequence-gap.json");
    assert_eq!(gap.error.code.status(), 409);
    let invalid: ErrorBody = example("error-validation.json");
    assert_eq!(invalid.error.code, ErrorCode::ValidationFailed);
}

#[test]
fn openapi_lists_every_route() {
    let text = std::fs::read_to_string(docs().join("openapi.yaml")).unwrap();
    let spec: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    let paths = spec["paths"].as_mapping().unwrap();
    for route in [
        "/api/v1/health",
        "/api/v1/runs",
        "/api/v1/runs/{run_id}",
        "/api/v1/runs/{run_id}/events",
        "/api/v1/runs/{run_id}/report",
        "/api/v1/runs/{run_id}/evidence/{evidence_id}",
        "/api/v1/runs/{run_id}/cancellation",
        "/api/v1/worker/runs/{run_id}/specification",
        "/api/v1/worker/runs/{run_id}/events",
        "/api/v1/worker/runs/{run_id}/control",
    ] {
        assert!(paths.contains_key(route), "missing {route}");
    }
    assert!(
        paths
            .keys()
            .all(|key| key.as_str().unwrap().starts_with("/api/v1/"))
    );
}
