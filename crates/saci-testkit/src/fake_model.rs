//! A scripted OpenAI-compatible Chat Completions server.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

/// One request received by the fake model.
#[derive(Debug, Clone)]
pub struct Recorded {
    /// HTTP method.
    pub method: String,
    /// Request path, for example `/v1/chat/completions`.
    pub path: String,
    /// Request headers.
    pub headers: HeaderMap,
    /// Parsed JSON body, or `Null`.
    pub body: Value,
}

/// One scripted reply.
#[derive(Debug, Clone)]
pub struct Reply {
    status: u16,
    body: String,
    delay: Option<Duration>,
}

impl Reply {
    /// A 200 reply with a JSON body.
    #[must_use]
    pub fn ok(body: &Value) -> Self {
        Self::raw(200, body.to_string())
    }

    /// A reply with any status and raw body.
    #[must_use]
    pub fn raw(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            delay: None,
        }
    }

    /// Delays the reply.
    #[must_use]
    pub const fn delayed(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

#[derive(Clone, Default)]
struct Shared {
    requests: Arc<Mutex<Vec<Recorded>>>,
    script: Arc<Mutex<VecDeque<Reply>>>,
}

/// A running fake model. Its base URL ends in `/v1`.
pub struct FakeModel {
    address: SocketAddr,
    shared: Shared,
}

impl FakeModel {
    /// Starts a server on an ephemeral loopback port that answers with
    /// `script` in order, and with HTTP 500 once the script is exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if the listener cannot be bound.
    pub async fn start(script: Vec<Reply>) -> std::io::Result<Self> {
        Self::bind("127.0.0.1:0", script).await
    }

    /// Like [`FakeModel::start`] on a chosen address.
    ///
    /// # Errors
    ///
    /// Returns an error if the listener cannot be bound.
    pub async fn bind(address: &str, script: Vec<Reply>) -> std::io::Result<Self> {
        let shared = Shared {
            requests: Arc::default(),
            script: Arc::new(Mutex::new(script.into())),
        };
        let state = shared.clone();
        let app = Router::new().fallback(move |method, uri, headers, body| {
            respond(state.clone(), method, uri, headers, body)
        });
        let listener = tokio::net::TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self { address, shared })
    }

    /// Base URL including the `/v1` prefix.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.address)
    }

    /// Requests received so far.
    #[must_use]
    pub fn requests(&self) -> Vec<Recorded> {
        lock(&self.shared.requests).clone()
    }

    /// Appends replies to the script.
    pub fn push(&self, replies: impl IntoIterator<Item = Reply>) {
        lock(&self.shared.script).extend(replies);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn respond(
    state: Shared,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    lock(&state.requests).push(Recorded {
        method: method.to_string(),
        path: uri.path().to_owned(),
        headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    let next = lock(&state.script).pop_front();
    let Some(reply) = next else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "fake model script exhausted",
        )
            .into_response();
    };
    if let Some(delay) = reply.delay {
        tokio::time::sleep(delay).await;
    }
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = Response::new(Body::from(reply.body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        "content-type",
        axum::http::HeaderValue::from_static("application/json"),
    );
    response
}

/// A Chat Completions reply with assistant text.
#[must_use]
pub fn text_reply(text: &str) -> Value {
    json!({
        "id": "chatcmpl-fake", "object": "chat.completion", "created": 1, "model": "fake-model",
        "choices": [{"index": 0, "finish_reason": "stop",
            "message": {"role": "assistant", "content": text}}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })
}

/// A Chat Completions reply with tool calls given as `(id, name, raw JSON arguments)`.
#[must_use]
pub fn tool_calls_reply(calls: &[(&str, &str, &str)]) -> Value {
    let calls: Vec<Value> = calls
        .iter()
        .map(|(id, name, arguments)| {
            json!({"id": id, "type": "function", "function": {"name": name, "arguments": arguments}})
        })
        .collect();
    json!({
        "id": "chatcmpl-fake", "object": "chat.completion", "created": 1, "model": "fake-model",
        "choices": [{"index": 0, "finish_reason": "tool_calls",
            "message": {"role": "assistant", "content": null, "tool_calls": calls}}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })
}
