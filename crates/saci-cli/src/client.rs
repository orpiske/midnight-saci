//! The CLI's connection to a backend, and failures phrased with what to do
//! next.

use std::process::ExitCode;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use saci_core::api::{API_BASE, ErrorBody, ErrorCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Default backend URL.
pub const DEFAULT_BACKEND: &str = "http://127.0.0.1:8080";

/// Exit codes (`docs/cli.md#exit-codes`).
pub mod exit {
    /// Invalid input: plan, configuration or arguments.
    pub const INVALID: u8 = 2;
    /// The backend is unreachable, refused the credentials, or failed.
    pub const BACKEND: u8 = 3;
    /// The outcome of a request is unknown; see the message for a safe retry.
    pub const UNCERTAIN: u8 = 4;
}

/// A failure with its exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// Process exit code.
    pub code: u8,
    /// What happened and what to do about it.
    pub message: String,
}

impl CliError {
    /// Creates a failure.
    #[must_use]
    pub fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The process exit code.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code)
    }
}

impl From<anyhow::Error> for CliError {
    fn from(error: anyhow::Error) -> Self {
        Self::new(exit::INVALID, format!("{error:#}"))
    }
}

/// Where the backend is and how to authenticate.
#[derive(Debug, Clone, clap::Args)]
pub struct Connection {
    /// Backend URL.
    #[arg(long, env = "SACI_BACKEND_URL", default_value = DEFAULT_BACKEND, global = true)]
    pub backend: String,
    /// Environment variable holding the client token (`backend.auth.token_env` on the backend).
    #[arg(long, default_value = "SACI_TOKEN", global = true)]
    pub token_env: String,
}

/// A connected client.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl Client {
    /// Builds a client from the connection options.
    ///
    /// # Errors
    ///
    /// Fails when the HTTP client cannot be built.
    pub fn new(connection: &Connection) -> Result<Self, CliError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|error| {
                CliError::new(
                    exit::BACKEND,
                    format!("cannot build the HTTP client: {error}"),
                )
            })?;
        let token = std::env::var(&connection.token_env)
            .ok()
            .filter(|t| !t.is_empty());
        Ok(Self {
            http,
            base: connection.backend.trim_end_matches('/').to_owned(),
            token,
        })
    }

    /// Sends a request to `route` (below `/api/v1`) and decodes the reply.
    /// Returns the status with the body, so callers can tell 200 from 202.
    /// A lost reply to anything but `GET` leaves its effect unknown.
    ///
    /// # Errors
    ///
    /// A [`CliError`] with guidance for connection, authentication, API and
    /// uncertain-outcome failures.
    pub async fn request<B: Serialize + Sync, T: DeserializeOwned>(
        &self,
        method: Method,
        route: &str,
        body: Option<&B>,
        headers: &[(&str, String)],
    ) -> Result<(StatusCode, T), CliError> {
        let changes = method != Method::GET;
        let response = self
            .build(method, route, body, headers)
            .send()
            .await
            .map_err(|error| self.transport(&error, changes))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| self.transport(&error, changes))?;
        if status.is_success() {
            let body = serde_json::from_slice(&bytes).map_err(|error| {
                CliError::new(
                    exit::BACKEND,
                    format!("unreadable reply from {}: {error}", self.base),
                )
            })?;
            return Ok((status, body));
        }
        Err(self.refusal(status, &bytes))
    }

    fn build<B: Serialize>(
        &self,
        method: Method,
        route: &str,
        body: Option<&B>,
        headers: &[(&str, String)],
    ) -> reqwest::RequestBuilder {
        let url = format!("{}{API_BASE}{route}", self.base);
        let mut request = self.http.request(method, &url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        request
    }

    fn transport(&self, error: &reqwest::Error, changes: bool) -> CliError {
        if error.is_connect() {
            return CliError::new(
                exit::BACKEND,
                format!(
                    "cannot reach the backend at {}: {error}. Start it with `saci backend start --config saci.yaml`, \
                     or point --backend (SACI_BACKEND_URL) at a running one.",
                    self.base
                ),
            );
        }
        if changes {
            return CliError::new(
                exit::UNCERTAIN,
                format!(
                    "the reply from {} was lost ({error}); the request may or may not have taken effect.",
                    self.base
                ),
            );
        }
        CliError::new(
            exit::BACKEND,
            format!(
                "the request to {} failed: {error}. It changed nothing; try again.",
                self.base
            ),
        )
    }

    fn refusal(&self, status: StatusCode, bytes: &[u8]) -> CliError {
        let Ok(body) = serde_json::from_slice::<ErrorBody>(bytes) else {
            return CliError::new(
                exit::BACKEND,
                format!("{} answered HTTP {status}", self.base),
            );
        };
        let error = body.error;
        let mut message = error.message.clone();
        for detail in &error.details {
            message.push_str(&format!("\n  {detail}"));
        }
        let code = match error.code {
            ErrorCode::Unauthorized => {
                message.push_str(
                    "\nSet SACI_TOKEN (or the variable named by --token-env) to the token configured by the \
                     backend's backend.auth.token_env.",
                );
                exit::BACKEND
            }
            ErrorCode::ValidationFailed
            | ErrorCode::BadRequest
            | ErrorCode::IdempotencyConflict
            | ErrorCode::NotFound
            | ErrorCode::RunFinished => exit::INVALID,
            _ => exit::BACKEND,
        };
        CliError::new(code, message)
    }
}
