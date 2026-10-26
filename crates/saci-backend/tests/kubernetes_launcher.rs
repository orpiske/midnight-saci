//! The Kubernetes Job launcher under the supervisor, against an in-memory
//! Kubernetes API: the Job and Secret a run gets, separate identities and
//! Secret references, no whole-worker retries, removal, cancellation,
//! failures, restart recovery and API errors.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use saci_backend::api::{Backend, ClientAuth, issue_token, router};
use saci_backend::launcher::{KubernetesLauncher, Launcher};
use saci_backend::store::Store;
use saci_backend::supervisor::{self, Timing};
use saci_core::config::Config;
use saci_core::event::{EventKind, RunEvent};
use saci_core::ids::RunId;
use saci_core::redaction::Redactor;
use saci_core::status::{Lifecycle, Outcome};
use saci_testkit::fake_kubernetes::FakeKubernetes;
use serde_json::{Value, json};

const PLAN: &str = "# Test Plan: In a cluster\n\n## Phase 1: Main\n\n### Test 1.1: Check\n\nRun `true`.\n\n\
                    **Expected result:**\n\n- It exits 0.\n";

const FAST: Timing = Timing {
    poll: Duration::from_millis(50),
    stop_grace: Duration::from_secs(7),
    deadline_grace: Duration::ZERO,
    exit_grace: Duration::from_millis(500),
};

fn config(api: &str) -> Config {
    let yaml = format!(
        "backend:\n  worker_url: http://saci:8080\nworker:\n  runtime: kubernetes\n  image: registry/saci-worker:1\n  \
         kubernetes:\n    api_url: {api}\n    namespace: tests\n    service_account: saci-worker\n    \
         image_pull_policy: IfNotPresent\nllm:\n  base_url: http://m/v1\n  model: m\n  api_key: env:MODEL_KEY\n  \
         api_key_secret:\n    name: saci-llm\n    key: api-key\npolicies:\n  default:\n    after_stop: skip\nenvironments:\n  local:\n    secrets:\n      \
         TARGET_TOKEN:\n        secret: target\n        key: token\n    worker:\n      cpu: 500m\n      memory: 512Mi\n"
    );
    Config::from_yaml(&yaml, "saci.yaml").unwrap()
}

struct Harness {
    cluster: FakeKubernetes,
    backend: Backend,
    url: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Self {
        let cluster = FakeKubernetes::start("tests").await.unwrap();
        Self::supervise(cluster, tempfile::tempdir().unwrap()).await
    }

    async fn supervise(cluster: FakeKubernetes, dir: tempfile::TempDir) -> Self {
        let config = config(&cluster.url());
        let store = Store::open(&dir.path().join("data")).await.unwrap();
        let launcher = Launcher::from_config(&config).await;
        let (commands, receiver) = supervisor::channel();
        let backend = Backend::new(store, config, Arc::new(commands));
        let supervision = supervisor::run(backend.clone(), launcher, receiver, FAST);
        tokio::spawn(Box::pin(supervision));
        let url = serve(&backend).await;
        Self {
            cluster,
            backend,
            url,
            _dir: dir,
        }
    }

    /// Waits for `run`'s Job to exist (`true`) or be gone (`false`).
    async fn job(&self, run: &RunId, wanted: bool) -> Option<Value> {
        let name = format!("saci-{run}");
        eventually(|| async { (self.cluster.job(&name).is_some() == wanted).then_some(()) }).await;
        self.cluster.job(&name)
    }

