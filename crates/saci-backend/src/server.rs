//! Starting the backend: configuration, database, API and supervision.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use saci_core::config::Config;
use tokio::net::TcpListener;

use crate::actions;
use crate::api::{Backend, ClientAuth, router};
use crate::launcher::{LaunchError, Launcher};
use crate::store::Store;
use crate::supervisor;

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
    let launcher = start_launcher(&config).await;
    let (commands, receiver) = supervisor::channel();
    let backend = Backend::new(store, config, Arc::new(commands));
    let timing = supervisor::Timing::default();
    tokio::spawn(Box::pin(supervisor::run(
        backend.clone(),
        launcher,
        receiver,
        timing,
    )));
    tokio::spawn(Box::pin(actions::run(backend.clone())));
    announce(listener.local_addr()?, &data_dir);
    axum::serve(listener, router(backend, auth))
        .with_graceful_shutdown(shutdown)
        .await
        .context("the backend server failed")
}

/// Logs and prints where the backend listens.
fn announce(address: std::net::SocketAddr, data_dir: &Path) {
    tracing::info!(%address, data_dir = %data_dir.display(), "Saci backend listening");
    println!(
        "Saci backend listening on http://{address} (data in {})",
        data_dir.display()
    );
    if crate::console::embedded() {
        println!("Web console at http://{address}/ui/");
    }
}

/// Resolves the configured launcher. A missing runtime does not stop the
/// backend: runs are still accepted and inspected, and each launch reports
/// the problem.
async fn start_launcher(config: &Config) -> Result<Launcher, LaunchError> {
    let launcher = Launcher::from_config(config).await;
    match &launcher {
        Ok(launcher) => tracing::info!(?launcher, "worker launcher ready"),
        Err(error) => tracing::warn!(%error, "runs cannot be launched until this is fixed"),
    }
    launcher
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
