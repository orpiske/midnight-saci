//! An in-memory Kubernetes API for one namespace, covering what the Job
//! launcher uses: Jobs, their pods, Secrets with owner references, pod
//! logs, label selectors and background deletion. Tests move pods through
//! their phases with [`FakeKubernetes::set_pod`].

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::Bytes;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

#[derive(Default)]
struct Cluster {
    jobs: BTreeMap<String, Value>,
    pods: BTreeMap<String, Value>,
    secrets: BTreeMap<String, Value>,
    requests: Vec<String>,
    refuse: Option<u16>,
    uids: u64,
}

type Shared = Arc<Mutex<Cluster>>;

/// A running fake API server.
pub struct FakeKubernetes {
    address: SocketAddr,
    namespace: String,
    cluster: Shared,
}

fn lock(cluster: &Shared) -> std::sync::MutexGuard<'_, Cluster> {
    cluster.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeKubernetes {
    /// Starts the server on an ephemeral loopback port for `namespace`.
    ///
    /// # Errors
    ///
    /// Returns an error if the listener cannot be bound.
    pub async fn start(namespace: &str) -> std::io::Result<Self> {
        let cluster = Shared::default();
        let (state, prefix) = (cluster.clone(), namespace.to_owned());
        let app = Router::new().fallback(move |method, uri, body| {
            let (state, prefix) = (state.clone(), prefix.clone());
            async move { handle(&state, &prefix, &method, &uri, &body) }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self {
            address,
            namespace: namespace.to_owned(),
            cluster,
        })
    }

    /// The API URL.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The namespace served.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Answers every following request with `status`, or normally again
    /// with `None`.
    pub fn refuse(&self, status: Option<u16>) {
        lock(&self.cluster).refuse = status;
    }

    /// The Job named `name`.
    #[must_use]
    pub fn job(&self, name: &str) -> Option<Value> {
        lock(&self.cluster).jobs.get(name).cloned()
    }

    /// The Secret named `name`.
    #[must_use]
    pub fn secret(&self, name: &str) -> Option<Value> {
        lock(&self.cluster).secrets.get(name).cloned()
    }

    /// The pod of Job `job`.
    #[must_use]
    pub fn pod(&self, job: &str) -> Option<Value> {
        lock(&self.cluster)
            .pods
            .values()
            .find(|pod| pod["metadata"]["labels"]["job-name"] == job)
            .cloned()
    }

    /// Requests received, as `METHOD path?query body`.
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        lock(&self.cluster).requests.clone()
    }

    /// Creates a Job as an earlier backend process would have, with a
    /// running pod.
    pub fn insert_job(&self, job: Value) {
        let mut cluster = lock(&self.cluster);
        let name = create_job(&mut cluster, job);
        drop(cluster);
        self.set_pod(&name, "Running", None);
    }

    /// Moves Job `job`'s pod to `phase` (`Running`, `Succeeded`, `Failed`),
    /// with the worker container's exit code once it has ended.
    pub fn set_pod(&self, job: &str, phase: &str, exit_code: Option<i32>) {
        let mut cluster = lock(&self.cluster);
        if let Some(pod) = cluster
            .pods
            .values_mut()
            .find(|pod| pod["metadata"]["labels"]["job-name"] == job)
        {
            pod["status"]["phase"] = json!(phase);
            if let Some(code) = exit_code {
                pod["status"]["containerStatuses"] =
                    json!([{"state": {"terminated": {"exitCode": code}}}]);
            }
        }
        let status = match phase {
            "Succeeded" => {
                json!({"succeeded": 1, "conditions": [{"type": "Complete", "status": "True"}]})
            }
            "Failed" => json!({"failed": 1, "conditions": [{"type": "Failed", "status": "True"}]}),
            _ => json!({"active": 1}),
        };
        if let Some(job) = cluster.jobs.get_mut(job) {
            job["status"] = status;
        }
    }
}

fn reply(status: StatusCode, body: &Value) -> Response {
    (status, axum::Json(body.clone())).into_response()
}

fn status(code: StatusCode, message: &str) -> Response {
    let body =
        json!({"kind": "Status", "status": "Failure", "message": message, "code": code.as_u16()});
    reply(code, &body)
}

fn handle(cluster: &Shared, namespace: &str, method: &Method, uri: &Uri, body: &Bytes) -> Response {
    let body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let mut cluster = lock(cluster);
    let query = uri.query().unwrap_or_default().to_owned();
    cluster
        .requests
        .push(format!("{method} {}?{query} {body}", uri.path()));
    if let Some(code) = cluster.refuse {
        let code = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return status(code, "refused by the test");
    }
    let prefixes = [
        (
            format!("/apis/batch/v1/namespaces/{namespace}/jobs"),
            "jobs",
        ),
        (format!("/api/v1/namespaces/{namespace}/pods"), "pods"),
        (format!("/api/v1/namespaces/{namespace}/secrets"), "secrets"),
    ];
    let Some((rest, kind)) = prefixes
        .iter()
        .find_map(|(prefix, kind)| Some((uri.path().strip_prefix(prefix.as_str())?, *kind)))
    else {
        return status(StatusCode::NOT_FOUND, "not served by the fake API");
    };
    let rest = rest.trim_start_matches('/').to_owned();
    route(&mut cluster, method, (kind, &rest, &query), body)
}

