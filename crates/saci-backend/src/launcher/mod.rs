//! Starting, observing and removing workers (`docs/launchers.md`).
//!
//! Every worker gets a stable name derived from its run, so a launch whose
//! result was lost can be recognized instead of repeated, and labels that
//! let the backend find its workers after a restart.

pub mod container;
pub mod kubernetes;

use std::time::Duration;

use saci_core::config::{Config, Runtime};
use saci_core::ids::RunId;
use saci_core::spec::{KubernetesSecretKey, RunSpec, SecretRef};

pub use container::ContainerLauncher;
pub use kubernetes::KubernetesLauncher;

/// Label carried by every worker the backend starts.
pub const MANAGED_LABEL: &str = "io.saci.managed";
/// Label naming the worker's run.
pub const RUN_LABEL: &str = "io.saci.run";

/// The worker name of `run`: `saci-<run id>`.
#[must_use]
pub fn worker_name(run: &RunId) -> String {
    format!("saci-{run}")
}

/// The run a worker name belongs to, if it is one of ours.
#[must_use]
pub fn run_of(name: &str) -> Option<RunId> {
    RunId::parse(name.strip_prefix("saci-")?).ok()
}

/// A failed launcher operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct LaunchError {
    /// What went wrong and what to change.
    pub message: String,
    /// The operation may have taken effect anyway (it timed out, or its
    /// result could not be read).
    pub uncertain: bool,
}

impl LaunchError {
    /// A failure that certainly changed nothing.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            uncertain: false,
        }
    }

    /// A failure whose effect is unknown.
    #[must_use]
    pub fn uncertain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            uncertain: true,
        }
    }
}

/// What the backend knows about a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerState {
    /// Created but not running yet.
    Starting,
    /// Running.
    Running,
    /// Ended with this exit code.
    Exited(i32),
    /// The runtime has no such worker.
    Missing,
}

/// Everything a worker is started with.
#[derive(Clone, PartialEq, Eq)]
pub struct Assignment {
    /// The run.
    pub run: RunId,
    /// Environment variables of the worker, values included.
    pub environment: Vec<(String, String)>,
    /// Environment variables read from Kubernetes Secrets (kubernetes
    /// runtime only).
    pub secret_refs: Vec<(String, KubernetesSecretKey)>,
    /// CPU limit, as the runtime accepts it.
    pub cpu: Option<String>,
    /// Memory limit, as the runtime accepts it.
    pub memory: Option<String>,
    /// The run's `limits.run_timeout_secs`.
    pub run_timeout: Duration,
}

impl std::fmt::Debug for Assignment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self
            .environment
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        f.debug_struct("Assignment")
            .field("run", &self.run)
            .field("environment", &names)
            .field("secret_refs", &self.secret_refs)
            .finish_non_exhaustive()
    }
}

