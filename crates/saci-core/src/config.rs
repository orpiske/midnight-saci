//! The YAML configuration file shared by the backend and the CLI.
//!
//! See `docs/configuration.md` for the reference and defaults.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::spec::{
    EnvironmentSpec, ExecutionLimits, FailureAction, LlmSettings, RetryPolicy, RunPolicy,
    SecretRef, WorkerResources,
};

/// Name of the policy used when a run names none.
pub const DEFAULT_POLICY: &str = "default";

/// The complete configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Backend service settings.
    #[serde(default)]
    pub backend: BackendConfig,
    /// How workers are launched.
    #[serde(default)]
    pub worker: WorkerConfig,
    /// The backend-default model connection inherited by workers.
    pub llm: LlmSettings,
    /// Default execution limits; omitted values use built-in defaults.
    #[serde(default)]
    pub limits: LimitsOverride,
    /// Target environments by name.
    #[serde(default)]
    pub environments: BTreeMap<String, EnvironmentConfig>,
    /// Run policies by name. `default` is implied when absent.
    #[serde(default)]
    pub policies: BTreeMap<String, PolicyConfig>,
}

/// Backend service settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendConfig {
    /// Socket address the REST API listens on.
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Directory holding the SQLite database and run data.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    /// Backend base URL as reachable from workers, for example
    /// `http://host.containers.internal:8080`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_url: Option<String>,
    /// Client authentication; required when `listen` is not loopback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            data_dir: default_data_dir(),
            worker_url: None,
            auth: None,
        }
    }
}

fn default_listen() -> String {
    "127.0.0.1:8080".to_owned()
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("saci-data")
}

/// Client authentication settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Environment variable holding the bearer token clients must send.
    pub token_env: String,
}

/// Which worker launcher to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    /// `podman` if found in `PATH`, otherwise `docker`.
    #[default]
    Auto,
    /// The `podman` command.
    Podman,
    /// The `docker` command.
    Docker,
    /// Kubernetes Jobs in the backend's namespace.
    Kubernetes,
}

/// How workers are launched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Launcher selection.
    #[serde(default)]
    pub runtime: Runtime,
    /// Worker image reference.
    #[serde(default = "default_image")]
    pub image: String,
    /// Container network to attach (`--network`), for local runtimes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    /// Extra `host:address` entries (`--add-host`), for local runtimes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_hosts: Vec<String>,
    /// Kubernetes launcher settings.
    #[serde(default)]
    pub kubernetes: KubernetesWorkerConfig,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            runtime: Runtime::Auto,
            image: default_image(),
            network: None,
            add_hosts: Vec::new(),
            kubernetes: KubernetesWorkerConfig::default(),
        }
    }
}

fn default_image() -> String {
    "localhost/saci-worker:latest".to_owned()
}

/// Kubernetes launcher settings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KubernetesWorkerConfig {
    /// Namespace for worker Jobs; defaults to the backend's own namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Service account the worker Jobs run as (not the backend's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
    /// Image pull policy for worker Jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_pull_policy: Option<String>,
}

/// Optional overrides of [`ExecutionLimits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsOverride {
    /// See [`ExecutionLimits::run_timeout_secs`].
    pub run_timeout_secs: Option<u64>,
    /// See [`ExecutionLimits::step_timeout_secs`].
    pub step_timeout_secs: Option<u64>,
    /// See [`ExecutionLimits::operation_timeout_secs`].
    pub operation_timeout_secs: Option<u64>,
    /// See [`ExecutionLimits::model_request_timeout_secs`].
    pub model_request_timeout_secs: Option<u64>,
    /// See [`ExecutionLimits::model_calls_per_step`].
    pub model_calls_per_step: Option<u32>,
    /// See [`ExecutionLimits::tool_calls_per_step`].
    pub tool_calls_per_step: Option<u32>,
    /// See [`ExecutionLimits::output_limit_bytes`].
    pub output_limit_bytes: Option<u64>,
}

impl LimitsOverride {
    /// Applies these overrides on top of `base`.
    #[must_use]
    pub fn apply(&self, base: ExecutionLimits) -> ExecutionLimits {
        ExecutionLimits {
            run_timeout_secs: self.run_timeout_secs.unwrap_or(base.run_timeout_secs),
            step_timeout_secs: self.step_timeout_secs.unwrap_or(base.step_timeout_secs),
            operation_timeout_secs: self
                .operation_timeout_secs
                .unwrap_or(base.operation_timeout_secs),
            model_request_timeout_secs: self
                .model_request_timeout_secs
                .unwrap_or(base.model_request_timeout_secs),
            model_calls_per_step: self
                .model_calls_per_step
                .unwrap_or(base.model_calls_per_step),
            tool_calls_per_step: self.tool_calls_per_step.unwrap_or(base.tool_calls_per_step),
            output_limit_bytes: self.output_limit_bytes.unwrap_or(base.output_limit_bytes),
        }
    }
}

