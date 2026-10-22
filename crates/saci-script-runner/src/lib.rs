#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Optional Saci module that exposes administrator-configured scripts as MCP
//! tools over Streamable HTTP. See `docs/script-runner.md`.
//!
//! No other Saci crate depends on this one. It runs as its own process, the
//! `saci-script-runner` binary.

pub mod config;
pub mod executor;
pub mod registry;
pub mod server;
