//! Starting the backend: configuration, database, API and supervision.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use saci_core::config::Config;
use saci_core::status::Outcome;
use tokio::net::TcpListener;

use crate::api::{Backend, ClientAuth, router};
use crate::store::Store;
use crate::supervisor::{self, Command, CommandReceiver};

/// Loads `config_path` and serves until interrupted (Ctrl-C or SIGTERM).
///
/// # Errors
///
/// Fails on invalid configuration, a missing client token, an unusable data
/// directory or listen address.
pub async fn run(config_path: &Path) -> anyhow::Result<()> {
    let config = Config::load(config_path).map_err(|error| anyhow::anyhow!("{error}"))?;
    let listener = TcpListener::bind(&config.backend.listen)
        .await
        .with_context(|| {
            format!(
                "cannot listen on {} (backend.listen)",
                config.backend.listen
            )
        })?;
    Box::pin(serve(config, listener, shutdown_signal())).await
}

/// Serves on `listener` until `shutdown` completes.
///
/// # Errors
///
/// See [`run`].
pub async fn serve(
    config: Config,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let auth = ClientAuth::from_config(&config, |name| std::env::var(name).ok())
        .map_err(|error| anyhow::anyhow!(error))?;
    let data_dir = config.backend.data_dir.clone();
    let store = Store::open(&data_dir)
        .await
        .with_context(|| format!("cannot open the database in {}", data_dir.display()))?;
    let (commands, receiver) = supervisor::channel();
    let backend = Backend::new(store, config, Arc::new(commands));
    tokio::spawn(supervise(backend.clone(), receiver));
    let address = listener.local_addr()?;
    tracing::info!(%address, data_dir = %data_dir.display(), "Saci backend listening");
    println!(
        "Saci backend listening on http://{address} (data in {})",
        data_dir.display()
    );
    axum::serve(listener, router(backend, auth))
        .with_graceful_shutdown(shutdown)
        .await
        .context("the backend server failed")
}

/// Handles launch and cancellation requests.
async fn supervise(backend: Backend, mut commands: CommandReceiver) {
    while let Some(command) = commands.recv().await {
        match command {
            Command::Launch(run) => {
                let reason = "no worker launcher is available in this build".to_owned();
                supervisor::finish(&backend, &run, Outcome::Error, reason).await;
            }
            Command::Cancel(run) => {
                tracing::info!(%run, "cancellation requested");
            }
        }
    }
}

async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
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
    }
    #[cfg(not(unix))]
    {
        let _ = interrupt.await;
    }
    tracing::info!("shutting down");
}
