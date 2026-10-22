//! The MCP adapter and the HTTP server.
//!
//! `tools/list` shows one tool per configured script. `tools/call` maps
//! results as follows:
//!
//! - An unknown tool name is a JSON-RPC error (`-32602`, invalid params).
//! - Invalid arguments, or no free execution slot, give a tool result with
//!   `isError: true` and status `rejected`. Nothing runs.
//! - An execution gives a tool result with the [`Execution`] fields as
//!   structured content and as JSON text. `isError` is false only for status
//!   `completed`.

use std::sync::Arc;

use anyhow::Context as _;
use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorData, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::config::{Config, ConfigError, ExecutionConfig, ServerConfig};
use crate::executor::{self, DRAIN_GRACE, Execution, KILL_GRACE, Status};
use crate::registry::{Registry, ScriptTool};

/// Path of the health endpoint. It is not authenticated and only reports
/// that the process is up.
pub const HEALTH_PATH: &str = "/health";

/// Target of the audit events.
const AUDIT: &str = "saci_script_runner::audit";

/// The MCP handler. Clones share the registry and the execution slots.
#[derive(Debug, Clone)]
pub struct ScriptRunner {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    registry: Registry,
    execution: ExecutionConfig,
    slots: Arc<Semaphore>,
    shutdown: CancellationToken,
}

impl ScriptRunner {
    /// Builds the handler. Scripts are cancelled when `shutdown` is.
    ///
    /// # Errors
    ///
    /// Fails when the registry cannot be built.
    pub fn new(config: &Config, shutdown: CancellationToken) -> Result<Self, ConfigError> {
        Ok(Self {
            shared: Arc::new(Shared {
                registry: Registry::new(config)?,
                execution: config.execution.clone(),
                slots: Arc::new(Semaphore::new(config.execution.max_concurrent)),
                shutdown,
            }),
        })
    }

    /// Waits until no script runs, at most `limit`.
    pub async fn wait_idle(&self, limit: std::time::Duration) -> bool {
        let all = u32::try_from(self.shared.execution.max_concurrent).unwrap_or(u32::MAX);
        tokio::time::timeout(limit, self.shared.slots.acquire_many(all))
            .await
            .is_ok()
    }

    async fn execute(&self, tool: &ScriptTool, call: Call<'_>) -> CallToolResult {
        let arguments = match tool.arguments(call.arguments) {
            Ok(arguments) => arguments,
            Err(reason) => return call.rejected(&reason),
        };
        let Ok(_slot) = self.shared.slots.clone().try_acquire_owned() else {
            return call.rejected(&format!(
                "busy: {} scripts are already running, try again later",
                self.shared.execution.max_concurrent
            ));
        };
        let execution = &self.shared.execution;
        let request = executor::Request {
            executable: &tool.executable,
            arguments: &arguments,
            working_directory: &execution.working_directory,
            environment: &execution.environment,
            timeout: tool.timeout,
            max_output_bytes: execution.max_output_bytes,
        };
        let cancelled = either(&call.cancel, &self.shared.shutdown);
        call.finished(executor::run(&request, cancelled).await)
    }
}

/// Completes when either token is cancelled.
async fn either(first: &CancellationToken, second: &CancellationToken) {
    tokio::select! {
        () = first.cancelled() => {}
        () = second.cancelled() => {}
    }
}

/// One `tools/call`, for results and audit events.
struct Call<'a> {
    id: String,
    tool: &'a str,
    client: Option<String>,
    arguments: Option<&'a serde_json::Map<String, Value>>,
    cancel: CancellationToken,
}

impl Call<'_> {
    fn rejected(&self, reason: &str) -> CallToolResult {
        tracing::warn!(
            target: AUDIT,
            execution_id = %self.id,
            tool = self.tool,
            client = self.client.as_deref().unwrap_or("-"),
            status = "rejected",
            reason,
            "script call rejected"
        );
        CallToolResult::structured_error(json!({
            "execution_id": self.id,
            "status": "rejected",
            "error": reason,
        }))
    }

    fn finished(&self, execution: Execution) -> CallToolResult {
        tracing::info!(
            target: AUDIT,
            execution_id = %self.id,
            tool = self.tool,
            client = self.client.as_deref().unwrap_or("-"),
            status = execution.status.as_str(),
            exit_code = ?execution.exit_code,
            signal = ?execution.signal,
            duration_ms = execution.duration_ms,
            stdout_truncated = execution.stdout_truncated,
            stderr_truncated = execution.stderr_truncated,
            "script execution"
        );
        let completed = execution.status == Status::Completed;
        let mut value = json!({ "execution_id": self.id });
        if let (Value::Object(target), Ok(Value::Object(fields))) =
            (&mut value, serde_json::to_value(execution))
        {
            target.extend(fields);
        }
        if completed {
            CallToolResult::structured(value)
        } else {
            CallToolResult::structured_error(value)
        }
    }
}

