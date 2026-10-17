//! Workers as Kubernetes Jobs in one namespace, driven through the
//! Kubernetes API (`docs/kubernetes.md`).
//!
//! The backend authenticates with its own service account. Workers run as
//! a separate one (`worker.kubernetes.service_account`), get no API token
//! unless that account is configured, and read their credentials from a
//! per-run Secret owned by their Job and from the Secrets the environment
//! names. A Job never retries its worker (`backoffLimit: 0`): a worker that
//! fails is recorded, not repeated, because its actions may already have
//! run.

use std::path::PathBuf;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use saci_core::config::WorkerConfig;
use saci_core::ids::RunId;
use serde_json::{Value, json};

use super::{Assignment, LaunchError, MANAGED_LABEL, RUN_LABEL, WorkerState, run_of, worker_name};

/// Where the in-cluster service account credentials are mounted.
const SERVICE_ACCOUNT: &str = "/var/run/secrets/kubernetes.io/serviceaccount";
/// How long one API request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Margin past the run timeout before Kubernetes ends a Job by itself,
/// should the backend not be there to stop it.
const DEADLINE_MARGIN_SECS: u64 = 300;
/// Variables passed as plain values; every other value travels in the
/// run's Secret.
const PLAIN: [&str; 2] = ["SACI_BACKEND_URL", "SACI_RUN_ID"];

/// Launches workers as Jobs.
#[derive(Debug, Clone)]
pub struct KubernetesLauncher {
    api: Api,
    image: String,
    pull_policy: Option<String>,
    service_account: Option<String>,
}

/// A Kubernetes API endpoint and the namespace the backend works in.
#[derive(Debug, Clone)]
struct Api {
    base: String,
    namespace: String,
    token_file: Option<PathBuf>,
    client: reqwest::Client,
}

impl KubernetesLauncher {
    /// Connects with the configured API URL (`worker.kubernetes.api_url`,
    /// for example `kubectl proxy`), or with the pod's service account when
    /// the backend runs in the cluster.
    ///
    /// # Errors
    ///
    /// Fails when neither is available or the namespace is unknown.
    pub fn from_config(worker: &WorkerConfig) -> Result<Self, LaunchError> {
        let settings = &worker.kubernetes;
        let api = match &settings.api_url {
            Some(url) => Api::external(url, settings.namespace.clone())?,
            None => Api::in_cluster(settings.namespace.clone())?,
        };
        Ok(Self {
            api,
            image: worker.image.clone(),
            pull_policy: settings.image_pull_policy.clone(),
            service_account: settings.service_account.clone(),
        })
    }

