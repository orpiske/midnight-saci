//! A backend on a temporary database with a recording supervisor, driven
//! in process.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use saci_backend::api::{Backend, ClientAuth, Supervisor};
use saci_backend::store::Store;
use saci_core::config::Config;
use saci_core::ids::RunId;
use serde_json::{Value, json};
use tower::ServiceExt;

pub const TOKEN: &str = "client-token-value";

pub const PLAN: &str = "# Test Plan: Service health

## Phase 1: Main

### Test 1.1: Check health

Run `curl $URL/health`.

**Expected result:**

- The status is 200.

### Step 1.2: Clean up

Remove the data.

**Expected result:**

- Nothing remains.
";

const CONFIG: &str = "llm:
  base_url: http://model:11434/v1
  model: test-model
environments:
  local:
    parameters:
      URL: http://service
    secrets:
      TARGET_TOKEN:
        env: TARGET_TOKEN_SOURCE
";

/// What the API asked the supervisor to do.
#[derive(Default)]
pub struct Recording {
    pub submitted: Mutex<Vec<RunId>>,
    pub cancelled: Mutex<Vec<RunId>>,
}

impl Supervisor for Recording {
    fn submitted(&self, run: RunId) {
        self.submitted.lock().unwrap().push(run);
    }

    fn cancel_requested(&self, run: RunId) {
        self.cancelled.lock().unwrap().push(run);
    }
}

pub struct TestBackend {
    pub backend: Backend,
    pub app: Router,
    pub supervisor: Arc<Recording>,
    _dir: tempfile::TempDir,
}

pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl TestBackend {
    pub async fn start(auth: ClientAuth) -> Self {
        Self::start_with(auth, CONFIG).await
    }

    /// A backend with the configuration `yaml`.
    pub async fn start_with(auth: ClientAuth, yaml: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let config = Config::from_yaml(yaml, "test.yaml").unwrap_or_else(|e| panic!("{e}"));
        let supervisor = Arc::new(Recording::default());
        let backend = Backend::new(store, config, supervisor.clone());
        let app = saci_backend::api::router(backend.clone(), auth);
        Self {
            backend,
            app,
            supervisor,
            _dir: dir,
        }
    }

    pub async fn call(
        &self,
        method: Method,
        uri: &str,
        body: Option<&Value>,
        headers: &[(&str, &str)],
    ) -> Answer {
        let mut request = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        let request = request
            .header("content-type", "application/json")
            .body(body)
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer {
            status,
            headers,
            body,
        }
    }

    pub async fn get(&self, uri: &str) -> Answer {
        self.call(Method::GET, uri, None, &[]).await
    }

    pub async fn post(&self, uri: &str, body: &Value) -> Answer {
        self.call(Method::POST, uri, Some(body), &[]).await
    }

    /// Submits the test plan and returns the new run's ID.
    pub async fn submit(&self) -> RunId {
        self.submit_plan(PLAN).await
    }

    /// Submits `plan` and returns the new run's ID.
    pub async fn submit_plan(&self, plan: &str) -> RunId {
        let answer = self.post("/api/v1/runs", &submission(plan)).await;
        assert_eq!(answer.status, StatusCode::ACCEPTED, "{}", answer.body);
        RunId::parse(answer.body["id"].as_str().unwrap()).unwrap()
    }
}

pub fn submission(plan: &str) -> Value {
    json!({
        "bundle": {"entry": "plan.md", "documents": BTreeMap::from([("plan.md", plan)])},
        "environment": "local"
    })
}
