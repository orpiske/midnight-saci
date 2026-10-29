//! Connections to configured MCP servers, shared by the worker (tools for
//! steps) and the backend (outcome actions).
//!
//! Transport handling, initialization and protocol-version negotiation come
//! from the `rmcp` SDK (the same major version as Rig's `rmcp` bridge).
//! Saci only decides which servers to start, what they receive, which of
//! their tools are exposed, and bounds every step with a deadline.

mod schema;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::process::Stdio;
use std::time::Duration;

use rmcp::model::{
    CallToolResult, ClientCapabilities, ClientInfo, ContentBlock, Implementation, Tool,
};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{Peer, RoleClient, ServiceExt};
use saci_core::redaction::Redactor;
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::{HttpServer, McpServerSpec, McpTransport, StdioServer};

pub use rmcp;
pub use schema::{check_arguments, fit_arguments};

/// What a server may receive: the variables of a stdio server process, and
/// the values `${NAME}` expands to in its settings.
pub trait Projection: Sync {
    /// Variables a stdio server process starts with, before its own `env`.
    fn environment(&self) -> &BTreeMap<String, String>;
    /// `text` with `${NAME}` replaced by the values this projection allows.
    fn expand(&self, text: &str) -> String;
    /// Hides the credential values a server may receive, in what the
    /// server writes to its standard error.
    fn redactor(&self) -> &Redactor;
}

/// Who connects, and with what.
#[derive(Clone, Copy)]
pub struct Client<'a> {
    /// Client name announced to servers, such as `saci-worker`.
    pub name: &'static str,
    /// Where stdio commands are looked up, for messages: `worker` or `backend`.
    pub place: &'static str,
    /// Variables and expansion values.
    pub projection: &'a dyn Projection,
    /// Bound on starting and initializing each server.
    pub timeout: Duration,
}

impl std::fmt::Debug for Client<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("name", &self.name)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// A connected MCP server and its selected tools.
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

fn client_info(name: &str) -> ClientInfo {
    ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new(name, env!("CARGO_PKG_VERSION")),
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
    client: Client<'_>,
) -> Result<Vec<McpServer>, ExecutionError> {
    let mut connected = Vec::new();
    for (name, spec) in servers {
        match Box::pin(connect(name, spec, client)).await {
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
    client: Client<'_>,
) -> Result<McpServer, ExecutionError> {
    let setup = Box::pin(initialize(name, spec, client));
    tokio::time::timeout(client.timeout, setup)
        .await
        .map_err(|_| {
            mcp_error(
                name,
                format!("did not initialize within {}s", client.timeout.as_secs()),
            )
        })?
}

type Service = RunningService<RoleClient, ClientInfo>;

async fn initialize(
    name: &str,
    spec: &McpServerSpec,
    client: Client<'_>,
) -> Result<McpServer, ExecutionError> {
    let service = match &spec.transport {
        McpTransport::Stdio(stdio) => Box::pin(serve_stdio(name, stdio, client)).await?,
        McpTransport::Http(http) => Box::pin(serve_http(name, http, client)).await?,
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
    client: Client<'_>,
) -> Result<Service, ExecutionError> {
    let transport = start_stdio(name, stdio, client)?;
    Box::pin(client_info(client.name).serve(transport))
        .await
        .map_err(|error| init_error(name, error))
}

async fn serve_http(
    name: &str,
    http: &HttpServer,
    client: Client<'_>,
) -> Result<Service, ExecutionError> {
    let transport = http_transport(name, http, client.projection)?;
    Box::pin(client_info(client.name).serve(transport))
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
    client: Client<'_>,
) -> Result<TokioChildProcess, ExecutionError> {
    let credentials = client.projection;
    let mut command = tokio::process::Command::new(&stdio.command);
    command
        .args(&stdio.args)
        .env_clear()
        .envs(credentials.environment())
        .envs(
            stdio
                .env
                .iter()
                .map(|(key, value)| (key, credentials.expand(value))),
        )
        .kill_on_drop(true);
    let (transport, stderr) = TokioChildProcess::builder(command)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| spawn_error(name, stdio, client.place, &error))?;
    if let Some(stderr) = stderr {
        tokio::spawn(log_stderr(
            name.to_owned(),
            stderr,
            credentials.redactor().clone(),
        ));
    }
    Ok(transport)
}

fn spawn_error(
    name: &str,
    stdio: &StdioServer,
    place: &str,
    error: &std::io::Error,
) -> ExecutionError {
    let reason = if error.kind() == std::io::ErrorKind::NotFound {
        format!(
            "command `{}` was not found in the {place} PATH",
            stdio.command
        )
    } else {
        format!("cannot start `{}`: {error}", stdio.command)
    };
    mcp_error(name, reason)
}

/// Longest stderr line logged, after redaction; the rest is dropped.
const STDERR_LINE: usize = 2048;

/// Logs what a stdio server writes to its standard error, a line at a
/// time, with every credential it may have received redacted.
async fn log_stderr(name: String, stderr: tokio::process::ChildStderr, redactor: Redactor) {
    use tokio::io::AsyncBufReadExt as _;
    let mut lines = tokio::io::BufReader::new(stderr).split(b'\n');
    while let Ok(Some(line)) = lines.next_segment().await {
        let text = String::from_utf8_lossy(&line);
        let text = redactor.redact(text.trim_end());
        let mut end = text.len().min(STDERR_LINE);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        tracing::info!(server = %name, "mcp server: {}", &text[..end]);
    }
}

fn http_transport(
    name: &str,
    http: &HttpServer,
    credentials: &dyn Projection,
) -> Result<StreamableHttpClientTransport<reqwest::Client>, ExecutionError> {
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

/// The result's content as text: text blocks as written, other blocks
/// described, and structured content as JSON.
pub fn result_text(result: &CallToolResult) -> String {
    let mut text = String::new();
    for block in &result.content {
        if !text.is_empty() {
            text.push('\n');
        }
        match block {
            ContentBlock::Text(content) => text.push_str(&content.text),
            other => {
                let kind = serde_json::to_value(other)
                    .ok()
                    .and_then(|value| value["type"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "non-text".to_owned());
                let _ = write!(text, "[{kind} content, not shown]");
            }
        }
    }
    if let Some(structured) = &result.structured_content {
        let _ = write!(text, "\nstructured content: {structured}");
    }
    text
}
