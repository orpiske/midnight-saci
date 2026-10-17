//! The worker's reporting client against a real backend on a local port.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use saci_backend::api::{Backend, ClientAuth, Supervisor, issue_token, router};
use saci_backend::store::Store;
use saci_core::config::Config;
use saci_core::event::EventKind;
use saci_core::ids::RunId;
use saci_core::status::Outcome;
use saci_worker::events::EventSink;
use saci_worker::llm::Backoff;
use saci_worker::report::{
    BackendClient, ReportError, deliver, fetch_specification, watch_cancellation,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const FAST: Backoff = Backoff {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(50),
};

const PLAN: &str = "# Test Plan: Reporting\n\n## Phase 1: Main\n\n### Test 1.1: Check\n\nRun `true`.\n\n**Expected result:**\n\n- It exits 0.\n";

struct Idle;

impl Supervisor for Idle {
    fn submitted(&self, _: RunId) {}
    fn cancel_requested(&self, _: RunId) {}
}

struct Served {
    url: String,
    backend: Backend,
    _dir: tempfile::TempDir,
}

async fn serve() -> Served {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let config = Config::from_yaml(
        "llm:\n  base_url: http://m/v1\n  model: m\nenvironments:\n  local: {}\n",
        "t.yaml",
    )
    .unwrap();
    let backend = Backend::new(store, config, Arc::new(Idle));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = router(backend.clone(), ClientAuth::Open);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Served {
        url,
        backend,
        _dir: dir,
    }
}

async fn submit(served: &Served) -> RunId {
    let body = json!({"bundle": {"entry": "plan.md", "documents": {"plan.md": PLAN}}, "environment": "local"});
    let reply: serde_json::Value = rig::rig_reqwest::reqwest::Client::new()
        .post(format!("{}/api/v1/runs", served.url))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    RunId::parse(reply["id"].as_str().unwrap()).unwrap()
}

async fn client(served: &Served) -> BackendClient {
    let run = submit(served).await;
    let token = issue_token(&served.backend, &run).await.unwrap();
    BackendClient::new(&served.url, run, token).unwrap()
}

#[tokio::test]
async fn the_worker_fetches_only_its_own_specification() {
    let served = serve().await;
    let own = client(&served).await;
    let spec = fetch_specification(&own, FAST).await.unwrap();
    assert_eq!(&spec.run_id, own.run());
    let other = submit(&served).await;
    let token = issue_token(&served.backend, &submit(&served).await)
        .await
        .unwrap();
    let foreign = BackendClient::new(&served.url, other, token).unwrap();
    let error = foreign.specification().await.unwrap_err();
    assert!(
        matches!(
            error,
            ReportError::Rejected(saci_core::api::ErrorCode::Forbidden, _)
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn delivered_events_are_acknowledged_and_outlive_the_worker() {
    let served = serve().await;
    let client = client(&served).await;
    let (sink, events) = EventSink::new(client.run().clone());
    sink.emit(None, None, EventKind::RunStarted { model: "m".into() });
    sink.emit(
        None,
        None,
        EventKind::RunFinished {
            outcome: Outcome::Error,
            reason: "stopped".into(),
        },
    );
    drop(sink);
    let acknowledged = deliver(&client, events, FAST).await.unwrap();
    assert_eq!(acknowledged, 2);
    let run = client.run().clone();
    drop(client);
    let (stored, finished) = served.backend.store.events(&run, 0, 10).await.unwrap();
    assert_eq!(stored.len(), 2);
    assert!(finished);
}

#[tokio::test]
async fn resent_events_are_not_duplicated() {
    let served = serve().await;
    let client = client(&served).await;
    let (sink, mut events) = EventSink::new(client.run().clone());
    sink.emit(None, None, EventKind::RunStarted { model: "m".into() });
    sink.emit(None, None, EventKind::CancellationRequested);
    let batch = vec![events.recv().await.unwrap(), events.recv().await.unwrap()];
    assert_eq!(client.send(batch.clone()).await.unwrap(), 2);
    assert_eq!(client.send(batch).await.unwrap(), 2);
    let (stored, _) = served
        .backend
        .store
        .events(client.run(), 0, 10)
        .await
        .unwrap();
    assert_eq!(stored.len(), 2);
}

#[tokio::test]
async fn cancellation_requests_reach_the_worker() {
    let served = serve().await;
    let client = client(&served).await;
    let cancel = CancellationToken::new();
    let stop = CancellationToken::new();
    let watcher = {
        let (client, cancel, stop) = (client.clone(), cancel.clone(), stop.clone());
        tokio::spawn(async move {
            watch_cancellation(&client, &cancel, &stop, Duration::from_millis(50)).await;
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!cancel.is_cancelled());
    served
        .backend
        .store
        .request_cancel(client.run(), time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), watcher)
        .await
        .unwrap()
        .unwrap();
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn an_unreachable_backend_is_a_transient_failure() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client =
        BackendClient::new(&format!("http://{address}"), RunId::generate(), "t".into()).unwrap();
    let error = client.specification().await.unwrap_err();
    assert!(matches!(error, ReportError::Transient(_)), "{error:?}");
}

/// An observation of `unit` whose stdout is `bytes` long.
fn observation(unit: &saci_core::run::UnitIdentity, index: u64, bytes: usize) -> EventKind {
    use saci_core::evidence::{CapturedText, Evidence, EvidenceRecord, ShellEvidence, Termination};
    let record = EvidenceRecord::Shell(ShellEvidence {
        command: "cat big".into(),
        stdout: CapturedText::complete("x".repeat(bytes)),
        stderr: CapturedText::default(),
        exit_code: Some(0),
        termination: Termination::Exited,
        restarted: false,
        session_lost: false,
    });
    EventKind::Observation {
        evidence: Box::new(Evidence {
            id: saci_core::ids::EvidenceId::from_index(index),
            unit: unit.key.clone(),
            attempt: 1,
            recorded_at: time::OffsetDateTime::now_utc(),
            duration_ms: 1,
            record,
        }),
    }
}

fn stdout_of(kind: &EventKind) -> &saci_core::evidence::ShellEvidence {
    match kind {
        EventKind::Observation { evidence } => match &evidence.record {
            saci_core::evidence::EvidenceRecord::Shell(shell) => shell,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_large_backlog_is_split_and_an_oversized_observation_cut_to_fit() {
    let served = serve().await;
    let client = client(&served).await;
    let spec = fetch_specification(&client, FAST).await.unwrap();
    let plan = saci_core::bundle::expand(&spec.bundle).unwrap();
    let unit = &plan.units[0].identity;
    let (sink, events) = EventSink::new(client.run().clone());
    sink.emit(None, None, EventKind::RunStarted { model: "m".into() });
    sink.emit(Some(unit), Some(1), EventKind::AttemptStarted);
    // Well past the 2 MiB a default request body allows, in many events.
    for index in 1..=60 {
        sink.emit(Some(unit), Some(1), observation(unit, index, 100 * 1024));
    }
    let oversized = saci_core::api::MAX_EVENT_BATCH_BYTES + 1024;
    sink.emit(Some(unit), Some(1), observation(unit, 61, oversized));
    drop(sink);
    let acknowledged = deliver(&client, events, FAST).await.unwrap();
    assert_eq!(acknowledged, 63);
    let (stored, _) = served
        .backend
        .store
        .events(client.run(), 0, 100)
        .await
        .unwrap();
    assert_eq!(stored.len(), 63);
    let shell = stdout_of(&stored[62].kind);
    assert!(shell.stdout.truncated);
    assert_eq!(shell.stdout.total_bytes, oversized as u64);
    assert!(shell.stdout.text.len() < saci_core::api::MAX_EVENT_BATCH_BYTES);
}
