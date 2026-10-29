//! A local backend that the watcher starts when none answers.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::process::{Child, Command};

use super::log;
use crate::client::Client;

/// How long a started backend has to answer.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a backend has to stop after SIGTERM.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// `saci backend start --config <config>`, started on demand.
pub struct LocalBackend {
    config: PathBuf,
    child: Option<Child>,
}

impl LocalBackend {
    /// A backend that starts from `config` when it is first needed.
    #[must_use]
    pub const fn new(config: PathBuf) -> Self {
        Self {
            config,
            child: None,
        }
    }

    /// Makes sure that a backend answers. When none does and the started
    /// backend is not running, starts it and waits until it answers.
    ///
    /// # Errors
    ///
    /// Fails when the backend cannot start, exits, or does not answer in
    /// time.
    pub async fn ensure(&mut self, client: &Client) -> anyhow::Result<()> {
        if answers(client).await {
            return Ok(());
        }
        if !self.running() {
            self.start(client)?;
        }
        let deadline = Instant::now() + START_TIMEOUT;
        while Instant::now() < deadline {
            if answers(client).await {
                log(&format!("the backend answers at {}", client.base()));
                return Ok(());
            }
            if !self.running() {
                bail!("the backend started with {} exited", self.config.display());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        bail!(
            "the backend did not answer at {} in {} s",
            client.base(),
            START_TIMEOUT.as_secs()
        )
    }

    fn start(&mut self, client: &Client) -> anyhow::Result<()> {
        let saci = std::env::current_exe().context("cannot find the saci binary")?;
        let child = Command::new(saci)
            .args(["backend", "start", "--config"])
            .arg(&self.config)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            // Its own process group: the watcher stops it after Ctrl-C.
            .process_group(0)
            .spawn()
            .context("cannot start the backend")?;
        log(&format!(
            "no backend answers at {}; started `saci backend start --config {}`",
            client.base(),
            self.config.display()
        ));
        self.child = Some(child);
        Ok(())
    }

    fn running(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    }

    /// Stops the started backend with SIGTERM, and kills it when it does
    /// not stop in time. A backend that the watcher did not start stays.
    pub async fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if let Some(pid) = child.id().and_then(|id| i32::try_from(id).ok()) {
            let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
        }
        if tokio::time::timeout(STOP_TIMEOUT, child.wait())
            .await
            .is_err()
        {
            log("the backend did not stop in time; killing it");
            let _ = child.kill().await;
        }
    }
}

async fn answers(client: &Client) -> bool {
    client.get::<serde_json::Value>("/health").await.is_ok()
}

/// True for `http://127.0.0.1:…`, `http://localhost:…` and `http://[::1]:…`:
/// the watcher starts only backends on this host.
#[must_use]
pub fn is_local(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', '?']).next().unwrap_or_default();
    let host = host.rsplit_once(':').map_or(host, |(host, _)| host);
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_urls_are_local() {
        for url in [
            "http://127.0.0.1:8080",
            "http://localhost:18080/",
            "http://[::1]:9",
        ] {
            assert!(is_local(url), "{url}");
        }
        for url in [
            "https://127.0.0.1:8080",
            "http://saci.example.com:8080",
            "http://10.0.0.1",
        ] {
            assert!(!is_local(url), "{url}");
        }
    }
}
