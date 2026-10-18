#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Shared Saci contracts used by the CLI, backend and worker.
//!
//! This crate holds the data types and pure logic that every process agrees
//! on. It performs no network or container work.

pub mod action;
pub mod api;
pub mod assessment;
pub mod bundle;
pub mod config;
pub mod event;
pub mod evidence;
pub mod ids;
pub mod parser;
pub mod plan;
pub mod redaction;
pub mod run;
pub mod spec;
pub mod status;
pub mod view;

/// Version of the Saci contracts implemented by this build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
