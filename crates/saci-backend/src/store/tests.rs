use std::collections::BTreeMap;

use saci_core::event::RunEvent;
use saci_core::plan::PlanBundle;
use saci_core::run::{ReportHeader, UnitIdentity, WorkerImage};
use saci_core::spec::{EnvironmentSpec, ExecutionLimits, LlmSettings, RunPolicy, SCHEMA_VERSION};
use saci_core::status::Outcome;
use serde::Deserialize;

use super::*;

#[derive(Deserialize)]
struct Fixture {
    header: ReportHeader,
    units: Vec<UnitIdentity>,
    events: Vec<RunEvent>,
}

fn fixture() -> Fixture {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../saci-core/tests/fixtures/runs/success.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn new_run(fixture: &Fixture, run_id: RunId) -> NewRun {
    let mut header = fixture.header.clone();
    header.run_id = run_id.clone();
    let spec = RunSpec {
        schema_version: SCHEMA_VERSION,
        run_id,
        created_at: header.created_at,
        bundle: PlanBundle {
            entry: "plan.md".into(),
            documents: BTreeMap::from([("plan.md".into(), "# Test Plan: X".into())]),
        },
        environment: EnvironmentSpec::default(),
        policy: RunPolicy::default(),
        limits: ExecutionLimits::default(),
        llm: LlmSettings {
            base_url: "http://model/v1".into(),
            model: "test-model".into(),
            api_key_env: None,
            api_key_secret: None,
        },
        judge: None,
    };
    NewRun {
        spec,
        report: RunReport::new(header, fixture.units.clone()),
        idempotency: None,
        actions: BTreeMap::new(),
    }
}

/// A store holding the success fixture's run, before any event.
struct Seeded {
    _dir: tempfile::TempDir,
    fixture: Fixture,
    id: RunId,
    store: Store,
}

impl Seeded {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fixture = fixture();
        let id = fixture.header.run_id.clone();
        let store = Store::open(dir.path()).await.unwrap();
        created(&store, new_run(&fixture, id.clone())).await;
        Self {
            _dir: dir,
            fixture,
            id,
            store,
        }
    }

    async fn append(&self, range: std::ops::Range<usize>) -> Result<u64, StoreError> {
        let events = self.fixture.events[range].to_vec();
        self.store
            .append_events(&self.id, events, Redactor::new())
            .await
    }
}

async fn created(store: &Store, run: NewRun) -> RunRecord {
    match store.create_run(run).await.unwrap() {
        Created::New(record) => *record,
        Created::Existing(_) => panic!("expected a new run"),
    }
}

#[tokio::test]
async fn records_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture();
    let id = fixture.header.run_id.clone();
    let store = Store::open(dir.path()).await.unwrap();
    created(&store, new_run(&fixture, id.clone())).await;
    let ack = store
        .append_events(&id, fixture.events.clone(), Redactor::new())
        .await
        .unwrap();
    assert_eq!(ack, fixture.events.len() as u64);
    let before = store.run(&id).await.unwrap();
    drop(store);
    let reopened = Store::open(dir.path()).await.unwrap();
    let after = reopened.run(&id).await.unwrap();
    assert_eq!(after, before);
    assert_eq!(after.report.outcome, Some(Outcome::Passed));
    assert_eq!(after.summary().last_event_seq, ack);
    let (events, finished) = reopened.events(&id, 0, 100).await.unwrap();
    assert_eq!(events, fixture.events);
    assert!(finished);
}

#[tokio::test]
async fn duplicates_are_ignored_and_gaps_store_nothing() {
    let seeded = Box::pin(Seeded::new()).await;
    assert_eq!(seeded.append(0..3).await.unwrap(), 3);
    assert_eq!(seeded.append(0..3).await.unwrap(), 3);
    let error = seeded.append(4..6).await.unwrap_err();
    assert!(matches!(
        error,
        StoreError::Contract(ContractError::OutOfOrder { .. })
    ));
    assert_eq!(seeded.append(2..5).await.unwrap(), 5);
    let (events, finished) = seeded.store.events(&seeded.id, 3, 100).await.unwrap();
    assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), [4, 5]);
    assert!(!finished);
}

#[tokio::test]
async fn finished_runs_are_protected() {
    let seeded = Box::pin(Seeded::new()).await;
    let all = seeded.fixture.events.len();
    seeded.append(0..all).await.unwrap();
    let mut late = seeded.fixture.events.last().unwrap().clone();
    late.seq += 1;
    let store = &seeded.store;
    let error = store
        .append_events(&seeded.id, vec![late], Redactor::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::Contract(ContractError::Finished)
    ));
    let change = store.update_report(&seeded.id, |_| Ok(())).await;
    assert!(matches!(
        change,
        Err(StoreError::Contract(ContractError::Finished))
    ));
    let now = OffsetDateTime::now_utc();
    let cancel = store.request_cancel(&seeded.id, now).await.unwrap();
    assert_eq!(
        cancel.cancel_requested_at, None,
        "a finished run is not cancelled"
    );
}

