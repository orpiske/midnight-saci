//! `saci watch` against local bare repositories and a backend whose worker
//! the test plays: the test receives each submitted run and finishes it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

use saci_backend::api::{Backend, ClientAuth, Supervisor, issue_token, router};
use saci_backend::store::Store;
use saci_core::config::Config;
use saci_core::event::{EventKind, RunEvent};
use saci_core::ids::RunId;
use saci_core::status::Outcome;
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

const SACI: &str = env!("CARGO_BIN_EXE_saci");

fn plan(title: &str) -> String {
    format!(
        "# Test Plan: {title}

## Phase 1: Main

### Test 1.1: Check health

Run `curl $URL/health`.

**Expected result:**

- The status is 200.
"
    )
}

/// Sends each submitted run to the test.
struct Submissions(mpsc::UnboundedSender<RunId>);

impl Supervisor for Submissions {
    fn submitted(&self, run: RunId) {
        let _ = self.0.send(run);
    }
    fn cancel_requested(&self, _: RunId) {}
}

/// Two remote repositories, a backend and a watcher configuration.
struct Fixture {
    dir: tempfile::TempDir,
    backend: Backend,
    url: String,
    runs: mpsc::UnboundedReceiver<RunId>,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        text(&output.stderr)
    );
    text(&output.stdout).trim().to_owned()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

impl Fixture {
    async fn start(quiet_secs: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for name in ["server", "tests"] {
            let bare = dir.path().join(format!("remote/{name}.git"));
            std::fs::create_dir_all(&bare).unwrap();
            git(&bare, &["init", "--bare", "--initial-branch=main"]);
            let work = dir.path().join(format!("work/{name}"));
            std::fs::create_dir_all(&work).unwrap();
            git(&work, &["init", "--initial-branch=main"]);
            git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        }
        let fixture = Self::serve(dir).await;
        fixture.write("tests", "plans/smoke.md", &plan("Smoke"));
        fixture.write("tests", "plans/upgrade.md", &plan("Upgrade"));
        fixture.write("server", "VERSION", "1");
        fixture.configure(quiet_secs);
        fixture
    }

