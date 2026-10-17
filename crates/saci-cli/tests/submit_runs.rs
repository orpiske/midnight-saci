//! `saci backend start` and `saci run` against a real backend process.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::process::{Child, Command};

const SACI: &str = env!("CARGO_BIN_EXE_saci");
const TOKEN: &str = "a-client-token";

const PLAN: &str = "# Test Plan: Service health

## Phase 1: Main

### Test 1.1: Check health

Run `curl $URL/health`.

**Expected result:**

- The status is 200.
";

/// A backend started with `saci backend start`, killed on drop.
struct Backend {
    url: String,
    dir: tempfile::TempDir,
    _process: Child,
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

fn config(dir: &Path, port: u16, auth: bool) -> String {
    let auth = if auth {
        "\n  auth:\n    token_env: SACI_TEST_TOKEN"
    } else {
        ""
    };
    format!(
        "backend:\n  listen: 127.0.0.1:{port}\n  data_dir: {}{auth}\nllm:\n  base_url: http://model:11434/v1\n  \
         model: test-model\nenvironments:\n  local:\n    parameters:\n      URL: http://service\n",
        dir.join("data").display()
    )
}

impl Backend {
    async fn start(auth: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port().await;
        let path = dir.path().join("saci.yaml");
        std::fs::write(&path, config(dir.path(), port, auth)).unwrap();
        std::fs::write(dir.path().join("plan.md"), PLAN).unwrap();
        let process = Command::new(SACI)
            .args(["backend", "start", "--config"])
            .arg(&path)
            .env("SACI_TEST_TOKEN", TOKEN)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let address = format!("127.0.0.1:{port}");
        for _ in 0..200 {
            if tokio::net::TcpStream::connect(&address).await.is_ok() {
                return Self {
                    url: format!("http://{address}"),
                    dir,
                    _process: process,
                };
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the backend did not start on {address}");
    }

    fn plan(&self) -> String {
        self.dir.path().join("plan.md").display().to_string()
    }
}

async fn saci(backend: &str, args: &[&str], token: Option<&str>) -> Output {
    let mut command = Command::new(SACI);
    command
        .arg("--backend")
        .arg(backend)
        .args(args)
        .env_remove("SACI_TOKEN");
    if let Some(token) = token {
        command.env("SACI_TOKEN", token);
    }
    command.output().await.unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[tokio::test]
async fn run_prints_the_accepted_run_and_repeats_safely_with_its_key() {
    let backend = Backend::start(false).await;
    let plan = backend.plan();
    let args = [
        "run",
        &plan,
        "--environment",
        "local",
        "--idempotency-key",
        "k-1",
    ];
    let first = saci(&backend.url, &args, None).await;
    assert!(first.status.success(), "{}", text(&first.stderr));
    let id = text(&first.stdout).trim().to_owned();
    assert!(!id.is_empty() && !id.contains(char::is_whitespace), "{id}");
    assert!(text(&first.stderr).contains("accepted"));
    let again = saci(&backend.url, &args, None).await;
    assert!(again.status.success());
    assert_eq!(
        text(&again.stdout).trim(),
        id,
        "the same key returns the same run"
    );
    assert!(text(&again.stderr).contains("already accepted"));
    let json = saci(&backend.url, &["run", &plan, "-e", "local", "--json"], None).await;
    let summary: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_ne!(summary["id"], id.as_str(), "a new key starts a new run");
    assert_eq!(summary["plan_title"], "Service health");
}

#[tokio::test]
async fn authentication_failures_say_which_token_to_set() {
    let backend = Backend::start(true).await;
    let plan = backend.plan();
    let args = ["run", &plan, "--environment", "local"];
    let refused = saci(&backend.url, &args, None).await;
    assert_eq!(refused.status.code(), Some(3));
    assert!(
        text(&refused.stderr).contains("Set SACI_TOKEN"),
        "{}",
        text(&refused.stderr)
    );
    let accepted = saci(&backend.url, &args, Some(TOKEN)).await;
    assert!(accepted.status.success(), "{}", text(&accepted.stderr));
}

#[tokio::test]
async fn backend_validation_is_reported_as_invalid_input() {
    let backend = Backend::start(false).await;
    let plan = backend.plan();
    let output = saci(
        &backend.url,
        &["run", &plan, "--environment", "staging"],
        None,
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("staging"),
        "{}",
        text(&output.stderr)
    );
}

#[tokio::test]
async fn an_invalid_plan_is_rejected_before_contacting_the_backend() {
    let dir = tempfile::tempdir().unwrap();
    let plan = dir.path().join("plan.md");
    std::fs::write(&plan, "no title here\n").unwrap();
    let url = format!("http://127.0.0.1:{}", free_port().await);
    let output = saci(&url, &["run", plan.to_str().unwrap(), "-e", "local"], None).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("is not a valid plan"),
        "{}",
        text(&output.stderr)
    );
}

#[tokio::test]
async fn an_unreachable_backend_says_how_to_start_one() {
    let dir = tempfile::tempdir().unwrap();
    let plan = dir.path().join("plan.md");
    std::fs::write(&plan, PLAN).unwrap();
    let url = format!("http://127.0.0.1:{}", free_port().await);
    let output = saci(&url, &["run", plan.to_str().unwrap(), "-e", "local"], None).await;
    assert_eq!(output.status.code(), Some(3));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("saci backend start --config"), "{stderr}");
}

#[tokio::test]
async fn a_lost_reply_names_the_key_for_a_safe_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer).await;
            drop(socket);
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let plan = dir.path().join("plan.md");
    std::fs::write(&plan, PLAN).unwrap();
    let args = [
        "run",
        plan.to_str().unwrap(),
        "-e",
        "local",
        "--idempotency-key",
        "k-9",
    ];
    let output = saci(&url, &args, None).await;
    assert_eq!(output.status.code(), Some(4));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("--idempotency-key k-9"), "{stderr}");
}

#[tokio::test]
async fn backend_start_reports_invalid_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saci.yaml");
    std::fs::write(&path, "llm: {}\n").unwrap();
    let output = Command::new(SACI)
        .args(["backend", "start", "--config"])
        .arg(&path)
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("saci.yaml"),
        "{}",
        text(&output.stderr)
    );
}
