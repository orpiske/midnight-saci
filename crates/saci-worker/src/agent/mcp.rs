//! MCP tools as agent tools, under the same controls as the built-in ones.
//!
//! Each selected tool of each connected server is registered as
//! `mcp__<server>__<tool>`, with the server's description and input schema.
//! Calls count against the step's tool-call limit, run within the
//! operation deadline, stop on cancellation, and are recorded as evidence.
//! A result the server marks as an error is a tool result. A transport or
//! protocol failure is recorded separately, as a protocol error.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Instant;

use rig::message::ToolName;
use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool};
use rmcp::{Peer, RoleClient};
use saci_core::evidence::{CapturedText, EvidenceRecord, McpEvidence};
use saci_core::redaction::{Redactor, capture_text};
use saci_core::run::{ErrorKind, ExecutionError};
use serde_json::Value;

use super::render;
use super::state::AttemptState;
use crate::mcp::McpServer;

/// Longest tool name model providers accept.
const MAX_NAME: usize = 64;

/// One MCP tool as the model sees it.
#[derive(Debug)]
pub struct McpBinding {
    server: String,
    peer: Peer<RoleClient>,
    tool: Tool,
    exposed: String,
}

impl McpBinding {
    /// The name the model calls it by.
    #[must_use]
    pub fn exposed(&self) -> &str {
        &self.exposed
    }
}

/// Every selected tool of `servers`, with stable, distinct names.
#[must_use]
pub fn bindings(servers: &[McpServer]) -> Vec<McpBinding> {
    let mut taken = BTreeSet::new();
    let mut bindings = Vec::new();
    for server in servers {
        for tool in server.tools() {
            let exposed = unique(&exposed_name(server.name(), &tool.name), &mut taken);
            bindings.push(McpBinding {
                server: server.name().to_owned(),
                peer: server.peer().clone(),
                tool: tool.clone(),
                exposed,
            });
        }
    }
    bindings
}

/// `mcp__<server>__<tool>`, limited to the characters and length model
/// providers accept for tool names.
#[must_use]
pub fn exposed_name(server: &str, tool: &str) -> String {
    let clean = |text: &str| -> String {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let name = format!("mcp__{}__{}", clean(server), clean(tool));
    if name.len() <= MAX_NAME {
        return name;
    }
    let digest = fnv(&format!("{server}\0{tool}"));
    format!("{}_{digest:08x}", &name[..MAX_NAME - 9])
}

/// `name`, or `name` with a numeric suffix when an earlier tool took it.
fn unique(name: &str, taken: &mut BTreeSet<String>) -> String {
    let mut candidate = name.to_owned();
    let mut counter = 2;
    while taken.contains(&candidate) {
        let suffix = format!("_{counter}");
        let keep = name.len().min(MAX_NAME - suffix.len());
        candidate = format!("{}{suffix}", &name[..keep]);
        counter += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

fn fnv(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// The agent tool of `binding`.
///
/// # Errors
///
/// Fails when the exposed name is not a valid tool name.
pub fn tool(
    state: Arc<AttemptState>,
    binding: Arc<McpBinding>,
    redactor: Redactor,
) -> Result<DynamicTool, ExecutionError> {
    let name = ToolName::new(binding.exposed.clone()).map_err(|_| {
        ExecutionError::new(
            ErrorKind::Tool,
            format!("invalid tool name `{}`", binding.exposed),
        )
    })?;
    let description = format!(
        "Tool `{}` of the MCP server `{}`: {} Returns evidence.",
        binding.tool.name,
        binding.server,
        binding
            .tool
            .description
            .as_deref()
            .unwrap_or("no description"),
    );
    let schema = Value::Object(binding.tool.input_schema.as_ref().clone());
    Ok(DynamicTool::new(name, description, schema, move |args| {
        let (state, binding, redactor) = (state.clone(), binding.clone(), redactor.clone());
        Box::pin(async move { call(&state, &binding, &redactor, args).await })
    }))
}

async fn call(
    state: &AttemptState,
    binding: &McpBinding,
    redactor: &Redactor,
    args: Value,
) -> Result<ToolOutput, ToolExecutionError> {
    let arguments = object(state, binding, &args)?;
    let _running = state.running();
    let started = Instant::now();
    let outcome = Box::pin(request(state, binding, arguments)).await;
    let mut evidence = McpEvidence {
        server: binding.server.clone(),
        tool: binding.tool.name.to_string(),
        arguments: args,
        result: CapturedText::default(),
        is_error: false,
        protocol_error: None,
    };
    match outcome {
        Ok(result) => {
            evidence.is_error = result.is_error.unwrap_or(false);
            let limit = state.limits.output_limit_bytes;
            evidence.result = capture_text(&result_text(&result), limit, redactor);
        }
        Err(error) => evidence.protocol_error = Some(error),
    }
    let recorded = state.record(started.elapsed(), EvidenceRecord::Mcp(evidence));
    Ok(ToolOutput::text(render::evidence(&recorded)))
}

/// The arguments as a JSON object; anything else is rejected unsent.
fn object(
    state: &AttemptState,
    binding: &McpBinding,
    args: &Value,
) -> Result<serde_json::Map<String, Value>, ToolExecutionError> {
    if let Value::Object(arguments) = args {
        return Ok(arguments.clone());
    }
    let reason = format!(
        "the arguments of `{}` must be a JSON object",
        binding.exposed
    );
    state.reject(&binding.exposed, &args.to_string(), &reason);
    Err(ToolExecutionError::invalid_args(format!(
        "{reason}. Call the tool again."
    )))
}

/// Sends the call, bounded by the operation deadline and cancellation.
async fn request(
    state: &AttemptState,
    binding: &McpBinding,
    arguments: serde_json::Map<String, Value>,
) -> Result<CallToolResult, String> {
    let request = CallToolRequestParams::new(binding.tool.name.clone()).with_arguments(arguments);
    let deadline = tokio::time::sleep(state.operation_deadline());
    tokio::select! {
        result = Box::pin(binding.peer.call_tool(request)) => {
            result.map_err(|error| format!("the call failed: {error}"))
        }
        () = deadline => Err(format!(
            "no result within the operation deadline ({}s)",
            state.limits.operation_timeout_secs
        )),
        () = state.cancel.cancelled() => Err("the call was cancelled".to_owned()),
    }
}

/// The result's content as text: text blocks as written, other blocks
/// described, and structured content as JSON.
fn result_text(result: &CallToolResult) -> String {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{MAX_NAME, exposed_name, unique};

    #[test]
    fn names_are_valid_stable_and_distinct() {
        assert_eq!(
            exposed_name("wanaku", "get_route"),
            "mcp__wanaku__get_route"
        );
        assert_eq!(exposed_name("db", "query.sql"), "mcp__db__query_sql");
        let long = exposed_name("server", &"t".repeat(100));
        assert_eq!(long.len(), MAX_NAME);
        assert_eq!(long, exposed_name("server", &"t".repeat(100)), "stable");
        assert_ne!(long, exposed_name("server", &"t".repeat(101)));
        let mut taken = BTreeSet::new();
        let first = unique(&exposed_name("db", "a.b"), &mut taken);
        let second = unique(&exposed_name("db", "a_b"), &mut taken);
        assert_eq!(
            (first.as_str(), second.as_str()),
            ("mcp__db__a_b", "mcp__db__a_b_2")
        );
        for builtin in ["shell", "http", "assess"] {
            assert!(!first.starts_with(builtin));
        }
    }
}
