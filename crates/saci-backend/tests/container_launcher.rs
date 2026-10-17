//! The Podman/Docker launcher: runtime resolution through `PATH`, the
//! arguments and environment a worker is started with (against a fake
//! runtime that records them), and (when `SACI_WORKER_IMAGE` names a built
//! image) a real run in a real container.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::Duration;

use saci_backend::launcher::container::{ContainerLauncher, Kind};
use saci_backend::launcher::{Assignment, Launcher, WorkerState, assignment, worker_name};
use saci_core::config::{Config, Runtime, WorkerConfig};
use saci_core::ids::RunId;
use saci_core::spec::RunSpec;

/// A fake runtime that records its arguments and environment in `dir` and
/// answers like Podman.
fn fake_runtime(dir: &Path, name: &str, version_fails: bool) {
    let version = if version_fails {
        "echo 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock' >&2; exit 1"
    } else {
        "echo 5.4.0"
    };
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{dir}/args'\nenv > '{dir}/env'\ncase \"$1\" in\n  version) {version};;\n  \
         run) echo 0123456789abcdef;;\n  inspect) echo 'exited 3';;\n  ps) printf 'saci-run-abc123\\nunrelated\\n';;\n  \
         stop|rm) echo \"Error: no such container $3\" >&2; exit 1;;\nesac\n",
        dir = dir.display()
    );
    let path = dir.join(name);
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn worker_config() -> WorkerConfig {
    let yaml = "worker:\n  image: localhost/saci-worker:test\n  network: saci\n  add_hosts: [\"host.containers.internal:host-gateway\"]\n\
                llm:\n  base_url: http://m/v1\n  model: m\nenvironments:\n  local: {}\n";
    Config::from_yaml(yaml, "saci.yaml").unwrap().worker
}

#[test]
fn auto_prefers_podman_and_falls_back_to_docker() {
    let both = tempfile::tempdir().unwrap();
    fake_runtime(both.path(), "podman", false);
    fake_runtime(both.path(), "docker", false);
    let only_docker = tempfile::tempdir().unwrap();
    fake_runtime(only_docker.path(), "docker", false);
    let worker = worker_config();
    let chosen =
        ContainerLauncher::resolve(Runtime::Auto, both.path().as_os_str(), &worker).unwrap();
    assert_eq!(chosen.kind(), Kind::Podman);
    let path = std::env::join_paths([only_docker.path(), both.path()]).unwrap();
    let first_in_path = ContainerLauncher::resolve(Runtime::Docker, &path, &worker).unwrap();
    assert_eq!(first_in_path.binary(), only_docker.path().join("docker"));
    let fallback =
        ContainerLauncher::resolve(Runtime::Auto, only_docker.path().as_os_str(), &worker).unwrap();
    assert_eq!(fallback.kind(), Kind::Docker);
}

