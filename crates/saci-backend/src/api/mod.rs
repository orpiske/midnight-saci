//! The REST API under `/api/v1` (see `docs/api.md` and `docs/openapi.yaml`).

mod actions;
mod catalog;
mod error;
mod investigation;
mod investigations;
mod runs;
mod submit;
mod worker;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use saci_core::api::{API_BASE, ErrorCode, MAX_EVENT_BATCH_BYTES};
use saci_core::config::Config;
use saci_core::ids::RunId;
use tokio::sync::watch;

pub use error::Failure;
pub use worker::{backend_redactor, issue_token};

use crate::store::Store;

/// What the API hands accepted and cancelled runs to: the worker launcher
/// and supervisor. Both calls return at once; the work happens elsewhere.
pub trait Supervisor: Send + Sync + 'static {
    /// A run was accepted and stored; start its worker.
    fn submitted(&self, run: RunId);
    /// Cancellation of an unfinished run was requested.
    fn cancel_requested(&self, run: RunId);
}

/// How clients authenticate.
#[derive(Clone, PartialEq, Eq)]
pub enum ClientAuth {
    /// No token: allowed only on a loopback listener (enforced by
    /// configuration validation).
    Open,
    /// `Authorization: Bearer <token>` is required.
    Token(String),
}

impl std::fmt::Debug for ClientAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Open => "Open",
            Self::Token(_) => "Token([REDACTED])",
        })
    }
}

impl ClientAuth {
    /// Reads the token named by `backend.auth.token_env`.
    ///
    /// # Errors
    ///
    /// Fails when the variable is configured but unset or empty.
    pub fn from_config(
        config: &Config,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let Some(auth) = &config.backend.auth else {
            return Ok(Self::Open);
        };
        match lookup(&auth.token_env) {
            Some(token) if !token.is_empty() => Ok(Self::Token(token)),
            _ => Err(format!(
                "backend.auth.token_env names `{}`, which is not set",
                auth.token_env
            )),
        }
    }

    fn allows(&self, header: Option<&str>) -> bool {
        match self {
            Self::Open => true,
            Self::Token(token) => header
                .and_then(|value| value.strip_prefix("Bearer "))
                .is_some_and(|sent| constant_time_eq(sent.as_bytes(), token.as_bytes())),
        }
    }
}

/// Compares secrets without an early exit on the first difference.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0_u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Shared state of every route.
#[derive(Clone)]
pub struct Backend {
    /// Durable run records.
    pub store: Store,
    /// The validated configuration.
    pub config: Arc<Config>,
    /// Starts and stops workers.
    pub supervisor: Arc<dyn Supervisor>,
    changes: Arc<watch::Sender<u64>>,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl Backend {
    /// Creates the shared state.
    #[must_use]
    pub fn new(store: Store, config: Config, supervisor: Arc<dyn Supervisor>) -> Self {
        Self {
            store,
            config: Arc::new(config),
            supervisor,
            changes: Arc::new(watch::channel(0).0),
        }
    }

    /// Reads a credential variable from the backend's environment, or the
    /// LLM key written literally in the configuration file.
    #[must_use]
    pub fn credential(&self, name: &str) -> Option<String> {
        self.config
            .llm
            .lookup(name, |name| std::env::var(name).ok())
    }

    /// Wakes clients waiting for events; call after a run record changed.
    pub fn notify(&self) {
        self.changes.send_modify(|version| *version += 1);
    }

    /// Observes [`Backend::notify`] calls.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
}

/// Client routes for outcome actions (`docs/actions.md`).
fn action_routes() -> Router<Backend> {
    Router::new()
        .route("/runs/{run_id}/actions", get(actions::list))
        .route("/runs/{run_id}/actions/{name}", get(actions::show))
        .route(
            "/runs/{run_id}/actions/{name}/approval",
            post(actions::approve),
        )
        .route(
            "/runs/{run_id}/actions/{name}/rejection",
            post(actions::reject),
        )
        .route(
            "/runs/{run_id}/actions/{name}/redispatch",
            post(actions::redispatch),
        )
        .route(
            "/runs/{run_id}/actions/{name}/reconciliation",
            post(actions::reconcile),
        )
        .route("/actions", get(actions::by_state))
}

/// Client routes for runs and what they may be submitted with.
fn client_routes() -> Router<Backend> {
    Router::new()
        .route("/runs", post(runs::submit).get(runs::list))
        .route("/runs/{run_id}", get(runs::inspect))
        .route("/runs/{run_id}/events", get(runs::events))
        .route("/runs/{run_id}/report", get(runs::report))
        .route("/runs/{run_id}/evidence/{evidence_id}", get(runs::evidence))
        .route("/runs/{run_id}/cancellation", post(runs::cancel))
        .route("/runs/{run_id}/plan", get(catalog::plan))
        .route("/runs/{run_id}/investigations", get(investigations::list))
        .route("/investigations", post(investigations::create))
        .route(
            "/investigations/{investigation_id}",
            get(investigations::show),
        )
        .route("/environments", get(catalog::environments))
        .route("/policies", get(catalog::policies))
}

/// The client and worker routes, the health check and the web console.
pub fn router(backend: Backend, auth: ClientAuth) -> Router {
    let runs = client_routes()
        .merge(action_routes())
        .route_layer(middleware::from_fn_with_state(Arc::new(auth), authorize));
    let workers = Router::new()
        .route(
            "/worker/runs/{run_id}/specification",
            get(worker::specification),
        )
        .route(
            "/worker/runs/{run_id}/events",
            post(worker::events).layer(DefaultBodyLimit::max(MAX_EVENT_BATCH_BYTES)),
        )
        .route("/worker/runs/{run_id}/control", get(worker::control))
        .route_layer(middleware::from_fn_with_state(
            backend.clone(),
            worker::authorize,
        ));
    let api = Router::new()
        .route("/health", get(runs::health))
        .merge(runs)
        .merge(workers)
        .fallback(unknown_route)
        .with_state(backend);
    Router::new()
        .nest(API_BASE, api)
        .merge(crate::console::routes())
}

/// Unknown API routes answer the structured error, never a page.
async fn unknown_route() -> Failure {
    Failure::new(ErrorCode::NotFound, "no such API route")
}

async fn authorize(
    State(auth): State<Arc<ClientAuth>>,
    request: Request,
    next: Next,
) -> Result<Response, Failure> {
    let header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if auth.allows(header) {
        Ok(next.run(request).await)
    } else {
        Err(Failure::new(
            ErrorCode::Unauthorized,
            "send Authorization: Bearer <token> with the token configured by backend.auth.token_env",
        ))
    }
}
