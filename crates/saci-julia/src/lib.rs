#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Optional Saci module that runs the open SupersonicLabs/Julia-1 decision
//! model as a System One judge service. See `docs/julia-judge.md`.
//!
//! No other Saci crate depends on this one. It runs as its own process, the
//! `saci-julia` binary, and answers the same evaluation format as the
//! TypeSafe System One API.

pub mod download;
pub mod encode;
pub mod engine;
pub mod parity;
pub mod server;
pub mod wire;