/// A target environment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentConfig {
    /// Free-form description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Non-secret parameters exported to the worker shell.
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    /// Secrets exported to the worker shell, by variable name.
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretRef>,
    /// Limits for runs in this environment, over the top-level limits.
    #[serde(default)]
    pub limits: LimitsOverride,
    /// Worker container resources.
    #[serde(default)]
    pub worker: WorkerResources,
}

/// A run policy.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Behavior after an unsuccessful unit; `stop` by default.
    #[serde(default)]
    pub on_failure: FailureAction,
    /// Retry behavior; a single attempt by default.
    #[serde(default)]
    pub retry: RetryPolicy,
}

/// The configuration could not be read or is invalid.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read configuration {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The YAML is malformed or has unknown, missing or mistyped fields.
    #[error("invalid configuration {source_name}: {message}")]
    Parse {
        /// The file or source name.
        source_name: String,
        /// The parser's message, including its location.
        message: String,
    },
    /// Values are well-formed but not acceptable.
    #[error("invalid configuration {source_name}:\n{}", format_issues(.issues))]
    Invalid {
        /// The file or source name.
        source_name: String,
        /// Every problem found.
        issues: Vec<ConfigIssue>,
    },
    /// A requested environment or policy does not exist.
    #[error("unknown {kind} `{name}`; configured: {available}")]
    Unknown {
        /// `environment` or `policy`.
        kind: &'static str,
        /// The requested name.
        name: String,
        /// The configured names.
        available: String,
    },
}

/// One validation problem at a configuration path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    /// Dotted path of the setting, for example `limits.step_timeout_secs`.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

fn format_issues(issues: &[ConfigIssue]) -> String {
    issues
        .iter()
        .map(|issue| format!("  - {issue}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Config {
    /// Reads and validates a configuration file.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the file cannot be read, does not parse,
    /// or fails validation.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::from_yaml(&text, &path.display().to_string())
    }

    /// Parses and validates configuration text. `source_name` labels errors.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Parse`] or [`ConfigError::Invalid`].
    pub fn from_yaml(text: &str, source_name: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_yaml::from_str(text).map_err(|error| ConfigError::Parse {
            source_name: source_name.to_owned(),
            message: error.to_string(),
        })?;
        let issues = config.validate();
        if issues.is_empty() {
            Ok(config)
        } else {
            Err(ConfigError::Invalid {
                source_name: source_name.to_owned(),
                issues,
            })
        }
    }

    /// Returns every validation problem; empty when the configuration is valid.
    #[must_use]
    pub fn validate(&self) -> Vec<ConfigIssue> {
        let mut issues = Issues::default();
        validate_backend(&self.backend, &mut issues);
        validate_llm(&self.llm, &mut issues);
        validate_limits(
            "limits",
            self.limits.apply(ExecutionLimits::default()),
            &mut issues,
        );
        for (name, environment) in &self.environments {
            self.validate_environment(name, environment, &mut issues);
        }
        for (name, policy) in &self.policies {
            validate_policy(&format!("policies.{name}"), policy, &mut issues);
        }
        if self.worker.image.trim().is_empty() {
            issues.push("worker.image", "must not be empty");
        }
        issues.0
    }

    fn validate_environment(&self, name: &str, env: &EnvironmentConfig, issues: &mut Issues) {
        let path = format!("environments.{name}");
        if !is_name(name) {
            issues.push(&path, "names use letters, digits, `-` and `_`");
        }
        for key in env.parameters.keys().chain(env.secrets.keys()) {
            if !is_variable_name(key) {
                issues.push(&format!("{path}.{key}"), "must be a shell variable name");
            }
        }
        for key in env
            .secrets
            .keys()
            .filter(|key| env.parameters.contains_key(*key))
        {
            issues.push(
                &format!("{path}.secrets.{key}"),
                "also defined as a parameter",
            );
        }
        for (key, secret) in &env.secrets {
            validate_secret(&format!("{path}.secrets.{key}"), secret, issues);
        }
        let limits = env.limits.apply(self.default_limits());
        validate_limits(&format!("{path}.limits"), limits, issues);
    }

    /// The top-level limits applied over the built-in defaults.
    #[must_use]
    pub fn default_limits(&self) -> ExecutionLimits {
        self.limits.apply(ExecutionLimits::default())
    }

    /// The environment called `name`, as recorded in a run specification.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unknown`] when no such environment exists.
    pub fn environment(
        &self,
        name: &str,
    ) -> Result<(EnvironmentSpec, ExecutionLimits), ConfigError> {
        let env = self
            .environments
            .get(name)
            .ok_or_else(|| unknown("environment", name, self.environments.keys()))?;
        let spec = EnvironmentSpec {
            name: name.to_owned(),
            parameters: env.parameters.clone(),
            secrets: env.secrets.clone(),
            worker: env.worker.clone(),
        };
        Ok((spec, env.limits.apply(self.default_limits())))
    }

    /// The policy called `name`. `default` exists even when not configured.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unknown`] when no such policy exists.
    pub fn policy(&self, name: &str) -> Result<RunPolicy, ConfigError> {
        let policy = match self.policies.get(name) {
            Some(policy) => policy.clone(),
            None if name == DEFAULT_POLICY => PolicyConfig::default(),
            None => return Err(unknown("policy", name, self.policies.keys())),
        };
        Ok(RunPolicy {
            name: name.to_owned(),
            on_failure: policy.on_failure,
            retry: policy.retry,
        })
    }
}

fn unknown<'a>(
    kind: &'static str,
    name: &str,
    names: impl Iterator<Item = &'a String>,
) -> ConfigError {
    let available: Vec<&str> = names.map(String::as_str).collect();
    ConfigError::Unknown {
        kind,
        name: name.to_owned(),
        available: if available.is_empty() {
            "none".to_owned()
        } else {
            available.join(", ")
        },
    }
}

