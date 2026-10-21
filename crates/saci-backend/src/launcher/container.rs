//! Workers as Podman or Docker containers, driven through the runtime's
//! command-line binary found in the backend's `PATH`.
//!
//! Commands are built as argument lists and never pass through a shell.
//! Secret values travel in the runtime command's environment and are
//! named with `--env NAME`, so they never appear in an argument list.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use saci_core::config::{PullPolicy, Runtime, WorkerConfig};
use saci_core::ids::RunId;
use saci_core::run::WorkerImage;
use tokio::process::Command;

use super::{Assignment, LaunchError, MANAGED_LABEL, RUN_LABEL, WorkerState, run_of, worker_name};

/// How long `run` may take, including pulling the image.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(300);
/// How long any other runtime command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// Which runtime binary is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `podman`.
    Podman,
    /// `docker`.
    Docker,
}

impl Kind {
    /// The executable name.
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Self::Podman => "podman",
            Self::Docker => "docker",
        }
    }
}

/// Launches workers with one container runtime.
#[derive(Debug, Clone)]
pub struct ContainerLauncher {
    kind: Kind,
    binary: PathBuf,
    image: String,
    network: Option<String>,
    add_hosts: Vec<String>,
    pull_policy: Option<PullPolicy>,
    launch_timeout: Duration,
}

/// Finds `name` in `path` the way a shell would. Returns a non-executable
/// match as an error, so it can be reported.
fn find(name: &str, path: &OsStr) -> Option<Result<PathBuf, PathBuf>> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .map(|candidate| {
            if executable(&candidate) {
                Ok(candidate)
            } else {
                Err(candidate)
            }
        })
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(_: &Path) -> bool {
    true
}

impl ContainerLauncher {
    /// Resolves `runtime` in `path`: `auto` prefers `podman` and falls back
    /// to `docker`.
    ///
    /// # Errors
    ///
    /// Names what is missing or not executable.
    pub fn resolve(
        runtime: Runtime,
        path: &OsStr,
        worker: &WorkerConfig,
    ) -> Result<Self, LaunchError> {
        let candidates: &[Kind] = match runtime {
            Runtime::Auto => &[Kind::Podman, Kind::Docker],
            Runtime::Podman => &[Kind::Podman],
            Runtime::Docker => &[Kind::Docker],
            Runtime::Kubernetes => {
                return Err(LaunchError::new(
                    "the kubernetes runtime does not use a container command",
                ));
            }
        };
        let mut problems = Vec::new();
        for kind in candidates {
            match find(kind.command(), path) {
                Some(Ok(binary)) => return Ok(Self::new(*kind, binary, worker)),
                Some(Err(found)) => problems.push(format!("{} is not executable", found.display())),
                None => problems.push(format!("{} is not in the backend's PATH", kind.command())),
            }
        }
        Err(LaunchError::new(format!(
            "no usable container runtime for worker.runtime `{}`: {}. Install Podman or Docker, \
             or put it on the backend's PATH (PATH={})",
            runtime_name(runtime),
            problems.join("; "),
            path.to_string_lossy()
        )))
    }

    fn new(kind: Kind, binary: PathBuf, worker: &WorkerConfig) -> Self {
        Self {
            kind,
            binary,
            image: worker.image.clone(),
            network: worker.network.clone(),
            add_hosts: worker.add_hosts.clone(),
            pull_policy: worker.image_pull_policy,
            launch_timeout: LAUNCH_TIMEOUT,
        }
    }

    /// Changes how long a launch may take before its outcome counts as
    /// unknown.
    #[must_use]
    pub const fn with_launch_timeout(mut self, timeout: Duration) -> Self {
        self.launch_timeout = timeout;
        self
    }