    /// The namespace workers run in.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.api.namespace
    }

    /// Checks that the backend may list Jobs in its namespace.
    ///
    /// # Errors
    ///
    /// Names the API failure, for example missing RBAC permissions.
    pub async fn check(&self) -> Result<(), LaunchError> {
        let path = self.api.jobs("?limit=1");
        let listed = Box::pin(self.api.request(Method::GET, &path, None)).await;
        listed.map(drop).map_err(|error| error.launch)
    }

    /// The Job manifest of `assignment`.
    #[must_use]
    pub fn job(&self, assignment: &Assignment) -> Value {
        let name = worker_name(&assignment.run);
        let labels = labels(&assignment.run);
        let deadline = assignment.run_timeout.as_secs() + DEADLINE_MARGIN_SECS;
        json!({
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": {"name": name, "labels": labels},
            "spec": {
                "backoffLimit": 0,
                "activeDeadlineSeconds": deadline,
                "template": {
                    "metadata": {"labels": labels},
                    "spec": self.pod(assignment, &name),
                },
            },
        })
    }

    fn pod(&self, assignment: &Assignment, name: &str) -> Value {
        let mut container = json!({
            "name": "worker",
            "image": self.image,
            "env": environment(assignment, name),
            "securityContext": {
                "allowPrivilegeEscalation": false,
                "capabilities": {"drop": ["ALL"]},
                "runAsNonRoot": true,
                "seccompProfile": {"type": "RuntimeDefault"},
            },
            "resources": resources(assignment),
        });
        if let Some(policy) = &self.pull_policy {
            container["imagePullPolicy"] = json!(policy);
        }
        let mut pod = json!({
            "restartPolicy": "Never",
            "automountServiceAccountToken": self.service_account.is_some(),
            "enableServiceLinks": false,
            "containers": [container],
        });
        if let Some(account) = &self.service_account {
            pod["serviceAccountName"] = json!(account);
        }
        pod
    }

    /// Creates the run's Job, then its Secret, owned by the Job so that
    /// deleting the Job removes it. The pod waits for the Secret.
    ///
    /// # Errors
    ///
    /// A [`LaunchError`]; `uncertain` when the Job may exist anyway.
    pub async fn launch(&self, assignment: &Assignment) -> Result<(), LaunchError> {
        let path = self.api.jobs("");
        let created = self
            .api
            .request(Method::POST, &path, Some(&self.job(assignment)))
            .await;
        let job = created.map_err(|error| match error.status {
            Some(StatusCode::CONFLICT) => LaunchError::new(format!(
                "the Job {} is already in use",
                worker_name(&assignment.run)
            )),
            _ => error.launch,
        })?;
        let secret = secret(assignment, &job);
        let path = self.api.path("api/v1", "secrets", "");
        match self.api.request(Method::POST, &path, Some(&secret)).await {
            Ok(_) => Ok(()),
            Err(error) if error.status == Some(StatusCode::CONFLICT) => Ok(()),
            Err(error) => Err(LaunchError::uncertain(format!(
                "the Job was created but its Secret was not: {}",
                error.launch.message
            ))),
        }
    }

    /// The state of `run`'s Job.
    ///
    /// # Errors
    ///
    /// Fails when the API cannot be asked.
    pub async fn state(&self, run: &RunId) -> Result<WorkerState, LaunchError> {
        let path = self.api.jobs(&format!("/{}", worker_name(run)));
        let job = match self.api.request(Method::GET, &path, None).await {
            Ok(job) => job,
            Err(error) if error.status == Some(StatusCode::NOT_FOUND) => {
                return Ok(WorkerState::Missing);
            }
            Err(error) => return Err(error.launch),
        };
        let pod = self.pod_of(run).await?;
        Ok(job_state(&job, pod.as_ref()))
    }

    /// Stops `run`'s worker: its pod gets SIGTERM, then SIGKILL after
    /// `grace`.
    ///
    /// # Errors
    ///
    /// Fails when the API refuses.
    pub async fn stop(&self, run: &RunId, grace: Duration) -> Result<(), LaunchError> {
        let Some(pod) = self.pod_of(run).await? else {
            return Ok(());
        };
        let name = pod["metadata"]["name"].as_str().unwrap_or_default();
        let path = self.api.path("api/v1", "pods", &format!("/{name}"));
        let body = json!({"kind": "DeleteOptions", "apiVersion": "v1", "gracePeriodSeconds": grace.as_secs()});
        self.api
            .tolerate_missing(Method::DELETE, &path, Some(&body))
            .await
    }

    /// The last lines of `run`'s worker output.
    pub async fn logs(&self, run: &RunId) -> String {
        let Ok(Some(pod)) = self.pod_of(run).await else {
            return String::new();
        };
        let name = pod["metadata"]["name"].as_str().unwrap_or_default();
        let path = self
            .api
            .path("api/v1", "pods", &format!("/{name}/log?tailLines=40"));
        let mut text = self.api.text(&path).await.unwrap_or_default();
        if let Some(waiting) = waiting_reason(&pod) {
            text.push_str(&waiting);
        }
        text
    }

    /// Removes `run`'s Job; Kubernetes then removes its pod and Secret.
    ///
    /// # Errors
    ///
    /// Fails when the API refuses.
    pub async fn remove(&self, run: &RunId) -> Result<(), LaunchError> {
        let path = self.api.jobs(&format!("/{}", worker_name(run)));
        let body =
            json!({"kind": "DeleteOptions", "apiVersion": "v1", "propagationPolicy": "Background"});
        self.api
            .tolerate_missing(Method::DELETE, &path, Some(&body))
            .await
    }

    /// Runs whose Jobs exist.
    ///
    /// # Errors
    ///
    /// Fails when the API cannot be asked.
    pub async fn workers(&self) -> Result<Vec<RunId>, LaunchError> {
        let path = self
            .api
            .jobs(&format!("?labelSelector={MANAGED_LABEL}%3Dtrue"));
        let jobs = self
            .api
            .request(Method::GET, &path, None)
            .await
            .map_err(|e| e.launch)?;
        Ok(items(&jobs)
            .iter()
            .filter_map(|job| run_of(job["metadata"]["name"].as_str()?))
            .collect())
    }

    /// The pod of `run`'s Job, if one exists.
    async fn pod_of(&self, run: &RunId) -> Result<Option<Value>, LaunchError> {
        let selector = format!("?labelSelector={RUN_LABEL}%3D{run}");
        let path = self.api.path("api/v1", "pods", &selector);
        let pods = self
            .api
            .request(Method::GET, &path, None)
            .await
            .map_err(|e| e.launch)?;
        Ok(items(&pods).into_iter().next())
    }
}