    async fn serve(dir: tempfile::TempDir) -> Self {
        let store = Store::open(&dir.path().join("data")).await.unwrap();
        let yaml = "llm:\n  base_url: http://m/v1\n  model: m\nenvironments:\n  local: {}\n";
        let config = Config::from_yaml(yaml, "saci.yaml").unwrap();
        let (sender, runs) = mpsc::unbounded_channel();
        let backend = Backend::new(store, config, Arc::new(Submissions(sender)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(backend.clone(), ClientAuth::Open);
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self {
            dir,
            backend,
            url,
            runs,
        }
    }

    fn configure(&self, quiet_secs: u64) {
        let remote = |name: &str| self.dir.path().join(format!("remote/{name}.git"));
        let yaml = format!(
            "workdir: {}\npoll_interval_secs: 1\nquiet_period_secs: {quiet_secs}\nrepositories:\n  \
             - {{ name: server, url: '{}', branch: main }}\n  \
             - {{ name: tests, url: '{}', branch: main }}\ntriggers:\n  - name: smoke\n    \
             on: [server]\n    plans:\n      - {{ plan: tests/plans/smoke.md, environment: local }}\n      \
             - {{ plan: tests/plans/missing.md, environment: local }}\n      \
             - {{ plan: tests/plans/upgrade.md, environment: local, policy: default }}\n",
            self.workdir().display(),
            remote("server").display(),
            remote("tests").display(),
        );
        std::fs::write(self.dir.path().join("watch.yaml"), yaml).unwrap();
    }

    fn workdir(&self) -> PathBuf {
        self.dir.path().join("watch")
    }

    /// Commits `content` to `path` in repository `name` and pushes it.
    fn write(&self, name: &str, path: &str, content: &str) -> String {
        let work = self.dir.path().join(format!("work/{name}"));
        let file = work.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, content).unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", &format!("Change {path}")]);
        git(&work, &["push", "origin", "main"]);
        git(&work, &["rev-parse", "HEAD"])
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(SACI);
        command
            .arg("--backend")
            .arg(&self.url)
            .arg("watch")
            .args(args)
            .arg("--config")
            .arg(self.dir.path().join("watch.yaml"))
            .env_remove("SACI_TOKEN")
            .kill_on_drop(true);
        command
    }

    async fn saci(&self, args: &[&str]) -> Output {
        self.command(args).output().await.unwrap()
    }

    fn spawn(&self, args: &[&str]) -> Child {
        let mut command = self.command(args);
        command.stdout(Stdio::null()).stderr(Stdio::piped());
        command.spawn().unwrap()
    }

    async fn next_run(&mut self) -> RunId {
        tokio::time::timeout(Duration::from_secs(30), self.runs.recv())
            .await
            .expect("no run was submitted in 30 s")
            .unwrap()
    }

    async fn no_run_for(&mut self, duration: Duration) {
        if let Ok(run) = tokio::time::timeout(duration, self.runs.recv()).await {
            panic!("unexpected run {run:?}");
        }
    }

    /// Ends `run` with `outcome`, as its worker would.
    async fn finish(&self, run: &RunId, outcome: Outcome) {
        let token = issue_token(&self.backend, run).await.unwrap();
        let event = |seq, kind| RunEvent {
            run_id: run.clone(),
            seq,
            at: OffsetDateTime::now_utc(),
            unit: None,
            attempt: None,
            source: None,
            kind,
        };
        let started = EventKind::RunStarted {
            model: "m".into(),
            environment: None,
        };
        let reason = "test".to_owned();
        let events = [
            event(1, started),
            event(2, EventKind::RunFinished { outcome, reason }),
        ];
        let response = reqwest::Client::new()
            .post(format!("{}/api/v1/worker/runs/{run}/events", self.url))
            .bearer_auth(token)
            .json(&json!({ "events": events }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    async fn plan_title(&self, run: &RunId) -> String {
        let summary: Value = reqwest::get(format!("{}/api/v1/runs/{run}", self.url))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        summary["plan_title"].as_str().unwrap().to_owned()
    }

    fn clone_head(&self, name: &str) -> String {
        git(
            &self.workdir().join("repos").join(name),
            &["rev-parse", "HEAD"],
        )
    }
}

async fn ends_ok(child: Child) -> String {
    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .expect("the watcher did not exit")
        .unwrap();
    let log = text(&output.stderr);
    assert!(output.status.success(), "{log}");
    log
}

#[tokio::test]
async fn a_change_runs_the_plans_in_order_and_once_per_snapshot() {
    let mut fixture = Fixture::start(0).await;
    let first = fixture.saci(&["--once"]).await;
    assert!(first.status.success(), "{}", text(&first.stderr));
    assert!(text(&first.stderr).contains("recorded the current commits"));
    fixture.no_run_for(Duration::from_millis(200)).await;

    let commit = fixture.write("server", "VERSION", "2");
    let watcher = fixture.spawn(&["--once"]);
    let smoke = fixture.next_run().await;
    assert_eq!(fixture.plan_title(&smoke).await, "Smoke");
    assert_eq!(fixture.clone_head("server"), commit);
    fixture.no_run_for(Duration::from_millis(1500)).await;
    fixture.finish(&smoke, Outcome::Error).await;
    let upgrade = fixture.next_run().await;
    assert_eq!(fixture.plan_title(&upgrade).await, "Upgrade");
    fixture.finish(&upgrade, Outcome::Failed).await;
    let log = ends_ok(watcher).await;
    assert!(log.contains("tests/plans/missing.md skipped"), "{log}");

    let again = fixture.saci(&["--once"]).await;
    assert!(again.status.success());
    fixture.no_run_for(Duration::from_millis(200)).await;
    let status = text(&fixture.saci(&["status"]).await.stdout);
    assert!(status.contains(&format!("{smoke} error")), "{status}");
    assert!(status.contains(&format!("{upgrade} failed")), "{status}");
    assert!(status.contains("skipped: not a valid plan"), "{status}");
}

#[tokio::test]
async fn a_restart_waits_for_the_recorded_run_instead_of_submitting_again() {
    let mut fixture = Fixture::start(0).await;
    let watcher = fixture.spawn(&["--once", "--run-now"]);
    let smoke = fixture.next_run().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(watcher);
    let status = text(&fixture.saci(&["status"]).await.stdout);
    assert!(
        status.contains(&format!("Waiting for: {smoke}")),
        "{status}"
    );

    let restarted = fixture.spawn(&["--once"]);
    fixture.no_run_for(Duration::from_millis(1500)).await;
    fixture.finish(&smoke, Outcome::Passed).await;
    let upgrade = fixture.next_run().await;
    fixture.finish(&upgrade, Outcome::Passed).await;
    ends_ok(restarted).await;
    fixture.no_run_for(Duration::from_millis(200)).await;
}

#[tokio::test]
async fn changes_within_the_quiet_period_give_one_cycle() {
    let mut fixture = Fixture::start(3).await;
    assert!(fixture.saci(&["--once"]).await.status.success());
    let watcher = fixture.spawn(&[]);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let server = fixture.write("server", "VERSION", "2");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let tests = fixture.write("tests", "plans/smoke.md", &plan("Smoke 2"));
    let smoke = fixture.next_run().await;
    assert_eq!(fixture.plan_title(&smoke).await, "Smoke 2");
    assert_eq!(fixture.clone_head("server"), server);
    assert_eq!(fixture.clone_head("tests"), tests);
    fixture.finish(&smoke, Outcome::Passed).await;
    let upgrade = fixture.next_run().await;
    fixture.finish(&upgrade, Outcome::Passed).await;
    fixture.no_run_for(Duration::from_secs(3)).await;

    let pid = i32::try_from(watcher.id().unwrap()).unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    let log = ends_ok(watcher).await;
    assert_eq!(log.matches("cycle started").count(), 1, "{log}");
}

/// Points the watcher at a backend on a free port that only it can start,
/// whose environments do not include the plans' environment.
async fn local_backend(fixture: &Fixture) -> u16 {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let backend = format!(
        "backend:\n  listen: 127.0.0.1:{port}\n  data_dir: {}\nllm:\n  base_url: http://m/v1\n  \
         model: m\nenvironments:\n  local: {{}}\n",
        fixture.dir.path().join("started").display()
    );
    std::fs::write(fixture.dir.path().join("saci.yaml"), backend).unwrap();
    let path = fixture.dir.path().join("watch.yaml");
    let watch = std::fs::read_to_string(&path)
        .unwrap()
        .replace("environment: local", "environment: nowhere");
    let section = format!("backend:\n  url: http://127.0.0.1:{port}\n  config: saci.yaml\n");
    std::fs::write(&path, section + &watch).unwrap();
    port
}

#[tokio::test]
async fn the_watcher_starts_a_local_backend_and_stops_it_on_exit() {
    let fixture = Fixture::start(0).await;
    let port = local_backend(&fixture).await;
    let log = ends_ok(fixture.spawn(&["--once", "--run-now"])).await;
    assert!(
        log.contains("started `saci backend start --config"),
        "{log}"
    );
    assert_eq!(
        log.matches("skipped").count(),
        3,
        "the started backend refuses the environment: {log}"
    );
    let address = format!("127.0.0.1:{port}");
    assert!(
        tokio::net::TcpStream::connect(&address).await.is_err(),
        "the started backend still answers"
    );
}
