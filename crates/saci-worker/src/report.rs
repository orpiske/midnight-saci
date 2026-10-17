//! The worker side of the reporting protocol (`docs/api.md#worker-reporting-protocol`):
//! fetching the assigned specification, delivering events until the backend
//! acknowledges them, and watching for cancellation.

use std::collections::VecDeque;
use std::time::Duration;

use rig::rig_reqwest::reqwest::{self, Method, StatusCode};
use saci_core::api::{
    API_BASE, ErrorBody, ErrorCode, EventAck, EventBatch, MAX_EVENT_BATCH_BYTES, WorkerControl,
};
use saci_core::event::RunEvent;
use saci_core::ids::RunId;
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::RunSpec;
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

use crate::events::EventReceiver;
use crate::llm::Backoff;

/// Events sent in one request at most.
const BATCH: usize = 100;

/// Room left in a request for the batch's own JSON around its events.
const ENVELOPE: usize = 64;

/// How long the backend may stay unreachable before delivery gives up.
const UNREACHABLE: Duration = Duration::from_secs(300);

/// A failed exchange with the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    /// The backend could not be reached or failed (5xx); worth retrying.
    Transient(String),
    /// A batch skipped events; the backend holds everything up to this.
    Gap(u64),
    /// The backend refused the request; retrying cannot help.
    Rejected(ErrorCode, String),
}

impl ReportError {
    fn into_execution(self) -> ExecutionError {
        let message = match self {
            Self::Transient(message) => format!("the backend is unreachable: {message}"),
            Self::Gap(acknowledged) => {
                format!("the backend reported a gap after event {acknowledged}")
            }
            Self::Rejected(code, message) => {
                format!("the backend refused the request ({code:?}): {message}")
            }
        };
        ExecutionError::new(ErrorKind::WorkerLost, message)
    }
}

/// Talks to the backend with the run-scoped worker token.
#[derive(Clone)]
pub struct BackendClient {
    http: reqwest::Client,
    base: String,
    run: RunId,
    token: String,
}

impl std::fmt::Debug for BackendClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendClient")
            .field("base", &self.base)
            .field("run", &self.run)
            .finish_non_exhaustive()
    }
}

impl BackendClient {
    /// A client for `run` at `backend_url` (without `/api/v1`).
    ///
    /// # Errors
    ///
    /// Fails when the HTTP client cannot be built.
    pub fn new(backend_url: &str, run: RunId, token: String) -> Result<Self, ExecutionError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| {
                ExecutionError::new(
                    ErrorKind::WorkerLost,
                    crate::llm::client_build_error(&error),
                )
            })?;
        Ok(Self {
            http,
            base: backend_url.trim_end_matches('/').to_owned(),
            run,
            token,
        })
    }

    /// The run this client reports for.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    async fn exchange<T: DeserializeOwned>(
        &self,
        method: Method,
        route: &str,
        body: Option<&EventBatch>,
    ) -> Result<T, ReportError> {
        let url = format!("{}{API_BASE}/worker/runs/{}/{route}", self.base, self.run);
        let mut request = self.http.request(method, url).bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| ReportError::Transient(error.to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| ReportError::Transient(error.to_string()))?;
        if status.is_success() {
            return serde_json::from_slice(&bytes)
                .map_err(|error| ReportError::Transient(format!("unreadable reply: {error}")));
        }
        Err(refusal(status, &bytes))
    }

    /// The assigned specification.
    ///
    /// # Errors
    ///
    /// See [`ReportError`].
    pub async fn specification(&self) -> Result<RunSpec, ReportError> {
        self.exchange(Method::GET, "specification", None).await
    }

    /// Sends `events`; returns the highest sequence number the backend has
    /// persisted.
    ///
    /// # Errors
    ///
    /// See [`ReportError`].
    pub async fn send(&self, events: Vec<RunEvent>) -> Result<u64, ReportError> {
        let batch = EventBatch { events };
        let ack: EventAck = self.exchange(Method::POST, "events", Some(&batch)).await?;
        Ok(ack.acknowledged_seq)
    }

    /// Whether cancellation was requested.
    ///
    /// # Errors
    ///
    /// See [`ReportError`].
    pub async fn cancel_requested(&self) -> Result<bool, ReportError> {
        let control: WorkerControl = self.exchange(Method::GET, "control", None).await?;
        Ok(control.cancel_requested)
    }
}

fn refusal(status: StatusCode, bytes: &[u8]) -> ReportError {
    let Ok(body) = serde_json::from_slice::<ErrorBody>(bytes) else {
        return if status.is_server_error() {
            ReportError::Transient(format!("HTTP {status}"))
        } else {
            ReportError::Rejected(ErrorCode::BadRequest, format!("HTTP {status}"))
        };
    };
    let error = body.error;
    let acknowledged = error
        .details
        .iter()
        .find_map(|detail| detail.strip_prefix("acknowledged_seq=")?.parse().ok());
    match (error.code, acknowledged) {
        (ErrorCode::SequenceGap, Some(acknowledged)) => ReportError::Gap(acknowledged),
        (ErrorCode::Internal, _) => ReportError::Transient(error.message),
        (code, _) => ReportError::Rejected(code, error.message),
    }
}

