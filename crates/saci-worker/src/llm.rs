//! The configured model connection: an OpenAI-compatible Chat Completions
//! endpoint reached through Rig, with bounded retries and actionable errors.
//!
//! Rig sends each model request exactly once, so [`RetryingClient`] wraps the
//! HTTP transport and repeats only the model request. Tool calls are executed
//! by the agent loop after a successful response and are never replayed.

use std::time::Duration;

use bytes::Bytes;
use rig::completion::PromptError;
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, ReqwestClient, Response, StatusCode,
    StreamingResponse,
};
use rig::providers::openai::wire::{Auth, Dialect};
use rig::providers::openai::{OpenAI, OpenAIConfig};
use rig::wasm_compat::WasmCompatSend;
use rig::{ErrorKind as RigErrorKind, Model};
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::{ExecutionLimits, LlmSettings};

/// Any OpenAI-compatible server; the base URL always comes from configuration.
static GATEWAY: Dialect = Dialect::gateway("saci", "http://127.0.0.1/v1", "SACI_LLM_API_KEY");

/// Longest provider body quoted in an error message.
const BODY_EXCERPT: usize = 500;

/// The chat model type the agent is built on.
pub type ChatModel = Model<rig::providers::openai::wire::Chat>;

/// Delays between retried model requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// Delay before the first retry; doubled for each further retry.
    pub initial: Duration,
    /// Upper bound of any delay, including one requested by `Retry-After`.
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(20),
        }
    }
}

impl Backoff {
    /// Delay before retry number `retry` (0-based), honoring `Retry-After`.
    #[must_use]
    pub fn delay(&self, retry: u32, retry_after: Option<Duration>) -> Duration {
        let exponential = self
            .initial
            .saturating_mul(2_u32.saturating_pow(retry.min(16)));
        retry_after.unwrap_or(exponential).min(self.max)
    }
}

/// A connection to the configured model.
#[derive(Clone)]
pub struct LlmConnection {
    model: ChatModel,
    model_name: String,
    api_key: Option<String>,
    max_turns: usize,
}

impl std::fmt::Debug for LlmConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConnection")
            .field("model", &self.model_name)
            .field("authenticated", &self.api_key.is_some())
            .field("max_turns", &self.max_turns)
            .finish_non_exhaustive()
    }
}

