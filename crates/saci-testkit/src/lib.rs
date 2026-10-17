#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Controlled fixtures for Saci tests.
//!
//! [`FakeModel`] is an OpenAI-compatible Chat Completions server that replays
//! scripted replies and records every request, so model interaction can be
//! checked without a paid provider. [`fake_mcp`] is a controlled MCP server,
//! also built as the `saci-fake-mcp` binary. [`playbook`] and
//! [`fake_target`] are the walkthrough's model and system under test, built
//! as `saci-fake-model` and `saci-fake-target`.

pub mod fake_mcp;
mod fake_model;
pub mod fake_target;
pub mod playbook;

pub use fake_model::{FakeModel, Recorded, Reply, text_reply, tool_calls_reply};
