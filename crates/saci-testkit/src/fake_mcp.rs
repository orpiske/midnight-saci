//! A controlled MCP server for tests and the local walkthrough.
//!
//! Tools: `echo` (returns its `text`), `fail` (a tool-level error result),
//! `sleep` (waits `seconds`), and `env` (returns the value of environment
//! variable `name`, to check what the server process received).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock as Content, ErrorData, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

/// The fake server's handler.
#[derive(Debug, Clone, Copy, Default)]
pub struct FakeMcp;

fn tool(name: &'static str, description: &'static str, schema: &Value) -> Tool {
    let Value::Object(schema) = schema.clone() else {
        return Tool::new(name, description, Arc::new(Map::new()));
    };
    Tool::new(name, description, Arc::new(schema))
}

fn string_arg(arguments: Option<&Map<String, Value>>, name: &str) -> String {
    arguments
        .and_then(|args| args.get(name))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

impl ServerHandler for FakeMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("saci-fake-mcp", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let text = json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]});
        let seconds = json!({"type": "object", "properties": {"seconds": {"type": "number"}}});
        let name = json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]});
        Ok(ListToolsResult::with_all_items(vec![
            tool("echo", "Return the given text", &text),
            tool(
                "fail",
                "Always report a tool error",
                &json!({"type": "object"}),
            ),
            tool("sleep", "Wait for the given number of seconds", &seconds),
            tool("env", "Return an environment variable of the server", &name),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let arguments = request.arguments.as_ref();
        match request.name.as_ref() {
            "echo" => Ok(CallToolResult::success(vec![Content::text(format!(
                "echo: {}",
                string_arg(arguments, "text")
            ))])),
            "fail" => Ok(CallToolResult::error(vec![Content::text(
                "the tool failed on purpose",
            )])),
            "sleep" => {
                let seconds = arguments
                    .and_then(|args| args.get("seconds"))
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0);
                tokio::time::sleep(Duration::from_secs_f64(seconds.clamp(0.0, 600.0))).await;
                Ok(CallToolResult::success(vec![Content::text("slept")]))
            }
            "env" => {
                let value = std::env::var(string_arg(arguments, "name")).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(value)]))
            }
            other => Err(ErrorData::invalid_params(
                format!("no tool `{other}`"),
                None,
            )),
        }
    }
}

/// A Streamable HTTP router serving [`FakeMcp`] at `/mcp`. When `token` is
/// set, requests must carry `Authorization: Bearer <token>`.
pub fn http_router(token: Option<String>, cancel: CancellationToken) -> Router {
    // A test fixture reached from containers under any host name.
    let config = StreamableHttpServerConfig::default()
        .with_cancellation_token(cancel)
        .disable_allowed_hosts();
    let service = StreamableHttpService::new(
        || Ok(FakeMcp),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let token = Arc::new(token);
    Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let token = token.clone();
            async move { authorize(token.as_deref(), request, next).await }
        }))
}

async fn authorize(
    token: Option<&str>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if let Some(token) = token {
        let expected = format!("Bearer {token}");
        let sent = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        if sent != Some(expected.as_str()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(next.run(request).await)
}

/// Serves [`http_router`] on an ephemeral loopback port and returns the
/// endpoint URL.
///
/// # Errors
///
/// Fails when the listener cannot be bound.
pub async fn serve_http(token: Option<String>) -> std::io::Result<(String, CancellationToken)> {
    let cancel = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = http_router(token, cancel.clone());
    let shutdown = cancel.clone();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await
    });
    Ok((format!("http://{address}/mcp"), cancel))
}

/// Path of the `saci-fake-mcp` binary next to the running test, building it
/// first when the test run did not.
///
/// # Errors
///
/// Fails when the binary cannot be located or built.
pub fn binary() -> std::io::Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    let profile_dir = exe
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| std::io::Error::other("cannot locate the target directory"))?;
    let path = profile_dir.join("saci-fake-mcp");
    if !path.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let status = std::process::Command::new(cargo)
            .args(["build", "-p", "saci-testkit", "--bin", "saci-fake-mcp"])
            .status()?;
        if !status.success() || !path.exists() {
            return Err(std::io::Error::other("building saci-fake-mcp failed"));
        }
    }
    Ok(path)
}
