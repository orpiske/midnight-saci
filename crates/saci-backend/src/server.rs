//! Starting the backend: configuration, database, API and supervision.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use saci_core::config::Config;
use saci_core::status::Outcome;
use tokio::net::TcpListener;

use crate::api::{Backend, ClientAuth, router};
use crate::launcher::{LaunchError, Launcher};
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
    let launcher = start_launcher(&config).await;
    let (commands, receiver) = supervisor::channel();
    let backend = Backend::new(store, config, Arc::new(commands));
    tokio::spawn(Box::pin(supervise(backend.clone(), launcher, receiver)));
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

/// Handles launch and cancellation requests.
async fn supervise(
    backend: Backend,
    launcher: Result<Launcher, LaunchError>,
    mut commands: CommandReceiver,
) {
    while let Some(command) = commands.recv().await {
        Box::pin(handle(&backend, launcher.as_ref(), command)).await;
    }
}

async fn handle(backend: &Backend, launcher: Result<&Launcher, &LaunchError>, command: Command) {
    match command {
        Command::Launch(run) => {
            let launched = match launcher {
                Ok(launcher) => Box::pin(supervisor::launch(backend, launcher, &run)).await,
                Err(error) => Err(error.clone()),
            };
            if let Err(error) = launched {
                let reason = format!("the worker could not be launched: {}", error.message);
                supervisor::finish(backend, &run, Outcome::Error, reason).await;
            }
        }
        Command::Cancel(run) => {
            let Ok(launcher) = launcher else { return };
            if let Err(error) = launcher.stop(&run, supervisor::STOP_GRACE).await {
                tracing::warn!(%run, %error, "cannot stop the worker");
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