    /// The runtime in use.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// The resolved binary.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Checks that the runtime answers (`<runtime> version`).
    ///
    /// # Errors
    ///
    /// Includes the runtime's own explanation, for example an unreachable
    /// Docker daemon.
    pub async fn check(&self) -> Result<(), LaunchError> {
        self.run(&["version"], COMMAND_TIMEOUT)
            .await
            .map(drop)
            .map_err(|error| {
                LaunchError::new(format!(
                    "{} is not usable: {}",
                    self.binary.display(),
                    error.message
                ))
            })
    }

    /// The arguments of `run` for `assignment`; values stay in the
    /// environment.
    #[must_use]
    pub fn run_arguments(&self, assignment: &Assignment) -> Vec<String> {
        let name = worker_name(&assignment.run);
        let mut args = vec![
            "run".to_owned(),
            "--detach".to_owned(),
            format!("--name={name}"),
            format!("--label={MANAGED_LABEL}=true"),
            format!("--label={RUN_LABEL}={}", assignment.run),
            "--security-opt=no-new-privileges".to_owned(),
        ];
        args.extend(self.runtime_options());
        if let Some(cpu) = &assignment.cpu {
            args.push(format!("--cpus={cpu}"));
        }
        if let Some(memory) = &assignment.memory {
            args.push(format!("--memory={memory}"));
        }
        for (variable, _) in &assignment.environment {
            args.push("--env".to_owned());
            args.push(variable.clone());
        }
        args.push(self.image.clone());
        args
    }

