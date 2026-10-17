#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `saci` command-line entry point.

use clap::Parser;

/// Saci, an AI test engineer that executes Markdown test plans.
#[derive(Debug, Parser)]
#[command(name = "saci", version, about = "Saci, an AI test engineer")]
struct Cli {}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    Ok(())
}