#[tokio::test]
async fn the_worker_image_is_recorded_after_the_run_finished() {
    let seeded = Box::pin(Seeded::new()).await;
    let all = seeded.fixture.events.len();
    seeded.append(0..all).await.unwrap();
    let before = seeded.store.run(&seeded.id).await.unwrap().report;
    let image = WorkerImage {
        reference: "quay.io/saci/worker:1.2".into(),
        digest: Some("sha256:bbb".into()),
        id: None,
    };
    seeded
        .store
        .set_worker_image(&seeded.id, image.clone())
        .await
        .unwrap();
    let after = seeded.store.run(&seeded.id).await.unwrap().report;
    assert_eq!(after.worker_image, Some(image.clone()));
    assert_eq!(
        RunReport {
            worker_image: None,
            ..after
        },
        before,
        "nothing else in the report changes"
    );
    let missing = RunId::parse("run-missing").unwrap();
    let error = seeded.store.set_worker_image(&missing, image).await;
    assert!(matches!(error, Err(StoreError::NotFound)));
}

#[tokio::test]
async fn observations_are_redacted_before_storage() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture();
    let id = fixture.header.run_id.clone();
    let store = Store::open(dir.path()).await.unwrap();
    created(&store, new_run(&fixture, id.clone())).await;
    let mut redactor = Redactor::new();
    redactor.add("TARGET", "target:8080");
    store
        .append_events(&id, fixture.events[..3].to_vec(), redactor)
        .await
        .unwrap();
    let record = store.run(&id).await.unwrap();
    let text = serde_json::to_string(&record.report).unwrap();
    assert!(
        text.contains("[REDACTED:TARGET]") && !text.contains("target:8080"),
        "{text}"
    );
    let (events, _) = store.events(&id, 0, 10).await.unwrap();
    assert!(
        !serde_json::to_string(&events)
            .unwrap()
            .contains("target:8080")
    );
}

#[tokio::test]
async fn idempotency_keys_return_the_same_run_or_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture();
    let store = Store::open(dir.path()).await.unwrap();
    let mut first = new_run(&fixture, RunId::generate());
    first.idempotency = Some(("key-1".into(), "hash-a".into()));
    let original = created(&store, first).await;
    let mut repeat = new_run(&fixture, RunId::generate());
    repeat.idempotency = Some(("key-1".into(), "hash-a".into()));
    let Created::Existing(existing) = store.create_run(repeat).await.unwrap() else {
        panic!("expected the existing run");
    };
    assert_eq!(existing.spec.run_id, original.spec.run_id);
    let mut different = new_run(&fixture, RunId::generate());
    different.idempotency = Some(("key-1".into(), "hash-b".into()));
    let conflict = store.create_run(different).await.unwrap_err();
    assert!(matches!(conflict, StoreError::IdempotencyConflict));
}

#[tokio::test]
async fn listing_pages_newest_first_and_finds_unfinished_runs() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = fixture();
    let store = Store::open(dir.path()).await.unwrap();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let id = RunId::generate();
        created(&store, new_run(&fixture, id.clone())).await;
        ids.push(id);
    }
    let first = store.list(None, 2).await.unwrap();
    let listed: Vec<_> = first.items.iter().map(|r| r.spec.run_id.clone()).collect();
    assert_eq!(listed, [ids[2].clone(), ids[1].clone()]);
    let second = store.list(first.next_cursor, 2).await.unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.next_cursor, None);
    assert_eq!(store.unfinished().await.unwrap().len(), 3);
    let at = OffsetDateTime::now_utc();
    store.request_cancel(&ids[0], at).await.unwrap();
    let again = store
        .request_cancel(&ids[0], at + time::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(
        again.cancel_requested_at.unwrap().unix_timestamp(),
        at.unix_timestamp()
    );
    assert!(matches!(
        store.run(&RunId::generate()).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn newer_schemas_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).await.unwrap());
    let connection = Connection::open(dir.path().join(DATABASE_FILE)).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    let error = Store::open(dir.path()).await.unwrap_err();
    assert!(error.to_string().contains("upgrade Saci"), "{error}");
}

/// Appends `secret` to every explanation, reason and message in `value`.
fn leak(value: &mut serde_json::Value, secret: &str) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                match item {
                    serde_json::Value::String(text)
                        if ["explanation", "reason", "message"].contains(&key.as_str()) =>
                    {
                        text.push_str(&format!(" via {secret}"));
                    }
                    other => leak(other, secret),
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|item| leak(item, secret)),
        _ => {}
    }
}

#[tokio::test]
async fn free_text_in_events_is_redacted_before_storage() {
    let dir = tempfile::tempdir().unwrap();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../saci-core/tests/fixtures/runs/failure.json"
    );
    let mut raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    leak(&mut raw["events"], "sk-leaked");
    let fixture: Fixture = serde_json::from_value(raw).unwrap();
    let id = fixture.header.run_id.clone();
    let store = Store::open(dir.path()).await.unwrap();
    created(&store, new_run(&fixture, id.clone())).await;
    let mut redactor = Redactor::new();
    redactor.add("KEY", "sk-leaked");
    store
        .append_events(&id, fixture.events.clone(), redactor)
        .await
        .unwrap();
    let report = serde_json::to_string(&store.run(&id).await.unwrap().report).unwrap();
    let (events, _) = store.events(&id, 0, 100).await.unwrap();
    let events = serde_json::to_string(&events).unwrap();
    for text in [report, events] {
        assert!(!text.contains("sk-leaked"), "{text}");
        assert!(text.contains("via [REDACTED:KEY]"), "{text}");
    }
}