    /// The `run` options from the worker settings: pull policy and network.
    fn runtime_options(&self) -> impl Iterator<Item = String> + '_ {
        let pull = self
            .pull_policy
            .map(|policy| format!("--pull={}", policy.container()));
        let network = self
            .network
            .iter()
            .map(|network| format!("--network={network}"));
        let hosts = self
            .add_hosts
            .iter()
            .map(|host| format!("--add-host={host}"));
        pull.into_iter().chain(network).chain(hosts)
    }

    /// Starts the worker container.
    ///
    /// # Errors
    ///
    /// A [`LaunchError`]; `uncertain` when the command timed out.
    pub async fn launch(&self, assignment: &Assignment) -> Result<(), LaunchError> {
        let args = self.run_arguments(assignment);
        let mut command = self.command(&args);
        command.envs(
            assignment
                .environment
                .iter()
                .map(|(name, value)| (name, value)),
        );
        let name = worker_name(&assignment.run);
        execute(command, self.launch_timeout)
            .await
            .map(drop)
            .map_err(|error| LaunchError {
                message: format!(
                    "cannot start {name} with image {}: {}",
                    self.image, error.message
                ),
                uncertain: error.uncertain,
            })
    }

    /// The container's state.
    ///
    /// # Errors
    ///
    /// Fails when the runtime cannot be asked.
    pub async fn state(&self, run: &RunId) -> Result<WorkerState, LaunchError> {
        let name = worker_name(run);
        let format = "{{.State.Status}} {{.State.ExitCode}}";
        match self
            .run(
                &[
                    "inspect",
                    "--type=container",
                    &format!("--format={format}"),
                    &name,
                ],
                COMMAND_TIMEOUT,
            )
            .await
        {
            Ok(output) => Ok(parse_state(&output)),
            Err(error) if is_missing(&error.message) => Ok(WorkerState::Missing),
            Err(error) => Err(error),
        }
    }

    /// The image the container runs: the configured reference, with the
    /// image ID and registry digest the runtime reports. Values the runtime
    /// cannot give are left out.
    pub async fn image(&self, run: &RunId) -> WorkerImage {
        let mut image = WorkerImage {
            reference: self.image.clone(),
            digest: None,
            id: None,
        };
        let name = worker_name(run);
        let args = ["inspect", "--type=container", "--format={{.Image}}", &name];
        match self.run(&args, COMMAND_TIMEOUT).await {
            Ok(output) => image.id = image_id(&output),
            Err(error) => tracing::debug!(%run, %error, "cannot read the worker's image"),
        }
        if let Some(id) = &image.id {
            let args = ["image", "inspect", "--format={{json .RepoDigests}}", id];
            match self.run(&args, COMMAND_TIMEOUT).await {
                Ok(output) => image.digest = repo_digest(&self.image, &output),
                Err(error) => tracing::debug!(%run, %error, "cannot read the image digests"),
            }
        }
        image
    }

    /// Sends SIGTERM, then SIGKILL after `grace`.
    ///
    /// # Errors
    ///
    /// Fails when the runtime refuses; a missing container is not an error.
    pub async fn stop(&self, run: &RunId, grace: Duration) -> Result<(), LaunchError> {
        let time = format!("--time={}", grace.as_secs());
        let timeout = COMMAND_TIMEOUT + grace;
        self.tolerate_missing(self.run(&["stop", &time, &worker_name(run)], timeout).await)
    }

    /// Removes the container.
    ///
    /// # Errors
    ///
    /// Fails when the runtime refuses; a missing container is not an error.
    pub async fn remove(&self, run: &RunId) -> Result<(), LaunchError> {
        self.tolerate_missing(
            self.run(&["rm", "--force", &worker_name(run)], COMMAND_TIMEOUT)
                .await,
        )
    }

    /// The last 40 lines of the container's output, or why there are none.
    pub async fn logs(&self, run: &RunId) -> String {
        let args = ["logs", "--tail=40", &worker_name(run)];
        let mut command = self.command(&args);
        command.stderr(Stdio::piped());
        match tokio::time::timeout(COMMAND_TIMEOUT, command.output()).await {
            Ok(Ok(output)) => {
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                text
            }
            Ok(Err(error)) => format!("cannot read the worker's output: {error}"),
            Err(_) => "reading the worker's output timed out".to_owned(),
        }
    }

    /// Runs whose containers exist, running or not.
    ///
    /// # Errors
    ///
    /// Fails when the runtime cannot be asked.
    pub async fn workers(&self) -> Result<Vec<RunId>, LaunchError> {
        let filter = format!("--filter=label={MANAGED_LABEL}=true");
        let output = self
            .run(
                &["ps", "--all", &filter, "--format={{.Names}}"],
                COMMAND_TIMEOUT,
            )
            .await?;
        Ok(output
            .lines()
            .filter_map(|name| run_of(name.trim()))
            .collect())
    }

    fn tolerate_missing(&self, result: Result<String, LaunchError>) -> Result<(), LaunchError> {
        match result {
            Ok(_) => Ok(()),
            Err(error) if is_missing(&error.message) => Ok(()),
            Err(error) => Err(LaunchError {
                message: format!("{}: {}", self.kind.command(), error.message),
                uncertain: error.uncertain,
            }),
        }
    }

    fn command<S: AsRef<OsStr>>(&self, args: &[S]) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(args).stdin(Stdio::null()).kill_on_drop(true);
        command
    }

    async fn run(&self, args: &[&str], timeout: Duration) -> Result<String, LaunchError> {
        execute(self.command(args), timeout).await
    }
}

const fn runtime_name(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Auto => "auto",
        Runtime::Podman => "podman",
        Runtime::Docker => "docker",
        Runtime::Kubernetes => "kubernetes",
    }
}

/// Runs `command`, returning its standard output.
async fn execute(mut command: Command, timeout: Duration) -> Result<String, LaunchError> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Err(LaunchError::new(format!(
                "cannot run the runtime command: {error}"
            )));
        }
        Err(_) => {
            return Err(LaunchError::uncertain(format!(
                "the runtime command did not finish within {} s",
                timeout.as_secs()
            )));
        }
    };
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(failure(&output))
}

/// A runtime command that exited unsuccessfully, with its explanation.
fn failure(output: &std::process::Output) -> LaunchError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    };
    LaunchError::new(format!("it exited with {}{detail}", output.status))
}

/// Whether a runtime error says the container does not exist.
fn is_missing(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("no such container")
        || lower.contains("no such object")
        || lower.contains("no container with name")
}

