//! `saci-julia` entry point.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use saci_julia::download::{self, REVISION};
use saci_julia::encode::Limits;
use saci_julia::engine::{Engine, Options};
use saci_julia::{parity, server};
use tokio_util::sync::CancellationToken;

/// Runs the Julia 1 decision model as a System One judge for Saci.
#[derive(Debug, Parser)]
#[command(name = "saci-julia", version, about)]
struct Cli {
    /// Directory with the model files. Default:
    /// `~/.cache/saci-julia/<revision>`.
    #[arg(long, global = true, env = "SACI_JULIA_MODEL_DIR")]
    model_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve judge evaluations over HTTP.
    Serve {
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:8095")]
        listen: SocketAddr,
        /// Environment variable holding the token that every evaluation
        /// must send as `Authorization: Bearer <token>`. Required when
        /// `--listen` is not a loopback address.
        #[arg(long)]
        token_env: Option<String>,
        #[command(flatten)]
        model: ModelArgs,
    },
    /// Download the model files.
    Download {
        /// Julia-1-ONNX commit to download.
        #[arg(long, default_value = REVISION)]
        revision: String,
    },
    /// Compare the logits with the reference PyTorch values.
    Parity {
        /// The cases file. Default: `parity-cases.json` in the model directory.
        #[arg(long)]
        cases: Option<PathBuf>,
        /// Largest allowed difference between two logits.
        #[arg(long, default_value_t = 1e-3)]
        tolerance: f32,
        #[command(flatten)]
        model: ModelArgs,
    },
}

#[derive(Debug, Args)]
struct ModelArgs {
    /// CPU threads for one evaluation. Default: ONNX Runtime chooses.
    #[arg(long)]
    threads: Option<usize>,
    /// Most tokens of one question with its options and the state.
    #[arg(long, default_value_t = Limits::default().max_length)]
    max_length: usize,
    /// Tokens for the question and its options.
    #[arg(long, default_value_t = Limits::default().head_length)]
    head_length: usize,
    /// Truncate input that does not fit, instead of rejecting it.
    #[arg(long)]
    no_strict: bool,
}

impl ModelArgs {
    const fn options(&self) -> Options {
        Options {
            limits: Limits {
                max_length: self.max_length,
                head_length: self.head_length,
                strict: !self.no_strict,
            },
            threads: self.threads,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match Box::pin(run(cli)).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn model_dir(cli: &Cli) -> anyhow::Result<PathBuf> {
    if let Some(dir) = &cli.model_dir {
        return Ok(dir.clone());
    }
    let home = std::env::var_os("HOME").context("set --model-dir or HOME")?;
    Ok(PathBuf::from(home).join(".cache/saci-julia").join(REVISION))
}

/// Reads the token from the variable `name`.
fn token(name: &str) -> anyhow::Result<String> {
    let value = std::env::var(name).with_context(|| format!("`{name}` is not set"))?;
    anyhow::ensure!(!value.is_empty(), "`{name}` is empty");
    Ok(value)
}

async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let dir = model_dir(&cli)?;
    match cli.command {
        Command::Download { revision } => {
            let token = std::env::var("HF_TOKEN").ok().filter(|t| !t.is_empty());
            download::download(&dir, &revision, token.as_deref()).await?;
            println!("{}", dir.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Serve {
            listen,
            token_env,
            model,
        } => {
            Box::pin(serve(dir, listen, token_env.as_deref(), &model)).await?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Parity {
            cases,
            tolerance,
            model,
        } => {
            let path = cases.unwrap_or_else(|| dir.join("parity-cases.json"));
            check_parity(dir, &path, tolerance, &model).await
        }
    }
}

async fn serve(
    dir: PathBuf,
    listen: SocketAddr,
    token_env: Option<&str>,
    model: &ModelArgs,
) -> anyhow::Result<()> {
    let token = token_env.map(token).transpose()?;
    if token.is_none() && !listen.ip().is_loopback() {
        bail!("--listen {listen} is not a loopback address: set --token-env NAME");
    }
    let options = model.options();
    tracing::info!(model_dir = %dir.display(), "loading the model");
    let engine = tokio::task::spawn_blocking(move || Engine::load(&dir, options)).await??;
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("cannot listen on {listen}"))?;
    tracing::info!(
        %listen,
        path = server::EVALUATE_PATH,
        token = token.is_some(),
        "serving"
    );
    let shutdown = CancellationToken::new();
    let trigger = shutdown.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        trigger.cancel();
    });
    Box::pin(server::serve(Arc::new(engine), token, listener, shutdown)).await
}

async fn check_parity(
    dir: PathBuf,
    path: &std::path::Path,
    tolerance: f32,
    model: &ModelArgs,
) -> anyhow::Result<ExitCode> {
    let cases = parity::read_cases(path)?;
    let options = model.options();
    let report = tokio::task::spawn_blocking(move || {
        let engine = Engine::load(&dir, options)?;
        parity::check(&engine, &cases, tolerance)
    })
    .await??;
    for (case, reason) in &report.mismatches {
        eprintln!("case {case}: {reason}");
    }
    println!(
        "{} of {} cases match (tolerance {tolerance}, largest difference {:.6})",
        report.cases - report.mismatches.len(),
        report.cases,
        report.largest_difference
    );
    Ok(if report.mismatches.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
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
