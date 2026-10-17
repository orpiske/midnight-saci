#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Controlled fixtures for Saci tests.
//!
//! [`FakeModel`] is an OpenAI-compatible Chat Completions server that replays
//! scripted replies and records every request, so model interaction can be
//! checked without a paid provider.

mod fake_model;

pub use fake_model::{FakeModel, Recorded, Reply, text_reply, tool_calls_reply};