#[derive(Default)]
struct Issues(Vec<ConfigIssue>);

impl Issues {
    fn push(&mut self, path: &str, message: &str) {
        self.0.push(ConfigIssue {
            path: path.to_owned(),
            message: message.to_owned(),
        });
    }
}

fn validate_backend(backend: &BackendConfig, issues: &mut Issues) {
    match backend.listen.parse::<std::net::SocketAddr>() {
        Err(_) => issues.push(
            "backend.listen",
            "must be an address such as 127.0.0.1:8080",
        ),
        Ok(address) if !address.ip().is_loopback() && backend.auth.is_none() => {
            issues.push("backend.auth", "required when listening beyond loopback");
        }
        Ok(_) => {}
    }
    if let Some(url) = &backend.worker_url
        && !is_http_url(url)
    {
        issues.push("backend.worker_url", "must be an http:// or https:// URL");
    }
    if let Some(auth) = &backend.auth
        && !is_variable_name(&auth.token_env)
    {
        issues.push(
            "backend.auth.token_env",
            "must be an environment variable name",
        );
    }
}

fn validate_llm(llm: &LlmSettings, issues: &mut Issues) {
    if !is_http_url(&llm.base_url) {
        issues.push("llm.base_url", "must be an http:// or https:// URL");
    }
    if llm.model.trim().is_empty() {
        issues.push("llm.model", "must name a tool-capable model");
    }
    if let Some(name) = &llm.api_key_env
        && !is_variable_name(name)
    {
        issues.push("llm.api_key_env", "must be an environment variable name");
    }
    if llm.api_key_secret.is_some() && llm.api_key_env.is_none() {
        issues.push("llm.api_key_secret", "requires llm.api_key_env");
    }
}

fn validate_limits(path: &str, limits: ExecutionLimits, issues: &mut Issues) {
    let values = [
        ("run_timeout_secs", limits.run_timeout_secs),
        ("step_timeout_secs", limits.step_timeout_secs),
        ("operation_timeout_secs", limits.operation_timeout_secs),
        (
            "model_request_timeout_secs",
            limits.model_request_timeout_secs,
        ),
        (
            "model_calls_per_step",
            u64::from(limits.model_calls_per_step),
        ),
        ("tool_calls_per_step", u64::from(limits.tool_calls_per_step)),
        ("output_limit_bytes", limits.output_limit_bytes),
    ];
    for (name, value) in values {
        if value == 0 {
            issues.push(&format!("{path}.{name}"), "must be greater than 0");
        }
    }
    if limits.step_timeout_secs > limits.run_timeout_secs {
        issues.push(
            &format!("{path}.step_timeout_secs"),
            "exceeds run_timeout_secs",
        );
    }
}

fn validate_policy(path: &str, policy: &PolicyConfig, issues: &mut Issues) {
    let retry = &policy.retry;
    if retry.max_attempts == 0 {
        issues.push(&format!("{path}.retry.max_attempts"), "must be at least 1");
    }
    if retry.max_attempts > 1 && retry.on.is_empty() {
        issues.push(
            &format!("{path}.retry.on"),
            "list the outcomes that allow a retry",
        );
    }
    for outcome in &retry.on {
        if !outcome.is_unsuccessful() {
            let message = format!("`{outcome}` cannot trigger a retry");
            issues.push(&format!("{path}.retry.on"), &message);
        }
    }
}

