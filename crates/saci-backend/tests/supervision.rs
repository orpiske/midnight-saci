//! Supervision against a fake runtime whose containers are state files:
//! removal after a finished run, lost workers, cancellation, the backend's
//! own deadline, uncertain launches and restart recovery.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use saci_backend::api::{Backend, ClientAuth, issue_token, router};
use saci_backend::launcher::{ContainerLauncher, Launcher};
use saci_backend::store::Store;
use saci_backend::supervisor::{self, Timing};
use saci_core::config::{Config, Runtime};
use saci_core::event::{EventKind, RunEvent};
use saci_core::ids::RunId;
use saci_core::redaction::Redactor;
use saci_core::run::WorkerImage;
use saci_core::status::{Lifecycle, Outcome};
use serde_json::{Value, json};

const PLAN: &str = "# Test Plan: Supervised\n\n## Phase 1: Main\n\n### Test 1.1: Check\n\nRun `true`.\n\n\
                    **Expected result:**\n\n- It exits 0.\n";

/// A runtime whose containers are `state-<name>` files holding
/// "<status> <exit code>", written atomically so that a concurrent
/// `inspect` never reads a partial file. Marker files make the next launch time out
/// with (`launch-slow`) or without (`launch-lost`) creating the container.
/// Every container runs image ID `4b2f8e0c1d3a`, pulled from a registry.
const FAKE_RUNTIME: &str = r#"#!/bin/sh
D='@DIR@'
echo "$*" >> "$D/calls"
name=""; last=""
for a in "$@"; do case "$a" in --name=*) name="${a#--name=}";; esac; last="$a"; done
case "$1" in
  version) echo 5.4.0;;
  run)
    if [ -e "$D/state-$name" ]; then echo "Error: the container name \"$name\" is already in use" >&2; exit 125; fi
    if [ -e "$D/launch-lost" ]; then rm "$D/launch-lost"; sleep 3; exit 0; fi
    echo "running 0" > "$D/tmp-$name" && mv "$D/tmp-$name" "$D/state-$name"
    if [ -e "$D/launch-slow" ]; then rm "$D/launch-slow"; sleep 3; fi
    echo 0123456789ab;;
  inspect)
    if [ ! -e "$D/state-$last" ]; then echo "Error: no such container $last" >&2; exit 125; fi
    case "$*" in *'{{.Image}}'*) echo 4b2f8e0c1d3a;; *) cat "$D/state-$last";; esac;;
  image) echo '["localhost/saci-worker@sha256:feed"]';;
  stop)
    if [ -e "$D/state-$last" ]; then echo "exited 143" > "$D/tmp-$last" && mv "$D/tmp-$last" "$D/state-$last"; else echo "Error: no such container $last" >&2; exit 125; fi;;
  rm) rm -f "$D/state-$last";;
  logs) echo "worker: cannot reach the backend: connection refused";;
  ps) for f in "$D"/state-*; do [ -e "$f" ] && echo "${f##*/state-}"; done;;
esac
"#;

const FAST: Timing = Timing {
    poll: Duration::from_millis(50),
    stop_grace: Duration::from_secs(1),
    deadline_grace: Duration::ZERO,
    exit_grace: Duration::from_millis(500),
};

struct Harness {
    backend: Backend,
    url: String,
    runtime: PathBuf,
    _dir: tempfile::TempDir,
}

fn config(limits: &str) -> Config {
    let yaml = format!(
        "backend:\n  worker_url: http://127.0.0.1:1\nworker:\n  runtime: podman\n{limits}llm:\n  base_url: http://m/v1\n  \
         model: m\nenvironments:\n  local: {{}}\n"
    );
    Config::from_yaml(&yaml, "saci.yaml").unwrap()
}

