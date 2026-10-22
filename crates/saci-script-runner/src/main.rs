//! `saci-script-runner` entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use saci_script_runner::config::Config;
use saci_script_runner::server;
use tokio_util::sync::CancellationToken;

/// Exposes configured scripts as MCP tools over Streamable HTTP.
#[derive(Debug, Parser)]
#[command(name = "saci-script-runner", version, about)]
struct Args {
    /// Configuration file (see docs/script-runner.md).
    #[arg(long, short, default_value = "script-runner.yaml")]
    config: PathBuf,
    /// Check the configuration and exit.
    #[arg(long)]
    check_config: bool,
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
    match Box::pin(run(&args)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}

async fn run(args: &Args) -> anyhow::Result<()> {
    let config = Config::load(&args.config)?;
    if args.check_config {
        println!(
            "{}: valid, {} tools",
            args.config.display(),
            config.tools.len()
        );
        return Ok(());
    }
    let token = server::auth_token(&config.server)?;
    let listener = tokio::net::TcpListener::bind(config.server.bind).await?;
    let shutdown = CancellationToken::new();
    let trigger = shutdown.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        trigger.cancel();
    });
    server::serve(&config, token, listener, shutdown).await
}

async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    else {
        let _ = interrupt.await;
        return;
    };
    tokio::select! {
        _ = interrupt => {}
        _ = terminate.recv() => {}
    }
    tracing::info!("shutting down");
}