fn labels(run: &RunId) -> Value {
    json!({ MANAGED_LABEL: "true", RUN_LABEL: run.to_string() })
}

fn items(list: &Value) -> Vec<Value> {
    list["items"].as_array().cloned().unwrap_or_default()
}

/// Plain values for the backend URL and run ID; every other variable from
/// the run's Secret or the Secret the environment names.
fn environment(assignment: &Assignment, secret: &str) -> Vec<Value> {
    let mut variables: Vec<Value> = assignment
        .environment
        .iter()
        .map(|(name, value)| {
            if PLAIN.contains(&name.as_str()) {
                json!({"name": name, "value": value})
            } else {
                let reference = json!({"name": secret, "key": name});
                json!({"name": name, "valueFrom": {"secretKeyRef": reference}})
            }
        })
        .collect();
    variables.extend(assignment.secret_refs.iter().map(|(name, key)| {
        let reference = json!({"name": key.name, "key": key.key});
        json!({"name": name, "valueFrom": {"secretKeyRef": reference}})
    }));
    variables
}

fn resources(assignment: &Assignment) -> Value {
    let mut limits = serde_json::Map::new();
    if let Some(cpu) = &assignment.cpu {
        limits.insert("cpu".into(), json!(cpu));
    }
    if let Some(memory) = &assignment.memory {
        limits.insert("memory".into(), json!(memory));
    }
    json!({"requests": limits, "limits": limits})
}

/// The run's Secret: every sensitive value, owned by `job`.
fn secret(assignment: &Assignment, job: &Value) -> Value {
    let data: serde_json::Map<String, Value> = assignment
        .environment
        .iter()
        .filter(|(name, _)| !PLAIN.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), json!(value)))
        .collect();
    let owner = json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "name": job["metadata"]["name"], "uid": job["metadata"]["uid"],
        "blockOwnerDeletion": true,
    });
    json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": worker_name(&assignment.run),
            "labels": labels(&assignment.run),
            "ownerReferences": [owner],
        },
        "type": "Opaque",
        "stringData": data,
    })
}

/// A Job's state, using its pod for the exit code.
#[must_use]
pub fn job_state(job: &Value, pod: Option<&Value>) -> WorkerState {
    let status = &job["status"];
    let ended = status["succeeded"].as_u64().unwrap_or(0) > 0
        || status["failed"].as_u64().unwrap_or(0) > 0
        || conditions(job).any(|kind| kind == "Complete" || kind == "Failed");
    let terminated = pod.and_then(|pod| {
        pod["status"]["containerStatuses"][0]["state"]["terminated"]["exitCode"].as_i64()
    });
    match (ended, terminated) {
        (_, Some(code)) => WorkerState::Exited(i32::try_from(code).unwrap_or(1)),
        (true, None) if status["succeeded"].as_u64().unwrap_or(0) > 0 => WorkerState::Exited(0),
        (true, None) => WorkerState::Exited(1),
        (false, None) => match pod.map(|pod| pod["status"]["phase"].as_str()) {
            Some(Some("Running")) => WorkerState::Running,
            _ => WorkerState::Starting,
        },
    }
}

