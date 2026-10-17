//! The walkthrough's plans run through the `saci-worker` binary with the
//! walkthrough playbook and the orders service: their `curl` commands run
//! in the worker's shell, and the verdicts follow what the service
//! answered, including when it is down.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use axum::Json;
use axum::routing::post;
use saci_core::event::RunEvent;
use saci_core::ids::RunId;
use saci_core::plan::PlanBundle;
use saci_core::run::{ReportHeader, RunReport};
use saci_core::spec::{
    EnvironmentSpec, ExecutionLimits, LlmSettings, RunPolicy, RunSpec, SCHEMA_VERSION,
};
use saci_core::status::Outcome;
use saci_testkit::fake_target;
use saci_testkit::playbook::Playbook;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::process::Command;

const WORKER: &str = env!("CARGO_BIN_EXE_saci-worker");

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Serves `router` on a free loopback port; returns its base URL.
async fn serve(router: axum::Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await });
    url
}

/// The scripted model answering from the walkthrough playbook.
async fn model() -> String {
    let path = repository().join("examples/walkthrough/playbook.json");
    let book = Playbook::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let router = axum::Router::new().route(
        "/v1/chat/completions",
        post(move |Json(request): Json<Value>| {
            let book = book.clone();
            async move { Json(book.reply(&request).unwrap()) }
        }),
    );
    format!("{}/v1", serve(router).await)
}

/// A URL nothing listens on.
fn closed() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// A run of the plan at `plan` against `service` with the playbook model.
async fn spec(plan: &str, service: &str) -> RunSpec {
    let text = std::fs::read_to_string(repository().join(plan)).unwrap();
    RunSpec {
        schema_version: SCHEMA_VERSION,
        run_id: RunId::generate(),
        created_at: time::OffsetDateTime::now_utc(),
        bundle: PlanBundle {
            entry: "plan.md".into(),
            documents: BTreeMap::from([("plan.md".to_owned(), text)]),
        },
        environment: EnvironmentSpec {
            name: "local".into(),
            parameters: BTreeMap::from([("SERVICE_URL".into(), service.into())]),
            ..EnvironmentSpec::default()
        },
        policy: RunPolicy::default(),
        limits: ExecutionLimits::default(),
        llm: LlmSettings {
            base_url: model().await,
            model: "walkthrough-playbook".into(),
            api_key_env: None,
            api_key_secret: None,
        },
    }
}

/// Runs `spec` in the worker binary and returns its events.
async fn events(spec: &RunSpec) -> Vec<RunEvent> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.json");
    std::fs::write(&path, serde_json::to_string(spec).unwrap()).unwrap();
    let output = Command::new(WORKER)
        .arg("--spec")
        .arg(&path)
        .arg("--workspace")
        .arg(dir.path())
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .output()
        .await
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Each unit's outcome when the plan at `plan` runs against `service`.
async fn outcomes(plan: &str, service: &str) -> Vec<Option<Outcome>> {
    let spec = spec(plan, service).await;
    let expanded = saci_core::bundle::expand(&spec.bundle).unwrap();
    let header = ReportHeader {
        run_id: spec.run_id.clone(),
        plan_title: expanded.plan.title.clone(),
        entry: "plan.md".into(),
        environment: "local".into(),
        policy: "default".into(),
        model: "walkthrough-playbook".into(),
        created_at: spec.created_at,
    };
    let units = expanded.units.iter().map(|u| u.identity.clone()).collect();
    let mut report = RunReport::new(header, units);
    for event in &events(&spec).await {
        report.apply(event).unwrap();
    }
    report.units.iter().map(|unit| unit.outcome).collect()
}

#[tokio::test]
async fn the_starter_plan_passes_only_when_the_service_answers() {
    let service = serve(fake_target::router()).await;
    let plan = "docs/templates/test-plan.md";
    assert_eq!(outcomes(plan, &service).await, [Some(Outcome::Passed)]);
    assert_eq!(outcomes(plan, &closed()).await, [Some(Outcome::Failed)]);
}

#[tokio::test]
async fn the_orders_plan_reads_and_fails_to_create() {
    let service = serve(fake_target::router()).await;
    let found = outcomes("examples/walkthrough/plans/orders.md", &service).await;
    assert_eq!(found, [Some(Outcome::Passed), Some(Outcome::Failed)]);
}
