#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Shared Saci contracts used by the CLI, backend and worker.
//!
//! This crate holds the data types and pure logic that every process agrees
//! on. It performs no network or container work.

/// Version of the Saci contracts implemented by this build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