fn fake_runtime(dir: &Path) -> PathBuf {
    let runtime = dir.join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let path = runtime.join("podman");
    std::fs::write(
        &path,
        FAKE_RUNTIME.replace("@DIR@", &runtime.display().to_string()),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    runtime
}

fn launcher(config: &Config, runtime: &Path) -> Launcher {
    let container =
        ContainerLauncher::resolve(Runtime::Podman, runtime.as_os_str(), &config.worker).unwrap();
    Launcher::Container(container.with_launch_timeout(Duration::from_millis(400)))
}

impl Harness {
    /// A backend with a supervisor on the fake runtime; `prepare` runs on
    /// the store before supervision starts, as an earlier backend process.
    async fn start(limits: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = fake_runtime(dir.path());
        Self::supervise(dir, runtime, config(limits)).await
    }

    async fn supervise(dir: tempfile::TempDir, runtime: PathBuf, config: Config) -> Self {
        let store = Store::open(&dir.path().join("data")).await.unwrap();
        let launcher = launcher(&config, &runtime);
        let (commands, receiver) = supervisor::channel();
        let backend = Backend::new(store, config, Arc::new(commands));
        tokio::spawn(Box::pin(supervisor::run(
            backend.clone(),
            Ok(launcher),
            receiver,
            FAST,
        )));
        let url = serve(&backend).await;
        Self {
            backend,
            url,
            runtime,
            _dir: dir,
        }
    }

    fn container(&self, run: &RunId) -> PathBuf {
        self.runtime.join(format!("state-saci-{run}"))
    }

    fn set_state(&self, run: &RunId, state: &str) {
        let partial = self.runtime.join(format!("tmp-saci-{run}"));
        std::fs::write(&partial, state).unwrap();
        std::fs::rename(partial, self.container(run)).unwrap();
    }

    fn calls(&self, prefix: &str) -> usize {
        let calls = std::fs::read_to_string(self.runtime.join("calls")).unwrap_or_default();
        calls
            .lines()
            .filter(|line| line.starts_with(prefix))
            .count()
    }

    /// Stores events as the worker would report them.
    async fn report(&self, run: &RunId, kinds: Vec<EventKind>) {
        let first = self.backend.store.run(run).await.unwrap().report.last_seq + 1;
        let events = numbered(run, first, kinds);
        let result = self
            .backend
            .store
            .append_events(run, events, Redactor::new())
            .await;
        if let Err(e) = result {
            let record = self.backend.store.run(run).await.unwrap();
            let reason = record.report.reason;
            panic!("cannot store the worker's events ({e:?}); the run already ended: {reason:?}");
        }
        self.backend.notify();
    }

    async fn finished(&self, run: &RunId) -> (Outcome, String) {
        eventually(|| async {
            let record = self.backend.store.run(run).await.unwrap();
            (record.report.lifecycle == Lifecycle::Finished).then(|| {
                (
                    record.report.outcome.unwrap(),
                    record.report.reason.unwrap_or_default(),
                )
            })
        })
        .await
    }
}

/// `kinds` as the run's events, numbered from `first`.
fn numbered(run: &RunId, first: u64, kinds: Vec<EventKind>) -> Vec<RunEvent> {
    kinds
        .into_iter()
        .zip(first..)
        .map(|(kind, seq)| RunEvent {
            run_id: run.clone(),
            seq,
            at: time::OffsetDateTime::now_utc(),
            unit: None,
            attempt: None,
            source: None,
            kind,
        })
        .collect()
}

async fn serve(backend: &Backend) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let app = router(backend.clone(), ClientAuth::Open);
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

async fn submit(url: &str) -> RunId {
    let body = json!({"bundle": {"entry": "plan.md", "documents": {"plan.md": PLAN}}, "environment": "local"});
    let client = reqwest::Client::new();
    let reply: Value = client
        .post(format!("{url}/runs"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    RunId::parse(reply["id"].as_str().unwrap()).unwrap()
}

async fn eventually<T, F, Fut>(mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    for _ in 0..200 {
        if let Some(value) = check().await {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the condition did not hold within 10 s");
}

async fn exists(path: &Path, wanted: bool) {
    eventually(|| async { (path.exists() == wanted).then_some(()) }).await;
}

fn started() -> EventKind {
    EventKind::RunStarted { model: "m".into() }
}

fn run_finished(outcome: Outcome, reason: &str) -> EventKind {
    EventKind::RunFinished {
        outcome,
        reason: reason.into(),
    }
}

#[tokio::test]
async fn a_finished_run_s_worker_is_removed_once_it_exits() {
    let harness = Harness::start("").await;
    let run = submit(&harness.url).await;
    exists(&harness.container(&run), true).await;
    harness
        .report(
            &run,
            vec![started(), run_finished(Outcome::Passed, "all passed")],
        )
        .await;
    harness.set_state(&run, "exited 0");
    exists(&harness.container(&run), false).await;
    assert_eq!(
        harness.finished(&run).await.1,
        "all passed",
        "the worker's own end is kept"
    );
    assert_eq!(harness.calls("run "), 1);
}

#[tokio::test]
async fn the_report_names_the_worker_s_image_after_the_worker_is_gone() {
    let harness = Harness::start("").await;
    let run = submit(&harness.url).await;
    exists(&harness.container(&run), true).await;
    harness.set_state(&run, "exited 2");
    exists(&harness.container(&run), false).await;
    let report = harness.backend.store.run(&run).await.unwrap().report;
    assert_eq!(report.lifecycle, Lifecycle::Finished);
    let image = WorkerImage {
        reference: harness.backend.config.worker.image.clone(),
        digest: Some("sha256:feed".to_owned()),
        id: Some("sha256:4b2f8e0c1d3a".to_owned()),
    };
    assert_eq!(
        report.worker_image,
        Some(image),
        "the report names the image"
    );
    assert_eq!(
        harness.calls("image inspect"),
        1,
        "a known image is not asked again"
    );
}

#[tokio::test]
async fn a_lost_worker_ends_the_run_with_its_last_output_and_keeps_its_events() {
    let harness = Harness::start("").await;
    let run = submit(&harness.url).await;
    exists(&harness.container(&run), true).await;
    harness.report(&run, vec![started()]).await;
    harness.set_state(&run, "exited 3");
    let (outcome, reason) = harness.finished(&run).await;
    assert_eq!(outcome, Outcome::Error);
    assert!(
        reason.contains("exited with code 3") && reason.contains("cannot reach the backend"),
        "{reason}"
    );
    let (events, _) = harness.backend.store.events(&run, 0, 10).await.unwrap();
    assert_eq!(events.len(), 1, "delivered events are kept");
    exists(&harness.container(&run), false).await;
}

#[tokio::test]
async fn a_vanished_worker_ends_the_run() {
    let harness = Harness::start("").await;
    let run = submit(&harness.url).await;
    exists(&harness.container(&run), true).await;
    std::fs::remove_file(harness.container(&run)).unwrap();
    let (outcome, reason) = harness.finished(&run).await;
    assert_eq!(outcome, Outcome::Error);
    assert!(reason.contains("disappeared"), "{reason}");
}

#[tokio::test]
async fn cancellation_stops_the_worker_and_converges_to_cancelled() {
    let harness = Harness::start("").await;
    let run = submit(&harness.url).await;
    exists(&harness.container(&run), true).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/runs/{run}/cancellation", harness.url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let (outcome, reason) = harness.finished(&run).await;
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(reason.contains("stopped before reporting"), "{reason}");
    assert_eq!(harness.calls("stop "), 1);
    exists(&harness.container(&run), false).await;
}

#[tokio::test]
async fn the_backend_enforces_the_run_deadline_itself() {
    let harness = Harness::start("limits:\n  run_timeout_secs: 1\n  step_timeout_secs: 1\n").await;
    let run = submit(&harness.url).await;
    exists(&harness.container(&run), true).await;
    harness.report(&run, vec![started()]).await;
    let (outcome, reason) = harness.finished(&run).await;
    assert_eq!(outcome, Outcome::Error);
    assert!(reason.contains("exceeded its deadline"), "{reason}");
    assert_eq!(harness.calls("stop "), 1);
}

#[tokio::test]
async fn an_uncertain_launch_that_created_the_worker_is_not_repeated() {
    let harness = Harness::start("").await;
    std::fs::write(harness.runtime.join("launch-slow"), "").unwrap();
    let run = submit(&harness.url).await;
    exists(&harness.container(&run), true).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        harness.calls("run "),
        1,
        "the worker existed, so it was not launched again"
    );
    harness
        .report(&run, vec![started(), run_finished(Outcome::Passed, "done")])
        .await;
    harness.set_state(&run, "exited 0");
    exists(&harness.container(&run), false).await;
}

#[tokio::test]
async fn an_uncertain_launch_without_a_worker_is_launched_once_more() {
    let harness = Harness::start("").await;
    std::fs::write(harness.runtime.join("launch-lost"), "").unwrap();
    let run = submit(&harness.url).await;
    exists(&harness.container(&run), true).await;
    assert_eq!(
        harness.calls("run "),
        2,
        "{}",
        std::fs::read_to_string(harness.runtime.join("calls")).unwrap()
    );
    let record = harness.backend.store.run(&run).await.unwrap();
    assert_eq!(
        record.report.lifecycle,
        Lifecycle::Accepted,
        "the run is still being supervised"
    );
}

#[tokio::test]
async fn a_launch_failure_ends_the_run_with_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = fake_runtime(dir.path());
    let mut config = config("");
    config.backend.worker_url = None;
    let harness = Harness::supervise(dir, runtime, config).await;
    let run = submit(&harness.url).await;
    let (outcome, reason) = harness.finished(&run).await;
    assert_eq!(outcome, Outcome::Error);
    assert!(
        reason.contains("could not be launched") && reason.contains("backend.worker_url"),
        "{reason}"
    );
}

/// Runs left by an earlier backend process: one never launched, one whose
/// worker still runs, one whose worker is gone.
async fn earlier_backend(dir: &Path, runtime: &Path) -> [RunId; 3] {
    let store = Store::open(&dir.join("data")).await.unwrap();
    let (commands, _ignored) = supervisor::channel();
    let backend = Backend::new(store, config(""), Arc::new(commands));
    let url = serve(&backend).await;
    let never = submit(&url).await;
    let running = submit(&url).await;
    let lost = submit(&url).await;
    issue_token(&backend, &running).await.unwrap();
    issue_token(&backend, &lost).await.unwrap();
    std::fs::write(runtime.join(format!("state-saci-{running}")), "running 0").unwrap();
    [never, running, lost]
}

#[tokio::test]
async fn restart_recovery_never_repeats_a_run_that_may_have_acted() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = fake_runtime(dir.path());
    let [never, running, lost] = earlier_backend(dir.path(), &runtime).await;
    let harness = Harness::supervise(dir, runtime, config("")).await;
    let (outcome, reason) = harness.finished(&lost).await;
    assert_eq!(outcome, Outcome::Error);
    assert!(reason.contains("not repeated"), "{reason}");
    exists(&harness.container(&never), true).await;
    assert_eq!(
        harness.calls("run "),
        1,
        "only the never-launched run starts"
    );
    harness
        .report(
            &running,
            vec![started(), run_finished(Outcome::Passed, "done")],
        )
        .await;
    harness.set_state(&running, "exited 0");
    exists(&harness.container(&running), false).await;
    assert_eq!(harness.finished(&running).await.1, "done");
}

/// A run an earlier backend process saw finish, without removing its worker.
async fn finished_earlier(dir: &Path) -> RunId {
    let store = Store::open(&dir.join("data")).await.unwrap();
    let (commands, _ignored) = supervisor::channel();
    let backend = Backend::new(store, config(""), Arc::new(commands));
    let run = submit(&serve(&backend).await).await;
    let kinds = [started(), run_finished(Outcome::Passed, "done")];
    let events = numbered(&run, 1, kinds.into());
    let store = &backend.store;
    store
        .append_events(&run, events, Redactor::new())
        .await
        .unwrap();
    run
}

#[tokio::test]
async fn restart_removes_workers_of_finished_runs_only() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = fake_runtime(dir.path());
    let finished = finished_earlier(dir.path()).await;
    let foreign = RunId::parse("run-abc123").unwrap();
    for run in [&finished, &foreign] {
        std::fs::write(runtime.join(format!("state-saci-{run}")), "exited 0").unwrap();
    }
    let harness = Harness::supervise(dir, runtime, config("")).await;
    exists(&harness.container(&finished), false).await;
    assert!(
        harness.container(&foreign).exists(),
        "a worker of a run this backend does not know is left alone"
    );
}