impl LlmConnection {
    /// Connects with the default [`Backoff`]. `lookup` reads environment
    /// variables (`std::env::var` in production).
    ///
    /// # Errors
    ///
    /// Fails when `api_key_env` is configured but not set, or when the HTTP
    /// client cannot be built.
    pub fn new(
        settings: &LlmSettings,
        limits: &ExecutionLimits,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ExecutionError> {
        Self::with_backoff(settings, limits, lookup, Backoff::default())
    }

    /// Like [`LlmConnection::new`] with explicit retry delays.
    ///
    /// # Errors
    ///
    /// See [`LlmConnection::new`].
    pub fn with_backoff(
        settings: &LlmSettings,
        limits: &ExecutionLimits,
        lookup: impl Fn(&str) -> Option<String>,
        backoff: Backoff,
    ) -> Result<Self, ExecutionError> {
        let api_key = api_key(settings, lookup)?;
        let http = rig::rig_reqwest::reqwest::Client::builder()
            .timeout(Duration::from_secs(limits.model_request_timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| invalid(client_build_error(&error)))?;
        let transport = RetryingClient {
            inner: ReqwestClient::from(http),
            retries: limits.model_request_retries,
            backoff,
        };
        let client: OpenAI = OpenAIConfig::with_key(&GATEWAY, api_key.clone().unwrap_or_default())
            .with_auth(Auth::OptionalBearer)
            .with_base_url(normalize_base_url(&settings.base_url))
            .connect(transport);
        Ok(Self {
            model: client.chat(settings.model.clone()),
            model_name: settings.model.clone(),
            api_key,
            max_turns: usize::try_from(limits.model_calls_per_step).unwrap_or(usize::MAX),
        })
    }

    /// The chat model to build agents on.
    #[must_use]
    pub fn model(&self) -> ChatModel {
        self.model.clone()
    }

    /// The configured model identifier.
    #[must_use]
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Model requests allowed per attempt; pass to `max_turns`, since Rig
    /// otherwise allows a single request.
    #[must_use]
    pub const fn max_turns(&self) -> usize {
        self.max_turns
    }

    /// Converts a failed agent run into an actionable, redacted error.
    #[must_use]
    pub fn classify(&self, error: &PromptError) -> ExecutionError {
        let mut classified = classify(error);
        if let Some(key) = &self.api_key
            && !key.is_empty()
        {
            classified.message = classified.message.replace(key.as_str(), "[REDACTED]");
        }
        classified
    }
}

/// Why an HTTP client could not be built, with every cause (for example a
/// missing CA certificate store), since reqwest's own message is only
/// "builder error".
pub(crate) fn client_build_error(error: &rig::rig_reqwest::reqwest::Error) -> String {
    let mut message = format!("cannot build the HTTP client: {error}");
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    message
}

fn invalid(message: String) -> ExecutionError {
    ExecutionError::new(ErrorKind::InvalidSpecification, message)
}

fn api_key(
    settings: &LlmSettings,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Option<String>, ExecutionError> {
    let Some(variable) = &settings.api_key_env else {
        return Ok(None);
    };
    match lookup(variable) {
        Some(key) if !key.is_empty() => Ok(Some(key)),
        _ => Err(invalid(format!(
            "llm.api_key_env names `{variable}`, which is not set in the worker environment"
        ))),
    }
}

/// Trims trailing slashes and a trailing `chat/completions`, which Rig
/// appends itself.
#[must_use]
pub fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    trimmed
        .strip_suffix("/chat/completions")
        .unwrap_or(trimmed)
        .trim_end_matches('/')
        .to_owned()
}

/// Maps a Rig run failure to Saci's error categories.
#[must_use]
pub fn classify(error: &PromptError) -> ExecutionError {
    match error {
        PromptError::MaxTurns { max_turns, .. } => ExecutionError::new(
            ErrorKind::LimitExceeded,
            format!("the attempt used all {max_turns} model requests allowed per step"),
        ),
        PromptError::Report(report) => {
            let body = error.provider_response_body().unwrap_or(&report.message);
            let message = match (report.http_status, report.kind) {
                (Some(status), _) => status_message(status, body),
                (None, RigErrorKind::Json | RigErrorKind::Response) => format!(
                    "the model provider returned a malformed response: {}",
                    excerpt(&report.message)
                ),
                (None, RigErrorKind::Http | RigErrorKind::Timeout) => transport_message(report),
                _ => format!("the model request failed: {}", excerpt(&report.message)),
            };
            ExecutionError::new(ErrorKind::Provider, message)
        }
        other => ExecutionError::new(
            ErrorKind::Provider,
            format!("the model run failed: {}", excerpt(&other.to_string())),
        ),
    }
}

fn status_message(status: u16, body: &str) -> String {
    let detail = excerpt(body);
    let lower = body.to_ascii_lowercase();
    match status {
        401 | 403 => format!(
            "the model provider rejected the credentials (HTTP {status}); check llm.api_key_env: {detail}"
        ),
        402 => format!("the model provider quota or credit is exhausted (HTTP 402): {detail}"),
        429 if is_quota(&lower) => {
            format!("the model provider quota is exhausted (HTTP 429): {detail}")
        }
        429 => format!(
            "the model provider is rate limiting requests (HTTP 429) after all retries: {detail}"
        ),
        404 => format!(
            "the model endpoint or model was not found (HTTP 404); check llm.base_url and llm.model: {detail}"
        ),
        400 | 422 if lower.contains("tool") || lower.contains("function") => format!(
            "the model or provider does not support tool calling (HTTP {status}); choose a tool-capable model: {detail}"
        ),
        500..=599 => {
            format!("the model provider is unavailable (HTTP {status}) after all retries: {detail}")
        }
        _ => format!("the model provider returned HTTP {status}: {detail}"),
    }
}

fn transport_message(report: &rig::ErrorReport) -> String {
    let chain = report.source_chain.join(": ");
    let all = format!("{} {chain}", report.message).to_ascii_lowercase();
    if all.contains("timed out") || all.contains("timeout") {
        "the model request timed out; raise limits.model_request_timeout_secs or check the provider"
            .to_owned()
    } else {
        format!(
            "cannot reach the model provider; check llm.base_url is reachable from the worker: {}",
            excerpt(&format!("{} {chain}", report.message))
        )
    }
}

fn is_quota(lower_body: &str) -> bool {
    ["insufficient_quota", "quota", "credit", "billing"]
        .iter()
        .any(|word| lower_body.contains(word))
}

fn excerpt(text: &str) -> String {
    let text = text.trim();
    if text.len() <= BODY_EXCERPT {
        return text.to_owned();
    }
    let mut end = BODY_EXCERPT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// An HTTP transport that retries transient model request failures.
#[derive(Clone)]
pub struct RetryingClient {
    inner: ReqwestClient,
    retries: u32,
    backoff: Backoff,
}

impl RetryingClient {
    /// Wraps `inner`, retrying up to `retries` times.
    #[must_use]
    pub const fn new(inner: ReqwestClient, retries: u32, backoff: Backoff) -> Self {
        Self {
            inner,
            retries,
            backoff,
        }
    }
}

/// Whether a failed request may be repeated, and how long to wait first.
fn retry_decision(error: &http_client::Error) -> Option<Option<Duration>> {
    if let Some(status) = error.non_success_status() {
        let quota = error
            .non_success_body()
            .is_some_and(|body| is_quota(&body.to_ascii_lowercase()));
        let transient = matches!(status.as_u16(), 500 | 502 | 503 | 504)
            || (status == StatusCode::TOO_MANY_REQUESTS && !quota);
        return transient.then(|| retry_after(error));
    }
    is_connect_error(error).then_some(None)
}

fn retry_after(error: &http_client::Error) -> Option<Duration> {
    let seconds = error
        .non_success_headers()?
        .get(http::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds))
}

fn is_connect_error(error: &http_client::Error) -> bool {
    reqwest_error(error).is_some_and(rig::rig_reqwest::reqwest::Error::is_connect)
}

fn reqwest_error(error: &http_client::Error) -> Option<&rig::rig_reqwest::reqwest::Error> {
    let http_client::Error::Instance(inner) = error else {
        return None;
    };
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(inner.as_ref());
    while let Some(current) = source {
        if let Some(reqwest) = current.downcast_ref::<rig::rig_reqwest::reqwest::Error>() {
            return Some(reqwest);
        }
        source = current.source();
    }
    None
}

/// Replaces reqwest's generic "error sending request" for a deadline, so the
/// failure reaching the agent says what happened.
#[derive(Debug)]
struct TimedOut;

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the model request timed out")
    }
}