/// Builds the worker's assignment: reporting configuration, the LLM key
/// and the target secrets, resolved from the backend's environment
/// (`lookup`). Nothing else of the backend's environment is passed.
///
/// # Errors
///
/// Fails when `backend.worker_url` is not set or a value the run needs is
/// missing from the backend's environment.
pub fn assignment(
    config: &Config,
    spec: &RunSpec,
    token: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Assignment, LaunchError> {
    let worker_url = config.backend.worker_url.as_deref().ok_or_else(|| {
        LaunchError::new(
            "backend.worker_url is not set; set it to the backend URL as workers reach it \
             (see docs/launchers.md#networking)",
        )
    })?;
    let mut environment = vec![
        ("SACI_BACKEND_URL".to_owned(), worker_url.to_owned()),
        ("SACI_RUN_ID".to_owned(), spec.run_id.to_string()),
        ("SACI_WORKER_TOKEN".to_owned(), token.to_owned()),
    ];
    let kubernetes = config.worker.runtime == Runtime::Kubernetes;
    let (values, secret_refs) = credentials(spec, kubernetes, &lookup)?;
    environment.extend(values);
    Ok(Assignment {
        run: spec.run_id.clone(),
        environment,
        secret_refs,
        cpu: spec.environment.worker.cpu.clone(),
        memory: spec.environment.worker.memory.clone(),
        run_timeout: Duration::from_secs(spec.limits.run_timeout_secs),
    })
}

type Credentials = (Vec<(String, String)>, Vec<(String, KubernetesSecretKey)>);

/// The LLM key and the target secrets, under the names the worker reads:
/// values from the backend's environment, and (with `kubernetes`)
/// references to Kubernetes Secrets the worker's pod reads itself.
fn credentials(
    spec: &RunSpec,
    kubernetes: bool,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<Credentials, LaunchError> {
    let mut credentials = (Vec::new(), Vec::new());
    if let Some(name) = &spec.llm.api_key_env {
        match &spec.llm.api_key_secret {
            Some(secret) if kubernetes => credentials.1.push((name.clone(), secret.clone())),
            _ => {
                let value = lookup(name).ok_or_else(|| {
                    LaunchError::new(format!(
                        "llm.api_key_env names {name}, which is not set in the backend's environment"
                    ))
                })?;
                credentials.0.push((name.clone(), value));
            }
        }
    }
    for (name, secret) in &spec.environment.secrets {
        target_secret(name, secret, kubernetes, lookup, &mut credentials)?;
    }
    Ok(credentials)
}

/// Adds target secret `name` as `SACI_SECRET_<name>`.
fn target_secret(
    name: &str,
    secret: &SecretRef,
    kubernetes: bool,
    lookup: &impl Fn(&str) -> Option<String>,
    (values, refs): &mut Credentials,
) -> Result<(), LaunchError> {
    let variable = format!("SACI_SECRET_{name}");
    match secret {
        SecretRef::Env { env } => {
            let value = lookup(env).ok_or_else(|| {
                LaunchError::new(format!(
                    "secret {name} comes from {env}, which is not set in the backend's environment"
                ))
            })?;
            values.push((variable, value));
        }
        SecretRef::Kubernetes { secret, key } if kubernetes => {
            let reference = KubernetesSecretKey {
                name: secret.clone(),
                key: key.clone(),
            };
            refs.push((variable, reference));
        }
        SecretRef::Kubernetes { secret, key } => {
            return Err(LaunchError::new(format!(
                "secret {name} comes from the Kubernetes Secret {secret}/{key}, which only the \
                 kubernetes runtime can read"
            )));
        }
    }
    Ok(())
}

/// The configured launcher.
#[derive(Debug, Clone)]
pub enum Launcher {
    /// Podman or Docker on the backend's host.
    Container(ContainerLauncher),
    /// Kubernetes Jobs in one namespace.
    Kubernetes(KubernetesLauncher),
}

impl Launcher {
    /// Resolves the launcher `config.worker.runtime` selects.
    ///
    /// # Errors
    ///
    /// Fails when the runtime's command is missing or unusable.
    pub async fn from_config(config: &Config) -> Result<Self, LaunchError> {
        match config.worker.runtime {
            Runtime::Kubernetes => {
                let launcher = KubernetesLauncher::from_config(&config.worker)?;
                Box::pin(launcher.check()).await?;
                Ok(Self::Kubernetes(launcher))
            }
            runtime => {
                let path = std::env::var_os("PATH").unwrap_or_default();
                let launcher = ContainerLauncher::resolve(runtime, &path, &config.worker)?;
                launcher.check().await?;
                Ok(Self::Container(launcher))
            }
        }
    }

    /// Starts the worker of `assignment`.
    ///
    /// # Errors
    ///
    /// A [`LaunchError`]; `uncertain` when the worker may exist anyway.
    pub async fn launch(&self, assignment: &Assignment) -> Result<(), LaunchError> {
        match self {
            Self::Container(launcher) => Box::pin(launcher.launch(assignment)).await,
            Self::Kubernetes(launcher) => Box::pin(launcher.launch(assignment)).await,
        }
    }

    /// The state of `run`'s worker.
    ///
    /// # Errors
    ///
    /// Fails when the runtime cannot be asked.
    pub async fn state(&self, run: &RunId) -> Result<WorkerState, LaunchError> {
        match self {
            Self::Container(launcher) => Box::pin(launcher.state(run)).await,
            Self::Kubernetes(launcher) => Box::pin(launcher.state(run)).await,
        }
    }

    /// Stops `run`'s worker: SIGTERM, then SIGKILL after `grace`.
    ///
    /// # Errors
    ///
    /// Fails when the runtime refuses.
    pub async fn stop(&self, run: &RunId, grace: Duration) -> Result<(), LaunchError> {
        match self {
            Self::Container(launcher) => Box::pin(launcher.stop(run, grace)).await,
            Self::Kubernetes(launcher) => Box::pin(launcher.stop(run, grace)).await,
        }
    }

    /// The last lines of `run`'s worker output, for diagnosing failures.
    pub async fn logs(&self, run: &RunId) -> String {
        match self {
            Self::Container(launcher) => Box::pin(launcher.logs(run)).await,
            Self::Kubernetes(launcher) => Box::pin(launcher.logs(run)).await,
        }
    }

    /// Removes `run`'s worker and its resources.
    ///
    /// # Errors
    ///
    /// Fails when the runtime refuses.
    pub async fn remove(&self, run: &RunId) -> Result<(), LaunchError> {
        match self {
            Self::Container(launcher) => Box::pin(launcher.remove(run)).await,
            Self::Kubernetes(launcher) => Box::pin(launcher.remove(run)).await,
        }
    }

    /// Runs whose workers exist, running or not.
    ///
    /// # Errors
    ///
    /// Fails when the runtime cannot be asked.
    pub async fn workers(&self) -> Result<Vec<RunId>, LaunchError> {
        match self {
            Self::Container(launcher) => Box::pin(launcher.workers()).await,
            Self::Kubernetes(launcher) => Box::pin(launcher.workers()).await,
        }
    }
}