    async fn report(&self, run: &RunId, kinds: Vec<EventKind>) {
        let events = kinds
            .into_iter()
            .zip(1..)
            .map(|(kind, seq)| RunEvent {
                run_id: run.clone(),
                seq,
                at: time::OffsetDateTime::now_utc(),
                unit: None,
                attempt: None,
                source: None,
                kind,
            })
            .collect();
        let store = &self.backend.store;
        store
            .append_events(run, events, Redactor::new())
            .await
            .unwrap();
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

async fn serve(backend: &Backend) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let app = router(backend.clone(), ClientAuth::Open);
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

async fn submit(url: &str) -> RunId {
    let body = json!({"bundle": {"entry": "plan.md", "documents": {"plan.md": PLAN}}, "environment": "local"});
    let request = reqwest::Client::new()
        .post(format!("{url}/runs"))
        .json(&body);
    let reply: Value = request.send().await.unwrap().json().await.unwrap();
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

fn finished_events(reason: &str) -> Vec<EventKind> {
    vec![
        EventKind::RunStarted {
            model: "m".into(),
            environment: None,
        },
        EventKind::RunFinished {
            outcome: Outcome::Passed,
            reason: reason.into(),
        },
    ]
}

/// The variable named `name` in a Job's container.
fn variable<'a>(job: &'a Value, name: &str) -> &'a Value {
    let env = job["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array()
        .unwrap();
    env.iter()
        .find(|variable| variable["name"] == name)
        .unwrap()
}

#[tokio::test]
async fn a_run_gets_one_job_with_its_own_identity_and_no_retries() {
    let harness = Harness::start().await;
    let run = submit(&harness.url).await;
    let job = harness.job(&run, true).await.unwrap();
    assert_eq!(job["spec"]["backoffLimit"], 0);
    let pod = &job["spec"]["template"]["spec"];
    assert_eq!(pod["restartPolicy"], "Never");
    assert_eq!(pod["serviceAccountName"], "saci-worker");
    let container = &pod["containers"][0];
    assert_eq!(container["securityContext"]["runAsNonRoot"], true);
    assert!(
        container["securityContext"]["runAsUser"].is_null(),
        "OpenShift assigns the UID"
    );
    assert_eq!(
        container["resources"]["limits"],
        json!({"cpu": "500m", "memory": "512Mi"})
    );
    assert_eq!(job["metadata"]["labels"]["io.saci.run"], run.to_string());
}

#[tokio::test]
async fn credentials_come_from_the_run_s_secret_and_configured_secrets() {
    let harness = Harness::start().await;
    let run = submit(&harness.url).await;
    let job = harness.job(&run, true).await.unwrap();
    assert_eq!(
        variable(&job, "SACI_BACKEND_URL")["value"],
        "http://saci:8080"
    );
    let secret_name = format!("saci-{run}");
    let token = &variable(&job, "SACI_WORKER_TOKEN")["valueFrom"]["secretKeyRef"];
    assert_eq!(
        *token,
        json!({"name": secret_name, "key": "SACI_WORKER_TOKEN"})
    );
    let target = &variable(&job, "SACI_SECRET_TARGET_TOKEN")["valueFrom"]["secretKeyRef"];
    assert_eq!(*target, json!({"name": "target", "key": "token"}));
    let model = &variable(&job, "MODEL_KEY")["valueFrom"]["secretKeyRef"];
    assert_eq!(*model, json!({"name": "saci-llm", "key": "api-key"}));
    let secret = harness.cluster.secret(&secret_name).unwrap();
    assert_eq!(
        secret["metadata"]["ownerReferences"][0]["uid"],
        job["metadata"]["uid"]
    );
    assert!(secret["stringData"]["SACI_WORKER_TOKEN"].is_string());
}

#[tokio::test]
async fn a_finished_run_s_job_and_secret_are_removed() {
    let harness = Harness::start().await;
    let run = submit(&harness.url).await;
    harness.job(&run, true).await;
    harness
        .cluster
        .set_pod(&format!("saci-{run}"), "Running", None);
    harness.report(&run, finished_events("all passed")).await;
    harness
        .cluster
        .set_pod(&format!("saci-{run}"), "Succeeded", Some(0));
    harness.job(&run, false).await;
    assert!(harness.cluster.secret(&format!("saci-{run}")).is_none());
    assert_eq!(harness.finished(&run).await.1, "all passed");
}

#[tokio::test]
async fn cancellation_stops_the_pod_and_ends_the_run_cancelled() {
    let harness = Harness::start().await;
    let run = submit(&harness.url).await;
    harness.job(&run, true).await;
    harness
        .cluster
        .set_pod(&format!("saci-{run}"), "Running", None);
    let url = format!("{}/runs/{run}/cancellation", harness.url);
    let response = reqwest::Client::new().post(url).send().await.unwrap();
    assert_eq!(response.status(), 202);
    let (outcome, reason) = harness.finished(&run).await;
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(reason.contains("stopped before reporting"), "{reason}");
    let requests = harness.cluster.requests();
    let position = |prefix: &str| requests.iter().position(|line| line.starts_with(prefix));
    let suspended = position(&format!(
        "PATCH /apis/batch/v1/namespaces/tests/jobs/saci-{run}"
    ));
    let deleted = position("DELETE /api/v1/namespaces/tests/pods/").unwrap();
    assert!(suspended.unwrap() < deleted, "{requests:?}");
    assert!(requests[deleted].contains("\"gracePeriodSeconds\":7"));
    harness.job(&run, false).await;
    let created = requests
        .iter()
        .filter(|line| line.starts_with("POST /apis/batch"))
        .count();
    assert_eq!(created, 1, "{requests:?}");
}

#[tokio::test]
async fn a_failed_worker_ends_the_run_with_its_output_and_is_not_retried() {
    let harness = Harness::start().await;
    let run = submit(&harness.url).await;
    harness.job(&run, true).await;
    harness
        .cluster
        .set_pod(&format!("saci-{run}"), "Failed", Some(3));
    let (outcome, reason) = harness.finished(&run).await;
    assert_eq!(outcome, Outcome::Error);
    assert!(
        reason.contains("exited with code 3") && reason.contains("cannot reach the model"),
        "{reason}"
    );
    harness.job(&run, false).await;
    let creations = harness
        .cluster
        .requests()
        .iter()
        .filter(|line| line.starts_with("POST /apis/batch"))
        .count();
    assert_eq!(creations, 1);
}

/// An earlier backend process: one run whose Job still runs, one whose Job
/// is gone.
async fn earlier_backend(cluster: &FakeKubernetes, dir: &std::path::Path) -> [RunId; 2] {
    let store = Store::open(&dir.join("data")).await.unwrap();
    let (commands, _ignored) = supervisor::channel();
    let backend = Backend::new(store, config(&cluster.url()), Arc::new(commands));
    let url = serve(&backend).await;
    let (running, lost) = (submit(&url).await, submit(&url).await);
    for run in [&running, &lost] {
        issue_token(&backend, run).await.unwrap();
    }
    let labels = json!({"io.saci.managed": "true", "io.saci.run": running.to_string()});
    let job = json!({"metadata": {"name": format!("saci-{running}"), "labels": labels},
                     "spec": {"template": {"metadata": {"labels": labels}, "spec": {}}}});
    cluster.insert_job(job);
    [running, lost]
}

#[tokio::test]
async fn restart_recovery_watches_existing_jobs_and_never_repeats_runs() {
    let (cluster, dir) = (
        FakeKubernetes::start("tests").await.unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let [running, lost] = earlier_backend(&cluster, dir.path()).await;
    let harness = Harness::supervise(cluster, dir).await;
    let (outcome, reason) = harness.finished(&lost).await;
    assert_eq!(outcome, Outcome::Error);
    assert!(reason.contains("not repeated"), "{reason}");
    harness
        .report(&running, finished_events("done after the restart"))
        .await;
    harness
        .cluster
        .set_pod(&format!("saci-{running}"), "Succeeded", Some(0));
    harness.job(&running, false).await;
    assert_eq!(harness.finished(&running).await.1, "done after the restart");
    assert!(
        harness.cluster.job(&format!("saci-{lost}")).is_none(),
        "the lost run gets no new Job"
    );
}

#[tokio::test]
async fn api_refusals_say_what_to_change() {
    let cluster = FakeKubernetes::start("tests").await.unwrap();
    cluster.refuse(Some(403));
    let error = Launcher::from_config(&config(&cluster.url()))
        .await
        .unwrap_err();
    assert!(
        error.message.contains("403") && error.message.contains("Role"),
        "{}",
        error.message
    );
    let mut outside = config(&cluster.url());
    outside.worker.kubernetes.api_url = None;
    let error = KubernetesLauncher::from_config(&outside.worker).unwrap_err();
    assert!(
        error.message.contains("worker.kubernetes.api_url"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn a_second_launch_for_a_run_reports_the_existing_job() {
    let cluster = FakeKubernetes::start("tests").await.unwrap();
    let launcher = KubernetesLauncher::from_config(&config(&cluster.url()).worker).unwrap();
    let assignment = saci_backend::launcher::Assignment {
        run: RunId::parse("run-abc123").unwrap(),
        environment: vec![("SACI_WORKER_TOKEN".into(), "worker-token".into())],
        secret_refs: Vec::new(),
        cpu: None,
        memory: None,
        run_timeout: Duration::from_secs(60),
        image: None,
    };
    launcher.launch(&assignment).await.unwrap();
    let error = launcher.launch(&assignment).await.unwrap_err();
    assert!(
        error.message.contains("already in use") && !error.uncertain,
        "{}",
        error.message
    );
    let job = cluster.job("saci-run-abc123").unwrap();
    assert_eq!(job["spec"]["activeDeadlineSeconds"], 360);
    assert!(
        !job.to_string().contains("worker-token"),
        "values stay out of the Job"
    );
}

/// A real Job in a real cluster (CI's `kubernetes` job): set
/// `SACI_KUBERNETES_API` (a `kubectl proxy` URL), `SACI_KUBERNETES_NAMESPACE`,
/// `SACI_WORKER_IMAGE` (loaded into the cluster) and `SACI_HOST_ADDRESS`
/// (how pods reach this machine).
#[tokio::test]
async fn a_real_cluster_runs_and_cancels_jobs() {
    let Some(cluster) = real::Cluster::from_env() else {
        eprintln!("SACI_KUBERNETES_API is not set; skipping the real cluster");
        return;
    };
    let backend = Box::pin(cluster.start()).await;
    let passed = Box::pin(backend.submit(real::ECHO)).await;
    let summary = Box::pin(backend.finished(&passed)).await;
    assert_eq!(summary["outcome"], "passed", "{summary}");
    Box::pin(backend.removed(&passed)).await;
    let cancelled = Box::pin(backend.submit(real::SLEEP)).await;
    Box::pin(backend.until(&cancelled, |summary| {
        summary["last_event_seq"].as_u64() >= Some(2)
    }))
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    Box::pin(backend.cancel(&cancelled)).await;
    let summary = Box::pin(backend.finished(&cancelled)).await;
    assert_eq!(summary["outcome"], "cancelled", "{summary}");
    Box::pin(backend.removed(&cancelled)).await;
}

mod real {
    use saci_backend::launcher::{Launcher, WorkerState};
    use saci_core::config::Config;
    use saci_core::ids::RunId;
    use saci_testkit::{FakeModel, Reply, tool_calls_reply};
    use serde_json::{Value, json};

    pub const ECHO: &str = "# Test Plan: Echo\n\n## Phase 1: Main\n\n### Test 1.1: Echo\n\nRun `echo in-cluster`.\n\n\
                            **Expected result:**\n\n- The output is `in-cluster`.\n";
    pub const SLEEP: &str = "# Test Plan: Sleep\n\n## Phase 1: Main\n\n### Step 1: Sleep\n\nRun `sleep 120`.\n\n\
                             **Expected result:**\n\n- It exits 0.\n";

    pub struct Cluster {
        api: String,
        namespace: String,
        image: String,
        host: String,
    }

    impl Cluster {
        pub fn from_env() -> Option<Self> {
            let get = |name| std::env::var(name).ok();
            Some(Self {
                api: get("SACI_KUBERNETES_API")?,
                namespace: get("SACI_KUBERNETES_NAMESPACE").unwrap_or_else(|| "default".into()),
                image: get("SACI_WORKER_IMAGE")?,
                host: get("SACI_HOST_ADDRESS")?,
            })
        }

        /// A backend on this machine launching Jobs, with a model that
        /// echoes once, assesses, then sleeps for the cancellation.
        pub async fn start(&self) -> Backend {
            let model = FakeModel::bind("0.0.0.0:0", script()).await.unwrap();
            let model_url = model.base_url().replace("0.0.0.0", &self.host);
            let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let dir = tempfile::tempdir().unwrap();
            let yaml = format!(
                "backend:\n  data_dir: {}\n  worker_url: http://{}:{port}\nworker:\n  runtime: kubernetes\n  \
                 image: {}\n  kubernetes:\n    api_url: {}\n    namespace: {}\n    image_pull_policy: Never\n\
                 llm:\n  base_url: {model_url}\n  model: scripted\nenvironments:\n  local: {{}}\n",
                dir.path().display(),
                self.host,
                self.image,
                self.api,
                self.namespace
            );
            let config = Config::from_yaml(&yaml, "saci.yaml").unwrap();
            let launcher = Box::pin(Launcher::from_config(&config)).await.unwrap();
            let server = saci_backend::server::serve(config, listener, std::future::pending());
            tokio::spawn(Box::pin(server));
            let url = format!("http://127.0.0.1:{port}/api/v1");
            Backend {
                url,
                launcher,
                _model: model,
                _dir: dir,
            }
        }
    }

    fn script() -> Vec<Reply> {
        let echo = json!({"command": "echo in-cluster"}).to_string();
        let assessment = json!({
            "outcome": "passed",
            "criteria": [{"criterion": 0, "status": "satisfied", "basis": "mechanical",
                          "evidence": ["ev-1"], "explanation": "printed in-cluster"}],
            "explanation": "it printed in-cluster"
        })
        .to_string();
        let sleep = json!({"command": "sleep 120"}).to_string();
        vec![
            Reply::ok(&tool_calls_reply(&[("s", "shell", &echo)])),
            Reply::ok(&tool_calls_reply(&[("a", "assess", &assessment)])),
            Reply::ok(&tool_calls_reply(&[("s", "shell", &sleep)])),
        ]
    }

    pub struct Backend {
        url: String,
        launcher: Launcher,
        _model: FakeModel,
        _dir: tempfile::TempDir,
    }

    impl Backend {
        pub async fn submit(&self, plan: &str) -> RunId {
            let body = json!({"bundle": {"entry": "plan.md", "documents": {"plan.md": plan}}, "environment": "local"});
            let request = reqwest::Client::new()
                .post(format!("{}/runs", self.url))
                .json(&body);
            let reply: Value = request.send().await.unwrap().json().await.unwrap();
            RunId::parse(reply["id"].as_str().unwrap()).unwrap()
        }

        pub async fn cancel(&self, run: &RunId) {
            let url = format!("{}/runs/{run}/cancellation", self.url);
            reqwest::Client::new().post(url).send().await.unwrap();
        }

        pub async fn until(&self, run: &RunId, condition: impl Fn(&Value) -> bool) -> Value {
            for _ in 0..240 {
                let url = format!("{}/runs/{run}", self.url);
                let summary: Value = reqwest::get(url).await.unwrap().json().await.unwrap();
                if condition(&summary) {
                    return summary;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            panic!("the run never reached the expected state");
        }

        pub async fn finished(&self, run: &RunId) -> Value {
            self.until(run, |summary| summary["lifecycle"] == "finished")
                .await
        }

        pub async fn removed(&self, run: &RunId) {
            for _ in 0..240 {
                if self.launcher.state(run).await.unwrap() == WorkerState::Missing {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            panic!("the Job was not removed");
        }
    }
}