impl std::error::Error for TimedOut {}

fn describe(error: http_client::Error) -> http_client::Error {
    if reqwest_error(&error).is_some_and(rig::rig_reqwest::reqwest::Error::is_timeout) {
        http_client::Error::instance(TimedOut)
    } else {
        error
    }
}

fn rebuild(parts: &http::request::Parts, body: &Bytes) -> Result<Request<Bytes>, http::Error> {
    let mut builder = Request::builder()
        .method(parts.method.clone())
        .uri(parts.uri.clone())
        .version(parts.version);
    if let Some(headers) = builder.headers_mut() {
        headers.clone_from(&parts.headers);
    }
    builder.body(body.clone())
}

impl HttpClientExt for RetryingClient {
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let client = self.clone();
        let (parts, body) = req.into_parts();
        let body: Bytes = body.into();
        Box::pin(async move {
            let mut retry = 0;
            loop {
                let request = rebuild(&parts, &body).map_err(http_client::Error::Protocol)?;
                let error = match Box::pin(client.inner.send::<Bytes, U>(request)).await {
                    Ok(response) => return Ok(response),
                    Err(error) => error,
                };
                let Some(wait) = retry_decision(&error).filter(|_| retry < client.retries) else {
                    return Err(describe(error));
                };
                let delay = client.backoff.delay(retry, wait);
                tracing::warn!(retry = retry + 1, ?delay, %error, "retrying model request");
                tokio::time::sleep(delay).await;
                retry += 1;
            }
        })
    }

    fn send_multipart<U>(
        &self,
        req: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.inner.send_multipart(req)
    }

    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        self.inner.send_streaming(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls_are_normalized_once() {
        assert_eq!(normalize_base_url("http://h/v1/"), "http://h/v1");
        assert_eq!(normalize_base_url("http://h/api/v1"), "http://h/api/v1");
        assert_eq!(
            normalize_base_url("http://h/v1/chat/completions/"),
            "http://h/v1"
        );
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let backoff = Backoff::default();
        assert_eq!(backoff.delay(0, None), Duration::from_secs(1));
        assert_eq!(backoff.delay(2, None), Duration::from_secs(4));
        assert_eq!(backoff.delay(10, None), Duration::from_secs(20));
        let honored = backoff.delay(0, Some(Duration::from_secs(7)));
        assert_eq!(honored, Duration::from_secs(7));
        let capped = backoff.delay(0, Some(Duration::from_secs(600)));
        assert_eq!(capped, Duration::from_secs(20));
    }

    #[test]
    fn long_bodies_are_truncated_on_char_boundaries() {
        let text = "é".repeat(400);
        let short = excerpt(&text);
        assert!(short.ends_with('…'));
        assert!(short.len() <= BODY_EXCERPT + '…'.len_utf8());
    }
}
