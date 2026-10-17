//! The REST API under `/api/v1` (see `docs/api.md` and `docs/openapi.yaml`).

mod actions;
mod error;
mod runs;
mod submit;
mod worker;

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use saci_core::api::{API_BASE, ErrorCode};
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

/// The client and worker routes and the health check.
pub fn router(backend: Backend, auth: ClientAuth) -> Router {
    let runs = Router::new()
        .route("/runs", post(runs::submit).get(runs::list))
        .route("/runs/{run_id}", get(runs::inspect))
        .route("/runs/{run_id}/events", get(runs::events))
        .route("/runs/{run_id}/report", get(runs::report))
        .route("/runs/{run_id}/evidence/{evidence_id}", get(runs::evidence))
        .route("/runs/{run_id}/cancellation", post(runs::cancel))
        .route("/runs/{run_id}/actions", get(actions::list))
        .route_layer(middleware::from_fn_with_state(Arc::new(auth), authorize));
    let workers = Router::new()
        .route(
            "/worker/runs/{run_id}/specification",
            get(worker::specification),
        )
        .route("/worker/runs/{run_id}/events", post(worker::events))
        .route("/worker/runs/{run_id}/control", get(worker::control))
        .route_layer(middleware::from_fn_with_state(
            backend.clone(),
            worker::authorize,
        ));
    let api = Router::new()
        .route("/health", get(runs::health))
        .merge(runs)
        .merge(workers)
        .with_state(backend);
    Router::new().nest(API_BASE, api)
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
