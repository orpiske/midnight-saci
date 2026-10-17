//! The MCP servers enabled for a run's environment, connected through
//! `saci-mcp` with the run's credentials: a stdio server gets the shell's
//! explicit environment, and `${NAME}` expands to environment parameters
//! and secrets only.

use std::collections::BTreeMap;
use std::time::Duration;

use saci_core::run::ExecutionError;
use saci_core::spec::McpServerSpec;
pub use saci_mcp::{McpServer, shutdown_all};

use crate::credentials::Credentials;

impl saci_mcp::Projection for Credentials {
    fn environment(&self) -> &BTreeMap<String, String> {
        self.shell_environment()
    }

    fn expand(&self, text: &str) -> String {
        Self::expand(self, text)
    }

    fn redactor(&self) -> &saci_core::redaction::Redactor {
        Self::redactor(self)
    }
}

const fn client(credentials: &Credentials, timeout: Duration) -> saci_mcp::Client<'_> {
    saci_mcp::Client {
        name: "saci-worker",
        place: "worker",
        projection: credentials,
        timeout,
    }
}

/// Connects to one server within `timeout`.
///
/// # Errors
///
/// See [`saci_mcp::connect`].
pub async fn connect(
    name: &str,
    spec: &McpServerSpec,
    credentials: &Credentials,
    timeout: Duration,
) -> Result<McpServer, ExecutionError> {
    Box::pin(saci_mcp::connect(name, spec, client(credentials, timeout))).await
}

/// Connects to every enabled server, each within `timeout`.
///
/// # Errors
///
/// See [`saci_mcp::connect_all`].
pub async fn connect_all(
    servers: &BTreeMap<String, McpServerSpec>,
    credentials: &Credentials,
    timeout: Duration,
) -> Result<Vec<McpServer>, ExecutionError> {
    Box::pin(saci_mcp::connect_all(servers, client(credentials, timeout))).await
}