fn validate_secret(path: &str, secret: &SecretRef, issues: &mut Issues) {
    match secret {
        SecretRef::Env { env } if !is_variable_name(env) => {
            issues.push(path, "env must be an environment variable name");
        }
        SecretRef::Kubernetes { secret, key } if secret.is_empty() || key.is_empty() => {
            issues.push(path, "secret and key must not be empty");
        }
        _ => {}
    }
}

fn is_http_url(value: &str) -> bool {
    let rest = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"));
    rest.is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}

/// Whether `name` is a portable shell variable name.
#[must_use]
pub fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "llm:\n  base_url: http://ollama:11434/v1\n  model: qwen3\n";

    fn issues(yaml: &str) -> Vec<String> {
        match Config::from_yaml(yaml, "test.yaml") {
            Err(ConfigError::Invalid { issues, .. }) => {
                issues.iter().map(ToString::to_string).collect()
            }
            other => panic!("expected validation issues, got {other:?}"),
        }
    }

    #[test]
    fn minimal_configuration_uses_defaults() {
        let config = Config::from_yaml(MINIMAL, "test.yaml").unwrap();
        assert_eq!(config.backend.listen, "127.0.0.1:8080");
        assert_eq!(config.default_limits(), ExecutionLimits::default());
        let policy = config.policy(DEFAULT_POLICY).unwrap();
        assert_eq!(policy.retry.max_attempts, 1);
        assert_eq!(policy.on_failure, FailureAction::Stop);
    }

    #[test]
    fn unknown_fields_are_rejected_with_their_name() {
        let error = Config::from_yaml(&format!("{MINIMAL}budget: 10\n"), "t.yaml").unwrap_err();
        assert!(error.to_string().contains("budget"), "{error}");
        let nested = format!("{MINIMAL}limits:\n  step_timeout: 5\n");
        let error = Config::from_yaml(&nested, "t.yaml").unwrap_err();
        assert!(error.to_string().contains("step_timeout"), "{error}");
    }

    #[test]
    fn missing_required_settings_are_reported() {
        let error = Config::from_yaml("llm:\n  model: m\n", "t.yaml").unwrap_err();
        assert!(error.to_string().contains("base_url"), "{error}");
    }

    #[test]
    fn invalid_limits_and_urls_are_reported() {
        let yaml = "llm:\n  base_url: ftp://x\n  model: ''\nlimits:\n  tool_calls_per_step: 0\n  step_timeout_secs: 7200\n";
        let found = issues(yaml);
        assert!(found.iter().any(|i| i.starts_with("llm.base_url")));
        assert!(found.iter().any(|i| i.starts_with("llm.model")));
        assert!(found.contains(&"limits.tool_calls_per_step: must be greater than 0".to_owned()));
        assert!(found.iter().any(|i| i.contains("exceeds run_timeout_secs")));
    }

    #[test]
    fn exposed_backends_require_authentication() {
        let yaml = format!("{MINIMAL}backend:\n  listen: 0.0.0.0:8080\n");
        assert_eq!(
            issues(&yaml),
            ["backend.auth: required when listening beyond loopback"]
        );
    }

    #[test]
    fn retries_must_be_explicit() {
        let yaml = format!("{MINIMAL}policies:\n  flaky:\n    retry:\n      max_attempts: 3\n");
        assert_eq!(
            issues(&yaml),
            ["policies.flaky.retry.on: list the outcomes that allow a retry"]
        );
        let passed = format!(
            "{MINIMAL}policies:\n  p:\n    retry:\n      max_attempts: 2\n      on: [passed]\n"
        );
        assert!(issues(&passed)[0].contains("`passed` cannot trigger a retry"));
    }

    #[test]
    fn environment_limits_override_top_level_limits() {
        let yaml = format!(
            "{MINIMAL}limits:\n  step_timeout_secs: 300\nenvironments:\n  local:\n    limits:\n      operation_timeout_secs: 5\n"
        );
        let config = Config::from_yaml(&yaml, "t.yaml").unwrap();
        let (_, limits) = config.environment("local").unwrap();
        assert_eq!(limits.step_timeout_secs, 300);
        assert_eq!(limits.operation_timeout_secs, 5);
        assert!(config.environment("staging").is_err());
    }

    #[test]
    fn secret_and_parameter_names_are_checked() {
        let yaml = format!(
            "{MINIMAL}environments:\n  local:\n    parameters:\n      TOKEN: x\n      bad-name: y\n    secrets:\n      TOKEN:\n        env: TARGET_TOKEN\n"
        );
        let found = issues(&yaml);
        assert!(
            found
                .iter()
                .any(|i| i.starts_with("environments.local.bad-name"))
        );
        assert!(
            found
                .iter()
                .any(|i| i.contains("also defined as a parameter"))
        );
    }
}