fn mcp_tool(tool: &ScriptTool) -> Tool {
    let annotations = ToolAnnotations::new()
        .read_only(tool.read_only)
        .destructive(!tool.read_only)
        .idempotent(false)
        .open_world(true);
    Tool::new(
        tool.name.clone(),
        tool.description.clone(),
        Arc::new(tool.input_schema()),
    )
    .with_annotations(annotations)
}

impl ServerHandler for ScriptRunner {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("saci-script-runner", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(
            self.shared.registry.tools().iter().map(mcp_tool).collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(tool) = self.shared.registry.get(&request.name) else {
            return Err(ErrorData::invalid_params(
                format!("unknown tool `{}`", request.name),
                None,
            ));
        };
        let call = Call {
            id: uuid::Uuid::new_v4().to_string(),
            tool: &tool.name,
            client: context
                .peer
                .peer_info()
                .map(|info| info.client_info.name.clone()),
            arguments: request.arguments.as_ref(),
            cancel: context.ct.clone(),
        };
        Ok(self.execute(tool, call).await)
    }
}

/// Reads the bearer token from the environment variable that
/// `server.auth.token` names.
///
/// # Errors
///
/// Fails when the variable is not set, or is empty.
pub fn auth_token(config: &ServerConfig) -> anyhow::Result<Option<String>> {
    let Some(auth) = &config.auth else {
        return Ok(None);
    };
    let name = &auth.token.env;
    let token = std::env::var(name).with_context(|| format!("`{name}` is not set"))?;
    anyhow::ensure!(!token.is_empty(), "`{name}` is empty");
    Ok(Some(token))
}

/// The HTTP application: the MCP endpoint and the health endpoint.
pub fn router(
    runner: ScriptRunner,
    config: &ServerConfig,
    token: Option<String>,
    shutdown: &CancellationToken,
) -> Router {
    let mut http = StreamableHttpServerConfig::default()
        .with_cancellation_token(shutdown.child_token())
        .with_allowed_origins(config.allowed_origins.clone());
    if !config.allowed_hosts.is_empty() {
        http = http.with_allowed_hosts(config.allowed_hosts.clone());
    }
    let service = StreamableHttpService::new(
        move || Ok(runner.clone()),
        Arc::new(LocalSessionManager::default()),
        http,
    );
    let gate = Arc::new(Gate {
        token,
        origins: config.allowed_origins.clone(),
    });
    Router::new()
        .nest_service(&config.endpoint, service)
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let gate = gate.clone();
            async move { gate.check(request, next).await }
        }))
        .route(
            HEALTH_PATH,
            get(|| async { axum::Json(json!({"status": "ok"})) }),
        )
}

/// Authentication and `Origin` checks for the MCP endpoint.
struct Gate {
    token: Option<String>,
    origins: Vec<String>,
}

impl Gate {
    async fn check(&self, request: Request, next: Next) -> Response {
        let headers = request.headers();
        if let Some(origin) = headers.get("origin") {
            let allowed = origin
                .to_str()
                .is_ok_and(|origin| self.origins.iter().any(|allowed| allowed == origin));
            if !allowed {
                return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
            }
        }
        if let Some(token) = &self.token {
            let sent = headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .unwrap_or_default();
            if !same(sent.as_bytes(), token.as_bytes()) {
                return (StatusCode::UNAUTHORIZED, "missing or wrong bearer token").into_response();
            }
        }
        next.run(request).await
    }
}

/// Compares in time that does not depend on where the inputs differ.
fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (l, r)| difference | (l ^ r))
            == 0
}

/// Serves until `shutdown` is cancelled, then stops every running script and
/// waits for it to be reaped.
///
/// # Errors
///
/// Fails when the registry cannot be built or the server fails.
pub async fn serve(
    config: &Config,
    token: Option<String>,
    listener: TcpListener,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let runner = ScriptRunner::new(config, shutdown.clone())?;
    let app = router(runner.clone(), &config.server, token, &shutdown);
    tracing::info!(
        address = %listener.local_addr()?,
        endpoint = config.server.endpoint,
        tools = config.tools.len(),
        "script runner ready"
    );
    let stopping = shutdown.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { stopping.cancelled().await })
        .await?;
    if !runner
        .wait_idle(KILL_GRACE + DRAIN_GRACE + std::time::Duration::from_secs(1))
        .await
    {
        tracing::warn!("scripts were still running at shutdown");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::same;

    #[test]
    fn tokens_compare_exactly() {
        assert!(same(b"secret", b"secret"));
        assert!(!same(b"secret", b"secreT"));
        assert!(!same(b"secret", b"secret2"));
        assert!(!same(b"", b"secret"));
    }
}
