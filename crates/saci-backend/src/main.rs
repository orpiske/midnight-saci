#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `saci-backend` entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

/// Saci backend service.
#[derive(Debug, Parser)]
#[command(name = "saci-backend", version, about = "Run the Saci backend service")]
struct Args {
    /// Configuration file (see docs/configuration.md).
    #[arg(long, short, default_value = "saci.yaml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match Box::pin(saci_backend::server::run(&args.config)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}