/// Fetches the specification, retrying while the backend is unreachable.
///
/// # Errors
///
/// Fails when the backend refuses, or stays unreachable for five minutes.
pub async fn fetch_specification(
    client: &BackendClient,
    backoff: Backoff,
) -> Result<RunSpec, ExecutionError> {
    let mut failures = Retrying::new(backoff);
    loop {
        match client.specification().await {
            Ok(spec) => return Ok(spec),
            Err(ReportError::Transient(message)) => failures.wait(message).await?,
            Err(other) => return Err(other.into_execution()),
        }
    }
}

/// Retry state while the backend is unreachable.
struct Retrying {
    backoff: Backoff,
    count: u32,
    since: Option<tokio::time::Instant>,
}

impl Retrying {
    const fn new(backoff: Backoff) -> Self {
        Self {
            backoff,
            count: 0,
            since: None,
        }
    }

    const fn reset(&mut self) {
        self.count = 0;
        self.since = None;
    }

    async fn wait(&mut self, message: String) -> Result<(), ExecutionError> {
        let since = *self.since.get_or_insert_with(tokio::time::Instant::now);
        if since.elapsed() >= UNREACHABLE {
            return Err(ReportError::Transient(message).into_execution());
        }
        tracing::warn!(%message, "backend unreachable; retrying");
        tokio::time::sleep(self.backoff.delay(self.count, None)).await;
        self.count += 1;
        Ok(())
    }
}

/// Delivers every event from `events` until the channel closes and the
/// backend has acknowledged all of them. Unacknowledged events are resent;
/// the backend ignores those it already stored. Returns the last
/// acknowledged sequence number.
///
/// # Errors
///
/// Fails when the backend refuses the events or stays unreachable for five
/// minutes.
pub async fn deliver(
    client: &BackendClient,
    mut events: EventReceiver,
    backoff: Backoff,
) -> Result<u64, ExecutionError> {
    let mut pending: VecDeque<RunEvent> = VecDeque::new();
    let mut acknowledged = 0;
    let mut failures = Retrying::new(backoff);
    loop {
        if !fill(&mut events, &mut pending).await {
            return Ok(acknowledged);
        }
        match client.send(next_batch(&mut pending)?).await {
            Err(ReportError::Gap(seq)) if pending.front().is_some_and(|e| e.seq > seq + 1) => {
                // The backend lacks events this worker no longer holds.
                return Err(ReportError::Gap(seq).into_execution());
            }
            Ok(seq) | Err(ReportError::Gap(seq)) => {
                acknowledged = seq;
                pending.retain(|event| event.seq > seq);
                failures.reset();
            }
            Err(ReportError::Transient(message)) => failures.wait(message).await?,
            Err(rejected) => return Err(rejected.into_execution()),
        }
    }
}

/// Serialized size of `event` in a batch, with its separator.
fn encoded_len(event: &RunEvent) -> usize {
    serde_json::to_vec(event).map_or(usize::MAX, |bytes| bytes.len() + 1)
}

/// The leading events of `pending` that fit in one request: at most
/// [`BATCH`] events and [`MAX_EVENT_BATCH_BYTES`] bytes. A first event too
/// large on its own has its captured output cut until it fits.
fn next_batch(pending: &mut VecDeque<RunEvent>) -> Result<Vec<RunEvent>, ExecutionError> {
    let budget = MAX_EVENT_BATCH_BYTES - ENVELOPE;
    if let Some(first) = pending.front_mut() {
        fit(first, budget)?;
    }
    let mut size = 0;
    let mut batch = Vec::new();
    for event in pending.iter().take(BATCH) {
        let bytes = encoded_len(event);
        if !batch.is_empty() && size + bytes > budget {
            break;
        }
        size += bytes;
        batch.push(event.clone());
    }
    Ok(batch)
}

fn fit(event: &mut RunEvent, budget: usize) -> Result<(), ExecutionError> {
    while encoded_len(event) > budget {
        if !event.kind.shrink_evidence() {
            return Err(ExecutionError::new(
                ErrorKind::WorkerLost,
                format!(
                    "event {} is larger than the backend accepts ({MAX_EVENT_BATCH_BYTES} bytes)",
                    event.seq
                ),
            ));
        }
        tracing::warn!(
            seq = event.seq,
            "observation too large to report; truncating its output"
        );
    }
    Ok(())
}

/// Tops up `pending` to a batch, waiting for an event when it is empty.
/// Returns false once the channel is closed and nothing is pending.
async fn fill(events: &mut EventReceiver, pending: &mut VecDeque<RunEvent>) -> bool {
    if pending.is_empty() {
        match events.recv().await {
            Some(event) => pending.push_back(event),
            None => return false,
        }
    }
    while pending.len() < BATCH {
        match events.try_recv() {
            Ok(event) => pending.push_back(event),
            Err(_) => break,
        }
    }
    true
}

/// Polls the backend every `interval` and cancels `cancel` once
/// cancellation is requested. Returns when `stop` or `cancel` is cancelled.
pub async fn watch_cancellation(
    client: &BackendClient,
    cancel: &CancellationToken,
    stop: &CancellationToken,
    interval: Duration,
) {
    loop {
        match client.cancel_requested().await {
            Ok(true) => {
                tracing::info!("cancellation requested");
                cancel.cancel();
                return;
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(?error, "cannot read the control state"),
        }
        tokio::select! {
            () = tokio::time::sleep(interval) => {}
            () = cancel.cancelled() => return,
            () = stop.cancelled() => return,
        }
    }
}
