//! `saci-backend` entry point.

use clap::Parser;

/// Saci backend service.
#[derive(Debug, Parser)]
#[command(name = "saci-backend", version, about = "Run the Saci backend service")]
struct Args {}

fn main() -> anyhow::Result<()> {
    let _args = Args::parse();
    Ok(())
}