/// The image ID in `inspect` output, as `sha256:<hex>`. Docker gives the
/// prefix and Podman does not.
fn image_id(output: &str) -> Option<String> {
    let id = output.trim();
    if id.is_empty() || id.contains(char::is_whitespace) {
        return None;
    }
    Some(if id.contains(':') {
        id.to_owned()
    } else {
        format!("sha256:{id}")
    })
}

/// The registry digest of `reference` in the image's `RepoDigests` (JSON,
/// entries `<repository>@<digest>`): the entry of the same repository, or
/// else the first one.
fn repo_digest(reference: &str, output: &str) -> Option<String> {
    let entries: Vec<String> = serde_json::from_str(output.trim()).ok()?;
    let wanted = repository(reference);
    let entry = entries
        .iter()
        .find(|entry| {
            entry
                .split_once('@')
                .is_some_and(|(repo, _)| repo == wanted)
        })
        .or_else(|| entries.first())?;
    entry.split_once('@').map(|(_, digest)| digest.to_owned())
}

/// `reference` without its tag or digest.
fn repository(reference: &str) -> &str {
    let name = reference
        .split_once('@')
        .map_or(reference, |(name, _)| name);
    match name.rsplit_once(':') {
        Some((repo, tag)) if !tag.contains('/') => repo,
        _ => name,
    }
}

fn parse_state(output: &str) -> WorkerState {
    let mut parts = output.split_whitespace();
    let status = parts.next().unwrap_or_default();
    let code = parts
        .next()
        .and_then(|code| code.parse().ok())
        .unwrap_or(-1);
    match status {
        "running" | "paused" | "stopping" => WorkerState::Running,
        "exited" | "dead" | "stopped" => WorkerState::Exited(code),
        _ => WorkerState::Starting,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_of_both_runtimes_are_understood() {
        assert_eq!(parse_state("running 0\n"), WorkerState::Running);
        assert_eq!(parse_state("exited 3"), WorkerState::Exited(3));
        assert_eq!(parse_state("created 0"), WorkerState::Starting);
        assert_eq!(parse_state("configured 0"), WorkerState::Starting);
        assert!(is_missing("Error: no such container saci-run-1"));
        assert!(is_missing("Error: No such object: saci-run-1"));
        assert!(is_missing(
            "Error: no container with name or ID \"saci-run-1\" found: no such container"
        ));
    }

    #[test]
    fn image_ids_of_both_runtimes_are_understood() {
        let hex = "4b2f8e0c1d3a";
        assert_eq!(
            image_id(&format!("sha256:{hex}\n")),
            Some(format!("sha256:{hex}"))
        );
        assert_eq!(image_id(&format!("{hex}\n")), Some(format!("sha256:{hex}")));
        assert_eq!(image_id("\n"), None);
        assert_eq!(image_id("running 0"), None);
    }

    #[test]
    fn the_digest_of_the_configured_repository_is_chosen() {
        let digests = r#"["docker.io/library/other@sha256:aaa","quay.io/saci/worker@sha256:bbb"]"#;
        let digest = |reference| repo_digest(reference, digests);
        assert_eq!(
            digest("quay.io/saci/worker:1.2"),
            Some("sha256:bbb".to_owned())
        );
        assert_eq!(digest("quay.io/saci/worker"), Some("sha256:bbb".to_owned()));
        assert_eq!(
            digest("quay.io/saci/worker@sha256:bbb"),
            Some("sha256:bbb".to_owned())
        );
        assert_eq!(
            digest("localhost:5000/worker:1"),
            Some("sha256:aaa".to_owned())
        );
        assert_eq!(repository("localhost:5000/worker"), "localhost:5000/worker");
        // A local build has no registry digest.
        assert_eq!(repo_digest("localhost/saci-worker:latest", "[]\n"), None);
        assert_eq!(repo_digest("localhost/saci-worker:latest", "null\n"), None);
    }
}
