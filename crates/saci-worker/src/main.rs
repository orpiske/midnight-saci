#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `saci-worker` entry point.

use clap::Parser;

/// Saci per-run worker.
#[derive(Debug, Parser)]
#[command(
    name = "saci-worker",
    version,
    about = "Execute one Saci run specification"
)]
struct Args {}

fn main() -> anyhow::Result<()> {
    let _args = Args::parse();
    Ok(())
}