fn conditions(job: &Value) -> impl Iterator<Item = &str> {
    job["status"]["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|condition| condition["status"] == "True")
        .filter_map(|condition| condition["type"].as_str())
}

/// Why the worker container has not started, for example an image that
/// cannot be pulled.
fn waiting_reason(pod: &Value) -> Option<String> {
    let waiting = &pod["status"]["containerStatuses"][0]["state"]["waiting"];
    let reason = waiting["reason"].as_str()?;
    let message = waiting["message"].as_str().unwrap_or_default();
    Some(format!(
        "\nthe worker container is waiting: {reason} {message}"
    ))
}

/// A failed API request.
struct ApiError {
    status: Option<StatusCode>,
    launch: LaunchError,
}

impl Api {
    fn external(url: &str, namespace: Option<String>) -> Result<Self, LaunchError> {
        let namespace = namespace.ok_or_else(|| {
            LaunchError::new(
                "worker.kubernetes.namespace is required with worker.kubernetes.api_url",
            )
        })?;
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| LaunchError::new(format!("cannot build the API client: {error}")))?;
        Ok(Self {
            base: url.trim_end_matches('/').to_owned(),
            namespace,
            token_file: None,
            client,
        })
    }

    fn in_cluster(namespace: Option<String>) -> Result<Self, LaunchError> {
        let (Ok(host), Ok(port)) = (
            std::env::var("KUBERNETES_SERVICE_HOST"),
            std::env::var("KUBERNETES_SERVICE_PORT"),
        ) else {
            return Err(LaunchError::new(
                "the backend is not running in a Kubernetes pod (KUBERNETES_SERVICE_HOST is not set); \
                 set worker.kubernetes.api_url, for example to a `kubectl proxy` URL",
            ));
        };
        let (namespace, client) = in_cluster_client(namespace)?;
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        Ok(Self {
            base: format!("https://{host}:{port}"),
            namespace,
            token_file: Some(PathBuf::from(SERVICE_ACCOUNT).join("token")),
            client,
        })
    }

    fn path(&self, group: &str, resource: &str, rest: &str) -> String {
        format!("/{group}/namespaces/{}/{resource}{rest}", self.namespace)
    }

    fn jobs(&self, rest: &str) -> String {
        self.path("apis/batch/v1", "jobs", rest)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<reqwest::Response, ApiError> {
        let mut request = self
            .client
            .request(method.clone(), format!("{}{path}", self.base));
        if let Some(file) = &self.token_file {
            // Re-read on every request: projected tokens are rotated.
            let token = std::fs::read_to_string(file).unwrap_or_default();
            request = request.bearer_auth(token.trim());
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        request.send().await.map_err(|error| ApiError {
            status: None,
            launch: LaunchError {
                message: format!("the Kubernetes API did not answer {method} {path}: {error}"),
                uncertain: method != Method::GET,
            },
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        let response = Box::pin(self.send(method.clone(), path, body)).await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
        }
        let reason = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|body| body["message"].as_str().map(str::to_owned))
            .unwrap_or(text);
        Err(ApiError {
            status: Some(status),
            launch: LaunchError::new(format!(
                "the Kubernetes API refused {method} {path} ({status}): {reason}{}",
                hint(status)
            )),
        })
    }

    async fn text(&self, path: &str) -> Option<String> {
        let response = Box::pin(self.send(Method::GET, path, None)).await.ok()?;
        response.status().is_success().then_some(())?;
        response.text().await.ok()
    }

    async fn tolerate_missing(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(), LaunchError> {
        match Box::pin(self.request(method, path, body)).await {
            Ok(_) => Ok(()),
            Err(error) if error.status == Some(StatusCode::NOT_FOUND) => Ok(()),
            Err(error) => Err(error.launch),
        }
    }
}

/// The namespace (configured or the pod's own) and a client trusting the
/// cluster's CA.
fn in_cluster_client(namespace: Option<String>) -> Result<(String, reqwest::Client), LaunchError> {
    let read = |name: &str| {
        std::fs::read(format!("{SERVICE_ACCOUNT}/{name}")).map_err(|error| {
            LaunchError::new(format!("cannot read the service account's {name}: {error}"))
        })
    };
    let namespace = match namespace {
        Some(namespace) => namespace,
        None => String::from_utf8_lossy(&read("namespace")?)
            .trim()
            .to_owned(),
    };
    let ca = reqwest::Certificate::from_pem(&read("ca.crt")?)
        .map_err(|error| LaunchError::new(format!("invalid cluster CA certificate: {error}")))?;
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .tls_certs_merge([ca])
        .build()
        .map_err(|error| LaunchError::new(format!("cannot build the API client: {error}")))?;
    Ok((namespace, client))
}

fn hint(status: StatusCode) -> &'static str {
    match status {
        StatusCode::UNAUTHORIZED => "; the backend's service account token was not accepted",
        StatusCode::FORBIDDEN => {
            "; grant the backend's service account the Role in docs/kubernetes.md#permissions"
        }
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkerState, job_state};
    use serde_json::json;

    #[test]
    fn job_states_follow_the_job_and_its_pod() {
        let active = json!({"status": {"active": 1}});
        assert_eq!(job_state(&active, None), WorkerState::Starting);
        let running = json!({"status": {"phase": "Running"}});
        assert_eq!(job_state(&active, Some(&running)), WorkerState::Running);
        let exited = json!({"status": {"phase": "Failed", "containerStatuses": [
            {"state": {"terminated": {"exitCode": 3}}}]}});
        assert_eq!(
            job_state(&json!({"status": {"failed": 1}}), Some(&exited)),
            WorkerState::Exited(3)
        );
        let deleted_pod = json!({"status": {"conditions": [{"type": "Failed", "status": "True"}]}});
        assert_eq!(job_state(&deleted_pod, None), WorkerState::Exited(1));
        assert_eq!(
            job_state(&json!({"status": {"succeeded": 1}}), None),
            WorkerState::Exited(0)
        );
    }
}
