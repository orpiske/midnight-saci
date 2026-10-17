//! Recorded observations produced by tools during a run.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{EvidenceId, UnitKey};

/// One recorded tool observation. Evidence is redacted before it is stored
/// or shown to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// Run-unique identifier referenced by assessments.
    pub id: EvidenceId,
    /// Unit whose attempt produced the observation.
    pub unit: UnitKey,
    /// One-based attempt number.
    pub attempt: u32,
    /// When the observation finished.
    #[serde(with = "time::serde::rfc3339")]
    pub recorded_at: OffsetDateTime,
    /// Wall-clock duration of the tool operation in milliseconds.
    pub duration_ms: u64,
    /// The tool-specific record.
    #[serde(flatten)]
    pub record: EvidenceRecord,
}

/// Tool-specific evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum EvidenceRecord {
    /// A shell command.
    Shell(ShellEvidence),
    /// An HTTP request.
    Http(HttpEvidence),
    /// A configured MCP tool invocation.
    Mcp(McpEvidence),
    /// A tool request that was rejected before it ran.
    Rejected(RejectedCall),
}

impl EvidenceRecord {
    /// Halves every captured text, for an observation too large to report
    /// whole; the drop stays visible as truncation. Returns false when
    /// nothing was left to drop.
    pub fn shrink(&mut self) -> bool {
        let texts: Vec<&mut CapturedText> = match self {
            Self::Shell(shell) => vec![&mut shell.stdout, &mut shell.stderr],
            Self::Http(http) => http
                .request_body
                .iter_mut()
                .chain(http.response.iter_mut().map(|response| &mut response.body))
                .collect(),
            Self::Mcp(mcp) => vec![&mut mcp.result],
            Self::Rejected(_) => Vec::new(),
        };
        let mut dropped = false;
        for text in texts {
            let half = text.text.len() / 2;
            dropped |= text.truncate_to(half);
        }
        dropped
    }

    /// Why this observation holds no answer from the target, when it holds
    /// none: the request got no response, the command did not finish, the
    /// MCP call got no result, or the call was rejected before it ran.
    #[must_use]
    pub fn unanswered(&self) -> Option<String> {
        match self {
            Self::Http(http) if http.response.is_none() => Some(format!(
                "the request got no response ({})",
                http.transport_error.as_deref().unwrap_or("no answer")
            )),
            Self::Shell(shell) if shell.exit_code.is_none() => Some(format!(
                "the command did not finish ({})",
                shell.termination.as_str()
            )),
            Self::Mcp(mcp) => mcp
                .protocol_error
                .as_ref()
                .map(|error| format!("the MCP call got no result ({error})")),
            Self::Rejected(call) => Some(format!(
                "the call was rejected before it ran ({})",
                call.reason
            )),
            _ => None,
        }
    }
}

/// Text captured from a tool, with any truncation made explicit.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapturedText {
    /// The retained (and redacted) text.
    pub text: String,
    /// Whether bytes beyond the configured limit were discarded.
    #[serde(default)]
    pub truncated: bool,
    /// Size of the original output in bytes.
    pub total_bytes: u64,
}

impl CapturedText {
    /// Captures `text` completely.
    #[must_use]
    pub fn complete(text: impl Into<String>) -> Self {
        let text = text.into();
        let total_bytes = text.len() as u64;
        Self {
            text,
            truncated: false,
            total_bytes,
        }
    }

    /// Keeps at most `bytes` of the text, cut on a character boundary, and
    /// marks it truncated; returns whether anything was dropped.
    pub fn truncate_to(&mut self, bytes: usize) -> bool {
        if self.text.len() <= bytes {
            return false;
        }
        let mut end = bytes;
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        self.text.truncate(end);
        self.truncated = true;
        true
    }

    /// The text as shown to the model and in reports, with a visible note
    /// when output was dropped.
    #[must_use]
    pub fn rendered(&self) -> String {
        if self.truncated {
            format!(
                "{}\n[output truncated: showing {} of {} bytes]",
                self.text,
                self.text.len(),
                self.total_bytes
            )
        } else {
            self.text.clone()
        }
    }
}

/// How a shell command ended, when it did not simply exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Termination {
    /// The command ran to completion and returned an exit status.
    Exited,
    /// The operation deadline expired and the process tree was stopped.
    TimedOut,
    /// The run was cancelled and the process tree was stopped.
    Cancelled,
    /// The command ended the shell session (for example with `exit`).
    SessionEnded,
    /// The shell session was lost; its context cannot be recovered.
    SessionLost,
}

impl Termination {
    /// The wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exited => "exited",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::SessionEnded => "session_ended",
            Self::SessionLost => "session_lost",
        }
    }
}

/// Evidence for one shell command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellEvidence {
    /// The command as submitted.
    pub command: String,
    /// Captured standard output.
    pub stdout: CapturedText,
    /// Captured standard error.
    pub stderr: CapturedText,
    /// Exit status, when the command exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// How the command ended.
    pub termination: Termination,
    /// The command ran in a new shell session because the previous one was
    /// lost or ended; earlier variables, functions and directory are gone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restarted: bool,
    /// The shell session did not survive this command.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub session_lost: bool,
}

/// Evidence for one HTTP request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpEvidence {
    /// Request method.
    pub method: String,
    /// Request URL, with secrets redacted.
    pub url: String,
    /// Request headers, with secrets redacted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub request_headers: BTreeMap<String, String>,
    /// Request body, when one was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body: Option<CapturedText>,
    /// The response, when one was received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<HttpResponseEvidence>,
    /// Transport failure (connection, TLS, timeout), distinct from a response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_error: Option<String>,
}

/// A received HTTP response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponseEvidence {
    /// Status code.
    pub status: u16,
    /// Response headers, with secrets redacted.
    pub headers: BTreeMap<String, String>,
    /// Response body.
    pub body: CapturedText,
    /// Why the body is incomplete, when reading it failed, timed out or was
    /// cancelled after the status and headers arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_error: Option<String>,
}

/// Evidence for one MCP tool invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpEvidence {
    /// Configured server name.
    pub server: String,
    /// Tool name as exposed by the server. Serialized as `tool_name`:
    /// `tool` is the evidence kind.
    #[serde(rename = "tool_name")]
    pub tool: String,
    /// Arguments sent, with secrets redacted.
    pub arguments: serde_json::Value,
    /// Tool result content.
    pub result: CapturedText,
    /// Whether the server reported the tool result as an error.
    pub is_error: bool,
    /// Connection or protocol failure, distinct from a tool result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_error: Option<String>,
}

/// A tool request that never ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedCall {
    /// Requested tool name.
    pub name: String,
    /// Requested arguments as received.
    pub arguments: String,
    /// Why it was rejected.
    pub reason: String,
}