#[test]
fn missing_or_non_executable_runtimes_are_named() {
    let dir = tempfile::tempdir().unwrap();
    let worker = worker_config();
    let missing =
        ContainerLauncher::resolve(Runtime::Podman, dir.path().as_os_str(), &worker).unwrap_err();
    assert!(
        missing
            .message
            .contains("podman is not in the backend's PATH"),
        "{}",
        missing.message
    );
    std::fs::write(dir.path().join("docker"), "not a program").unwrap();
    let error =
        ContainerLauncher::resolve(Runtime::Auto, dir.path().as_os_str(), &worker).unwrap_err();
    assert!(
        error
            .message
            .contains("podman is not in the backend's PATH"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("docker is not executable"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn an_unusable_runtime_reports_its_own_explanation() {
    let dir = tempfile::tempdir().unwrap();
    fake_runtime(dir.path(), "docker", true);
    let launcher =
        ContainerLauncher::resolve(Runtime::Docker, dir.path().as_os_str(), &worker_config())
            .unwrap();
    let error = launcher.check().await.unwrap_err();
    assert!(
        error.message.contains("is not usable") && error.message.contains("Docker daemon"),
        "{}",
        error.message
    );
}

fn sample_assignment() -> Assignment {
    Assignment {
        run: RunId::parse("run-abc123").unwrap(),
        environment: vec![
            (
                "SACI_BACKEND_URL".into(),
                "http://host.containers.internal:8080".into(),
            ),
            ("SACI_RUN_ID".into(), "run-abc123".into()),
            ("SACI_WORKER_TOKEN".into(), "worker-token-value".into()),
            (
                "SACI_SECRET_TARGET_TOKEN".into(),
                "target-secret-value".into(),
            ),
        ],
        cpu: Some("1.5".into()),
        memory: Some("1g".into()),
    }
}

/// Arguments every launch of the sample assignment must contain.
const EXPECTED_ARGS: [&str; 11] = [
    "run",
    "--detach",
    "--name=saci-run-abc123",
    "--label=io.saci.managed=true",
    "--label=io.saci.run=run-abc123",
    "--network=saci",
    "--add-host=host.containers.internal:host-gateway",
    "--cpus=1.5",
    "--memory=1g",
    "SACI_WORKER_TOKEN",
    "SACI_SECRET_TARGET_TOKEN",
];

#[tokio::test]
async fn workers_start_with_a_stable_name_and_values_only_in_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    fake_runtime(dir.path(), "podman", false);
    let launcher =
        ContainerLauncher::resolve(Runtime::Auto, dir.path().as_os_str(), &worker_config())
            .unwrap();
    launcher.launch(&sample_assignment()).await.unwrap();
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    for expected in EXPECTED_ARGS {
        assert!(args.contains(&expected), "{expected} missing from {args:?}");
    }
    assert_eq!(
        args.last(),
        Some(&"localhost/saci-worker:test"),
        "the image comes last"
    );
    let joined = args.join(" ");
    assert!(!joined.contains("worker-token-value") && !joined.contains("target-secret-value"));
    assert!(!args.iter().any(|arg| arg.starts_with("-v")
        || arg.starts_with("--volume")
        || arg.starts_with("--mount")));
    let env = std::fs::read_to_string(dir.path().join("env")).unwrap();
    assert!(env.contains("SACI_WORKER_TOKEN=worker-token-value"));
    assert!(env.contains("SACI_SECRET_TARGET_TOKEN=target-secret-value"));
}

#[tokio::test]
async fn state_listing_and_cleanup_of_missing_workers() {
    let dir = tempfile::tempdir().unwrap();
    fake_runtime(dir.path(), "podman", false);
    let launcher =
        ContainerLauncher::resolve(Runtime::Podman, dir.path().as_os_str(), &worker_config())
            .unwrap();
    let run = RunId::parse("run-abc123").unwrap();
    assert_eq!(launcher.state(&run).await.unwrap(), WorkerState::Exited(3));
    assert_eq!(
        launcher.workers().await.unwrap(),
        std::slice::from_ref(&run)
    );
    launcher.stop(&run, Duration::from_secs(1)).await.unwrap();
    launcher.remove(&run).await.unwrap();
    assert_eq!(worker_name(&run), "saci-run-abc123");
}

fn configured(yaml_secrets: &str) -> (Config, RunSpec) {
    let yaml = format!(
        "backend:\n  worker_url: http://host.containers.internal:8080\nllm:\n  base_url: http://m/v1\n  model: m\n  \
         api_key_env: MODEL_KEY\nenvironments:\n  local:\n    secrets:\n{yaml_secrets}"
    );
    let config = Config::from_yaml(&yaml, "saci.yaml").unwrap();
    let (environment, limits) = config.environment("local").unwrap();
    let spec = RunSpec {
        schema_version: saci_core::spec::SCHEMA_VERSION,
        run_id: RunId::parse("run-abc123").unwrap(),
        created_at: time::OffsetDateTime::UNIX_EPOCH,
        bundle: saci_core::plan::PlanBundle {
            entry: "plan.md".into(),
            documents: std::collections::BTreeMap::new(),
        },
        environment,
        policy: config.policy("default").unwrap(),
        limits,
        llm: config.llm.clone(),
    };
    (config, spec)
}

#[test]
fn assignments_carry_only_what_the_run_needs() {
    let (config, spec) = configured("      TARGET_TOKEN:\n        env: TOKEN_SOURCE\n");
    let lookup = |name: &str| match name {
        "MODEL_KEY" => Some("model-key".to_owned()),
        "TOKEN_SOURCE" => Some("target-secret".to_owned()),
        _ => None,
    };
    let assigned = assignment(&config, &spec, "worker-token", lookup).unwrap();
    let names: Vec<&str> = assigned
        .environment
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    let expected = ["SACI_BACKEND_URL", "SACI_RUN_ID", "SACI_WORKER_TOKEN"];
    assert_eq!(names[..3], expected);
    assert_eq!(names[3..], ["MODEL_KEY", "SACI_SECRET_TARGET_TOKEN"]);
    assert!(
        !format!("{assigned:?}").contains("target-secret"),
        "Debug hides values"
    );
}

#[test]
fn assignments_name_what_cannot_be_provided() {
    let (config, spec) = configured("      TARGET_TOKEN:\n        env: TOKEN_SOURCE\n");
    let missing = assignment(&config, &spec, "t", |name| {
        (name == "MODEL_KEY").then(|| "k".to_owned())
    })
    .unwrap_err();
    assert!(
        missing.message.contains("TOKEN_SOURCE"),
        "{}",
        missing.message
    );
    let (config, spec) =
        configured("      TARGET_TOKEN:\n        secret: target\n        key: token\n");
    let kubernetes = assignment(&config, &spec, "t", |_| Some("k".to_owned())).unwrap_err();
    assert!(
        kubernetes.message.contains("only the kubernetes runtime"),
        "{}",
        kubernetes.message
    );
    let mut unreachable = config;
    unreachable.backend.worker_url = None;
    let error = assignment(&unreachable, &spec, "t", |_| Some("k".to_owned())).unwrap_err();
    assert!(error.message.contains("backend.worker_url"));
}

/// A real worker container, launched by a real backend, runs a scripted
/// plan to its end.
#[tokio::test]
async fn the_backend_launches_the_real_image() {
    let Ok(image) = std::env::var("SACI_WORKER_IMAGE") else {
        eprintln!("SACI_WORKER_IMAGE is not set; skipping the real launch");
        return;
    };
    let runtime = std::env::var("SACI_CONTAINER_RUNTIME").unwrap_or_else(|_| "docker".into());
    let outcome = Box::pin(real::run(&image, &runtime)).await;
    assert_eq!(outcome, "passed");
}

mod real {
    use super::{Launcher, RunId};
    use saci_core::config::Config;
    use saci_testkit::{FakeModel, Reply, tool_calls_reply};
    use serde_json::{Value, json};

    const PLAN: &str = "# Test Plan: Launch\n\n## Phase 1: Main\n\n### Test 1.1: Echo\n\nRun `echo launched`.\n\n\
                        **Expected result:**\n\n- The output is `launched`.\n";

    fn script() -> Vec<Reply> {
        let shell = json!({ "command": "echo launched" }).to_string();
        let assessment = json!({
            "outcome": "passed",
            "criteria": [{"criterion": 0, "status": "satisfied", "basis": "mechanical",
                          "evidence": ["ev-1"], "explanation": "printed launched"}],
            "explanation": "it printed launched"
        })
        .to_string();
        vec![
            Reply::ok(&tool_calls_reply(&[("s", "shell", &shell)])),
            Reply::ok(&tool_calls_reply(&[("a", "assess", &assessment)])),
        ]
    }

    /// Runs the plan through a backend on the host network; returns the
    /// run's outcome.
    pub async fn run(image: &str, runtime: &str) -> String {
        let model = FakeModel::start(script()).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let yaml = config_yaml(dir.path(), address, image, runtime, &model.base_url());
        let config = Config::from_yaml(&yaml, "saci.yaml").unwrap();
        let launcher = Box::pin(Launcher::from_config(&config)).await.unwrap();
        spawn_backend(config, listener);
        let url = format!("http://{address}/api/v1");
        let run = Box::pin(submit(&url)).await;
        let summary = Box::pin(finished(&url, &run)).await;
        Box::pin(launcher.remove(&run)).await.unwrap();
        summary["outcome"].as_str().unwrap_or_default().to_owned()
    }

    fn spawn_backend(config: Config, listener: tokio::net::TcpListener) {
        let server = saci_backend::server::serve(config, listener, std::future::pending());
        tokio::spawn(Box::pin(server));
    }

    fn config_yaml(
        data: &std::path::Path,
        address: std::net::SocketAddr,
        image: &str,
        runtime: &str,
        model: &str,
    ) -> String {
        format!(
            "backend:\n  data_dir: {}\n  worker_url: http://{address}\nworker:\n  runtime: {runtime}\n  \
             image: {image}\n  network: host\nllm:\n  base_url: {model}\n  model: scripted\n\
             environments:\n  local: {{}}\n",
            data.display()
        )
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

    async fn finished(url: &str, run: &RunId) -> Value {
        let client = reqwest::Client::new();
        for _ in 0..120 {
            let summary: Value = client
                .get(format!("{url}/runs/{run}"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if summary["lifecycle"] == "finished" {
                return summary;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        panic!("the run did not finish");
    }
}
