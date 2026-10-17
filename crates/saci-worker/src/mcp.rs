//! Connections to the MCP servers enabled for a run's environment.
//!
//! Transport handling, initialization and protocol-version negotiation come
//! from the `rmcp` SDK (the same major version as Rig's `rmcp` bridge).
//! Saci only decides which servers to start, what they receive, which of
//! their tools are exposed, and bounds every step with a deadline.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::time::Duration;

use rig::rig_reqwest::reqwest::Client;
use rmcp::model::{ClientCapabilities, ClientInfo, Implementation, Tool};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{Peer, RoleClient, ServiceExt};
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::{HttpServer, McpServerSpec, McpTransport, StdioServer};

use crate::credentials::Credentials;

/// A connected MCP server and the tools steps may use.
pub struct McpServer {
    name: String,
    service: Service,
    tools: Vec<Tool>,
}

impl std::fmt::Debug for McpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServer")
            .field("name", &self.name)
            .field(
                "tools",
                &self.tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl McpServer {
    /// Configured server name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The exposed tools, after selection.
    #[must_use]
    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    /// The connection, for calling tools.
    #[must_use]
    pub fn peer(&self) -> &Peer<RoleClient> {
        self.service.peer()
    }

    /// Closes the connection; a stdio server process is stopped.
    pub async fn shutdown(self) {
        if let Err(error) = self.service.cancel().await {
            tracing::warn!(server = %self.name, %error, "MCP server did not shut down cleanly");
        }
    }
}

fn mcp_error(name: &str, message: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::new(ErrorKind::Tool, format!("MCP server `{name}`: {message}"))
}

fn client_info() -> ClientInfo {
    ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("saci-worker", env!("CARGO_PKG_VERSION")),
    )
}

/// Connects to every enabled server, each within `timeout`. On any failure
/// the servers already connected are shut down.
///
/// # Errors
///
/// Fails when a server cannot be started, does not initialize in time, or
/// lacks a selected tool.
pub async fn connect_all(
    servers: &BTreeMap<String, McpServerSpec>,
    credentials: &Credentials,
    timeout: Duration,
) -> Result<Vec<McpServer>, ExecutionError> {
    let mut connected = Vec::new();
    for (name, spec) in servers {
        match Box::pin(connect(name, spec, credentials, timeout)).await {
            Ok(server) => connected.push(server),
            Err(error) => {
                shutdown_all(connected).await;
                return Err(error);
            }
        }
    }
    Ok(connected)
}

/// Shuts every server down.
pub async fn shutdown_all(servers: Vec<McpServer>) {
    for server in servers {
        server.shutdown().await;
    }
}

/// Connects to one server, initializes it and discovers its tools.
///
/// # Errors
///
/// See [`connect_all`].
pub async fn connect(
    name: &str,
    spec: &McpServerSpec,
    credentials: &Credentials,
    timeout: Duration,
) -> Result<McpServer, ExecutionError> {
    let setup = Box::pin(initialize(name, spec, credentials));
    tokio::time::timeout(timeout, setup).await.map_err(|_| {
        mcp_error(
            name,
            format!("did not initialize within {}s", timeout.as_secs()),
        )
    })?
}

type Service = RunningService<RoleClient, ClientInfo>;

async fn initialize(
    name: &str,
    spec: &McpServerSpec,
    credentials: &Credentials,
) -> Result<McpServer, ExecutionError> {
    let service = match &spec.transport {
        McpTransport::Stdio(stdio) => Box::pin(serve_stdio(name, stdio, credentials)).await?,
        McpTransport::Http(http) => Box::pin(serve_http(name, http, credentials)).await?,
    };
    let discovered = Box::pin(list_tools(name, &service)).await?;
    let tools = select(name, discovered, spec.tools.as_deref())?;
    Ok(McpServer {
        name: name.to_owned(),
        service,
        tools,
    })
}

fn init_error(name: &str, error: impl std::fmt::Display) -> ExecutionError {
    mcp_error(name, format!("initialization failed: {error}"))
}

async fn serve_stdio(
    name: &str,
    stdio: &StdioServer,
    credentials: &Credentials,
) -> Result<Service, ExecutionError> {
    let transport = start_stdio(name, stdio, credentials)?;
    Box::pin(client_info().serve(transport))
        .await
        .map_err(|error| init_error(name, error))
}

async fn serve_http(
    name: &str,
    http: &HttpServer,
    credentials: &Credentials,
) -> Result<Service, ExecutionError> {
    let transport = http_transport(name, http, credentials)?;
    Box::pin(client_info().serve(transport))
        .await
        .map_err(|error| init_error(name, error))
}

async fn list_tools(name: &str, service: &Service) -> Result<Vec<Tool>, ExecutionError> {
    Box::pin(service.peer().list_all_tools())
        .await
        .map_err(|error| mcp_error(name, format!("listing tools failed: {error}")))
}

fn start_stdio(
    name: &str,
    stdio: &StdioServer,
    credentials: &Credentials,
) -> Result<TokioChildProcess, ExecutionError> {
    let mut command = tokio::process::Command::new(&stdio.command);
    command
        .args(&stdio.args)
        .env_clear()
        .envs(credentials.shell_environment())
        .envs(
            stdio
                .env
                .iter()
                .map(|(key, value)| (key, credentials.expand(value))),
        )
        .kill_on_drop(true);
    let (transport, _) = TokioChildProcess::builder(command)
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| {
            let reason = if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "command `{}` was not found in the worker PATH",
                    stdio.command
                )
            } else {
                format!("cannot start `{}`: {error}", stdio.command)
            };
            mcp_error(name, reason)
        })?;
    Ok(transport)
}

fn http_transport(
    name: &str,
    http: &HttpServer,
    credentials: &Credentials,
) -> Result<StreamableHttpClientTransport<Client>, ExecutionError> {
    let mut headers = HashMap::new();
    for (key, value) in &http.headers {
        let header = http::HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| mcp_error(name, format!("`{key}` is not a valid header name")))?;
        let value = http::HeaderValue::from_str(&credentials.expand(value))
            .map_err(|_| mcp_error(name, format!("the value of header `{key}` is not valid")))?;
        headers.insert(header, value);
    }
    let config = StreamableHttpClientTransportConfig::with_uri(credentials.expand(&http.url))
        .custom_headers(headers);
    Ok(StreamableHttpClientTransport::from_config(config))
}

fn select(
    name: &str,
    discovered: Vec<Tool>,
    wanted: Option<&[String]>,
) -> Result<Vec<Tool>, ExecutionError> {
    let Some(wanted) = wanted else {
        return Ok(discovered);
    };
    let missing: Vec<&str> = wanted
        .iter()
        .filter(|tool| !discovered.iter().any(|found| found.name == tool.as_str()))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        let offered: Vec<&str> = discovered.iter().map(|tool| tool.name.as_ref()).collect();
        return Err(mcp_error(
            name,
            format!(
                "does not offer {}; it offers {}",
                missing.join(", "),
                offered.join(", ")
            ),
        ));
    }
    Ok(discovered
        .into_iter()
        .filter(|tool| wanted.iter().any(|name| tool.name == name.as_str()))
        .collect())
}