fn route(
    cluster: &mut Cluster,
    method: &Method,
    target: (&str, &str, &str),
    body: Value,
) -> Response {
    let (kind, rest, query) = target;
    match (method.as_str(), kind, rest) {
        ("GET", kind, "") => list(cluster, kind, query),
        ("POST", "jobs", "") if cluster.jobs.contains_key(name_of(&body)) => {
            status(StatusCode::CONFLICT, "jobs.batch already exists")
        }
        ("POST", "jobs", "") => {
            let name = create_job(cluster, body);
            reply(StatusCode::CREATED, &cluster.jobs[&name])
        }
        ("POST", "secrets", "") if cluster.secrets.contains_key(name_of(&body)) => {
            status(StatusCode::CONFLICT, "secrets already exists")
        }
        ("POST", "secrets", "") => {
            cluster
                .secrets
                .insert(name_of(&body).to_owned(), body.clone());
            reply(StatusCode::CREATED, &body)
        }
        ("GET", "jobs", name) => found(cluster.jobs.get(name)),
        ("GET", "pods", path) if path.ends_with("/log") => (
            StatusCode::OK,
            "worker: starting\nworker: cannot reach the model\n",
        )
            .into_response(),
        ("DELETE", "jobs", name) => delete_job(cluster, name),
        ("DELETE", "pods", name) => delete_pod(cluster, name),
        _ => status(StatusCode::NOT_FOUND, "not found"),
    }
}

fn found(item: Option<&Value>) -> Response {
    item.map_or_else(
        || status(StatusCode::NOT_FOUND, "not found"),
        |item| reply(StatusCode::OK, item),
    )
}

fn name_of(item: &Value) -> &str {
    item["metadata"]["name"].as_str().unwrap_or_default()
}

fn list(cluster: &Cluster, kind: &str, query: &str) -> Response {
    let selector = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("labelSelector="))
        .map(|selector| selector.replace("%3D", "="));
    let items = match kind {
        "jobs" => &cluster.jobs,
        "pods" => &cluster.pods,
        _ => &cluster.secrets,
    };
    let matching: Vec<&Value> = items
        .values()
        .filter(|item| {
            selector.as_ref().is_none_or(|selector| {
                let (key, value) = selector.split_once('=').unwrap_or((selector, ""));
                item["metadata"]["labels"][key] == value
            })
        })
        .collect();
    reply(StatusCode::OK, &json!({"items": matching}))
}

fn create_job(cluster: &mut Cluster, mut job: Value) -> String {
    cluster.uids += 1;
    let name = name_of(&job).to_owned();
    job["metadata"]["uid"] = json!(format!("uid-{}", cluster.uids));
    job["status"] = json!({"active": 1});
    let mut labels = job["spec"]["template"]["metadata"]["labels"].clone();
    labels["job-name"] = json!(name);
    let pod_name = format!("{name}-{}", cluster.uids);
    let pod = json!({
        "metadata": {"name": pod_name, "labels": labels},
        "spec": job["spec"]["template"]["spec"],
        "status": {"phase": "Pending"},
    });
    cluster.pods.insert(pod_name, pod);
    cluster.jobs.insert(name.clone(), job);
    name
}

fn delete_job(cluster: &mut Cluster, name: &str) -> Response {
    let Some(job) = cluster.jobs.remove(name) else {
        return status(StatusCode::NOT_FOUND, "not found");
    };
    let uid = job["metadata"]["uid"].clone();
    cluster
        .pods
        .retain(|_, pod| pod["metadata"]["labels"]["job-name"] != name);
    cluster
        .secrets
        .retain(|_, secret| secret["metadata"]["ownerReferences"][0]["uid"] != uid);
    reply(StatusCode::OK, &job)
}

/// Deleting a Job's pod fails the Job, as `backoffLimit: 0` does.
fn delete_pod(cluster: &mut Cluster, name: &str) -> Response {
    let Some(pod) = cluster.pods.remove(name) else {
        return status(StatusCode::NOT_FOUND, "not found");
    };
    if let Some(job) = pod["metadata"]["labels"]["job-name"]
        .as_str()
        .and_then(|job| cluster.jobs.get_mut(job))
    {
        job["status"] = json!({"failed": 1, "conditions": [{"type": "Failed", "status": "True"}]});
    }
    reply(StatusCode::OK, &pod)
}
