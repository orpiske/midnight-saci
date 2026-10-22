//! The YAML configuration file shared by the backend and the CLI.
//!
//! See `docs/configuration.md` for the reference and defaults.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::action::{self, ActionDefinition};
use crate::spec::{
    EnvironmentSpec, ExecutionLimits, FailureAction, JudgeProvider, JudgeSettings,
    KubernetesSecretKey, LlmSettings, McpServerSpec, McpTransport, RetryPolicy, RunPolicy,
    SecretRef, WorkerResources,
};

/// Name of the policy used when a run names none.
pub const DEFAULT_POLICY: &str = "default";

/// Prefix that makes a setting read its value from the named environment
/// variable, for example `env:SACI_LLM_API_KEY`.
pub const ENV_PREFIX: &str = "env:";

/// Variable under which workers receive an LLM key written literally in the
/// configuration file.
pub const LLM_API_KEY_VAR: &str = "SACI_LLM_API_KEY";

/// The backend-default model connection, as written in the configuration
/// file. Runs receive it as [`LlmSettings`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    /// API base URL, or `env:NAME`. Loading replaces `env:NAME` with the
    /// value of `NAME`.
    pub base_url: String,
    /// Explicit tool-capable model identifier.
    pub model: String,
    /// The API credential, or `env:NAME` to read it from `NAME`. When
    /// omitted, no `Authorization` header is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Kubernetes Secret key that supplies the credential to cluster workers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_secret: Option<KubernetesSecretKey>,
}

impl LlmConfig {
    /// The variable that carries the credential to workers: `NAME` for
    /// `env:NAME`, otherwise [`LLM_API_KEY_VAR`].
    #[must_use]
    pub fn api_key_env(&self) -> Option<&str> {
        self.api_key
            .as_deref()
            .map(|key| key.strip_prefix(ENV_PREFIX).unwrap_or(LLM_API_KEY_VAR))
    }

    /// The connection runs receive. It names the credential variable and
    /// never holds the credential itself.
    #[must_use]
    pub fn settings(&self) -> LlmSettings {
        LlmSettings {
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            api_key_env: self.api_key_env().map(str::to_owned),
            api_key_secret: self.api_key_secret.clone(),
        }
    }

    /// Looks up variable `name` with `env`, except that a credential written
    /// literally in the file answers for [`LLM_API_KEY_VAR`].
    #[must_use]
    pub fn lookup(&self, name: &str, env: impl Fn(&str) -> Option<String>) -> Option<String> {
        match &self.api_key {
            Some(key) if name == LLM_API_KEY_VAR && !key.starts_with(ENV_PREFIX) => {
                Some(key.clone())
            }
            _ => env(name),
        }
    }
}

/// The optional System One judge, as written in the configuration file.
/// Runs receive it as [`JudgeSettings`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeConfig {
    /// The service that answers the judge's questions.
    #[serde(default)]
    pub provider: JudgeProvider,
    /// `env:NAME`: the judge credential is read from `NAME`. Required for
    /// TypeSafe; optional for Julia 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Kubernetes Secret key that supplies the credential to cluster workers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_secret: Option<KubernetesSecretKey>,
    /// The judge model identifier; the provider's default when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The complete evaluation endpoint URL, or `env:NAME`; the provider's
    /// default when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Lowest probability, in percent, that each expected result holds for
    /// the judge to pass a step without the model.
    #[serde(default = "default_judge_threshold")]
    pub threshold_percent: u8,
}

/// The default judge endpoint of `saci-julia serve`.
pub const DEFAULT_JULIA_ENDPOINT: &str = "http://127.0.0.1:8095/v1/evaluate";

const fn default_judge_threshold() -> u8 {
    90
}

impl JudgeConfig {
    /// The variable that carries the credential, when one is configured.
    #[must_use]
    pub fn api_key_env(&self) -> Option<&str> {
        self.api_key.as_deref()?.strip_prefix(ENV_PREFIX)
    }

    /// The model identifier, or the provider's default.
    #[must_use]
    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(match self.provider {
            JudgeProvider::Typesafe => "jev-latest",
            JudgeProvider::Julia1 => "julia-1",
        })
    }

    /// The evaluation endpoint, or the provider's default.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        self.endpoint.as_deref().unwrap_or(match self.provider {
            JudgeProvider::Typesafe => "https://api.typesafe.ai/v1/systemone",
            JudgeProvider::Julia1 => DEFAULT_JULIA_ENDPOINT,
        })
    }

    /// The judge runs receive. It names the credential variable and never
    /// holds the credential itself.
    #[must_use]
    pub fn settings(&self) -> JudgeSettings {
        JudgeSettings {
            provider: self.provider,
            endpoint: self.endpoint().to_owned(),
            model: self.model().to_owned(),
            api_key_env: self.api_key_env().map(str::to_owned),
            api_key_secret: self.api_key_secret.clone(),
            threshold_percent: self.threshold_percent,
        }
    }
}

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
    pub llm: LlmConfig,
    /// The optional System One judge; off when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<JudgeConfig>,
    /// Default execution limits; omitted values use built-in defaults.
    #[serde(default)]
    pub limits: LimitsOverride,
    /// Target environments by name.
    #[serde(default)]
    pub environments: BTreeMap<String, EnvironmentConfig>,
    /// Run policies by name. `default` is implied when absent.
    #[serde(default)]
    pub policies: BTreeMap<String, PolicyConfig>,
    /// MCP servers by name; environments enable them under `mcp`.
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpTransport>,
    /// Outcome actions, called by the backend after a run finishes.
    #[serde(default)]
    pub actions: ActionsConfig,
}

/// Outcome actions and what the backend needs to call them
/// (`docs/actions.md`). These servers and secrets are the backend's own:
/// they are never given to workers, and steps cannot call them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionsConfig {
    /// MCP servers reachable from the backend.
    #[serde(default)]
    pub servers: BTreeMap<String, McpTransport>,
    /// Values `${NAME}` expands to in `servers`, read by the backend.
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretRef>,
    /// Actions by name; policies select them under `actions`.
    #[serde(default)]
    pub definitions: BTreeMap<String, ActionDefinition>,
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
    /// When to pull the worker image; the runtime's default when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_pull_policy: Option<PullPolicy>,
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
            image_pull_policy: None,
            kubernetes: KubernetesWorkerConfig::default(),
        }
    }
}

fn default_image() -> String {
    "localhost/saci-worker:latest".to_owned()
}

/// When the worker image is pulled, with the Kubernetes names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullPolicy {
    /// Pull before each run, so a moved tag is followed.
    Always,
    /// Pull only when the image is not present locally.
    IfNotPresent,
    /// Never pull; the image must be present locally.
    Never,
}

impl PullPolicy {
    /// The Kubernetes `imagePullPolicy` value.
    #[must_use]
    pub const fn kubernetes(self) -> &'static str {
        match self {
            Self::Always => "Always",
            Self::IfNotPresent => "IfNotPresent",
            Self::Never => "Never",
        }
    }

    /// The `--pull` value of `podman run` and `docker run`.
    #[must_use]
    pub const fn container(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::IfNotPresent => "missing",
            Self::Never => "never",
        }
    }
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
    /// Image pull policy for worker Jobs; overrides
    /// [`WorkerConfig::image_pull_policy`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_pull_policy: Option<String>,
    /// Kubernetes API URL for a backend outside the cluster, for example
    /// `http://127.0.0.1:8001` from `kubectl proxy`. Inside a pod the
    /// backend uses its service account instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
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
    /// See [`ExecutionLimits::model_request_retries`].
    pub model_request_retries: Option<u32>,
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
            model_request_retries: self
                .model_request_retries
                .unwrap_or(base.model_request_retries),
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
    /// Configured MCP servers enabled for steps in this environment.
    #[serde(default)]
    pub mcp: BTreeMap<String, McpSelection>,
}

/// Enables a configured MCP server for an environment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSelection {
    /// Tools exposed to steps; all discovered tools when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
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
    /// Outcome actions (names under `actions.definitions`) considered when a
    /// run under this policy finishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
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
        Self::from_yaml_with_env(&text, &path.display().to_string(), |name| {
            std::env::var(name).ok()
        })
    }

    /// Parses and validates configuration text. `source_name` labels errors.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Parse`] or [`ConfigError::Invalid`].
    pub fn from_yaml(text: &str, source_name: &str) -> Result<Self, ConfigError> {
        Self::from_yaml_with_env(text, source_name, |_| None)
    }

    /// Like [`Config::from_yaml`], but replaces `llm.base_url: env:NAME`
    /// with the value `env` gives for `NAME`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Parse`] or [`ConfigError::Invalid`].
    pub fn from_yaml_with_env(
        text: &str,
        source_name: &str,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let mut config: Self = serde_yaml::from_str(text).map_err(|error| ConfigError::Parse {
            source_name: source_name.to_owned(),
            message: error.to_string(),
        })?;
        let mut issues = config.resolve_env_references(&env);
        // An unresolved reference is reported once, not again as a bad URL.
        let unresolved = issues.iter().any(|issue| issue.path == "llm.base_url");
        issues.extend(
            config
                .validate()
                .into_iter()
                .filter(|issue| !(unresolved && issue.path == "llm.base_url")),
        );
        if issues.is_empty() {
            Ok(config)
        } else {
            Err(ConfigError::Invalid {
                source_name: source_name.to_owned(),
                issues,
            })
        }
    }

    /// Replaces each `env:NAME` in `llm.base_url`, `judge.endpoint` and in environment
    /// parameters with the value `env` gives for `NAME`. Returns an issue
    /// for each variable that is not set.
    fn resolve_env_references(
        &mut self,
        env: &impl Fn(&str) -> Option<String>,
    ) -> Vec<ConfigIssue> {
        let mut issues = Issues::default();
        resolve_env_reference(&mut self.llm.base_url, "llm.base_url", env, &mut issues);
        if let Some(endpoint) = self.judge.as_mut().and_then(|j| j.endpoint.as_mut()) {
            resolve_env_reference(endpoint, "judge.endpoint", env, &mut issues);
        }
        for (name, environment) in &mut self.environments {
            for (key, value) in &mut environment.parameters {
                let path = format!("environments.{name}.parameters.{key}");
                resolve_env_reference(value, &path, env, &mut issues);
            }
        }
        issues.0
    }

    /// Returns every validation problem; empty when the configuration is valid.
    #[must_use]
    pub fn validate(&self) -> Vec<ConfigIssue> {
        let mut issues = Issues::default();
        validate_backend(&self.backend, &mut issues);
        validate_llm(&self.llm, &mut issues);
        validate_judge(self.judge.as_ref(), &mut issues);
        validate_limits(
            "limits",
            self.limits.apply(ExecutionLimits::default()),
            &mut issues,
        );
        for (name, environment) in &self.environments {
            self.validate_environment(name, environment, &mut issues);
        }
        for (name, policy) in &self.policies {
            let path = format!("policies.{name}");
            validate_policy(&path, policy, &mut issues);
            for action in &policy.actions {
                if !self.actions.definitions.contains_key(action) {
                    let message = format!("`{action}` is not defined under actions.definitions");
                    issues.push(&format!("{path}.actions"), &message);
                }
            }
        }
        for (name, server) in &self.mcp_servers {
            validate_mcp_server(&format!("mcp_servers.{name}"), name, server, &mut issues);
        }
        validate_actions(&self.actions, &mut issues);
        if self.worker.image.trim().is_empty() {
            issues.push("worker.image", "must not be empty");
        }
        issues.0
    }

    fn validate_variable_names(&self, path: &str, env: &EnvironmentConfig, issues: &mut Issues) {
        for key in env.parameters.keys().chain(env.secrets.keys()) {
            if !is_variable_name(key) {
                issues.push(&format!("{path}.{key}"), "must be a shell variable name");
            } else if key.starts_with("SACI_") {
                issues.push(
                    &format!("{path}.{key}"),
                    "names starting with SACI_ are reserved",
                );
            } else if self.llm.api_key_env() == Some(key.as_str()) {
                issues.push(
                    &format!("{path}.{key}"),
                    "carries llm.api_key; the LLM credential is never exported to tools",
                );
            } else if self
                .judge
                .as_ref()
                .is_some_and(|judge| judge.api_key_env() == Some(key.as_str()))
            {
                issues.push(
                    &format!("{path}.{key}"),
                    "carries judge.api_key; the judge credential is never exported to tools",
                );
            }
        }
    }

    fn validate_environment(&self, name: &str, env: &EnvironmentConfig, issues: &mut Issues) {
        let path = format!("environments.{name}");
        if !is_name(name) {
            issues.push(&path, "names use letters, digits, `-` and `_`");
        }
        self.validate_variable_names(&path, env, issues);
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
        self.validate_mcp_selection(&path, env, issues);
    }

    fn validate_mcp_selection(&self, path: &str, env: &EnvironmentConfig, issues: &mut Issues) {
        for (name, selection) in &env.mcp {
            let path = format!("{path}.mcp.{name}");
            if !self.mcp_servers.contains_key(name) {
                issues.push(&path, "is not defined under mcp_servers");
            }
            let tools = selection.tools.iter().flatten();
            if tools.clone().any(|tool| tool.trim().is_empty()) {
                issues.push(&format!("{path}.tools"), "tool names must not be empty");
            }
            if selection.tools.as_ref().is_some_and(Vec::is_empty) {
                issues.push(
                    &format!("{path}.tools"),
                    "omit `tools` to enable all tools; an empty list enables none",
                );
            }
        }
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
            mcp: self.enabled_mcp(env),
        };
        Ok((spec, env.limits.apply(self.default_limits())))
    }

    fn enabled_mcp(&self, env: &EnvironmentConfig) -> BTreeMap<String, McpServerSpec> {
        env.mcp
            .iter()
            .filter_map(|(name, selection)| {
                let transport = self.mcp_servers.get(name)?.clone();
                let tools = selection.tools.clone();
                Some((name.clone(), McpServerSpec { transport, tools }))
            })
            .collect()
    }

    /// The outcome actions of the policy called `name`, by action name, as
    /// recorded with a run when it is accepted.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unknown`] when no such policy exists.
    pub fn policy_actions(
        &self,
        name: &str,
    ) -> Result<BTreeMap<String, ActionDefinition>, ConfigError> {
        let names = match self.policies.get(name) {
            Some(policy) => policy.actions.clone(),
            None if name == DEFAULT_POLICY => Vec::new(),
            None => return Err(unknown("policy", name, self.policies.keys())),
        };
        Ok(names
            .into_iter()
            .filter_map(|action| {
                let definition = self.actions.definitions.get(&action)?.clone();
                Some((action, definition))
            })
            .collect())
    }

    /// The judge settings runs receive, when a judge is configured.
    #[must_use]
    pub fn judge_settings(&self) -> Option<Box<JudgeSettings>> {
        self.judge.as_ref().map(|judge| Box::new(judge.settings()))
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

/// Replaces `value` with the variable it names when it is `env:NAME`.
fn resolve_env_reference(
    value: &mut String,
    path: &str,
    env: &impl Fn(&str) -> Option<String>,
    issues: &mut Issues,
) {
    let Some(name) = value.strip_prefix(ENV_PREFIX) else {
        return;
    };
    match env(name) {
        Some(resolved) => *value = resolved,
        None => issues.push(
            path,
            &format!("names {name}, which is not set in the environment"),
        ),
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

fn validate_llm(llm: &LlmConfig, issues: &mut Issues) {
    if !is_http_url(&llm.base_url) {
        issues.push("llm.base_url", "must be an http:// or https:// URL");
    }
    if llm.model.trim().is_empty() {
        issues.push("llm.model", "must name a tool-capable model");
    }
    let key_variable = llm
        .api_key
        .as_deref()
        .and_then(|key| key.strip_prefix(ENV_PREFIX));
    match (llm.api_key.as_deref(), key_variable) {
        (_, Some(name)) if !is_variable_name(name) => {
            issues.push("llm.api_key", "env: must name an environment variable");
        }
        (Some(key), None) if key.trim().is_empty() => {
            issues.push("llm.api_key", "must not be empty");
        }
        _ => {}
    }
    if llm.api_key_secret.is_some() && key_variable.is_none() {
        issues.push("llm.api_key_secret", "requires llm.api_key: env:NAME");
    }
}

fn validate_judge(judge: Option<&JudgeConfig>, issues: &mut Issues) {
    let Some(judge) = judge else {
        return;
    };
    match &judge.api_key {
        Some(key) if !key.strip_prefix(ENV_PREFIX).is_some_and(is_variable_name) => {
            issues.push("judge.api_key", "must be env:NAME");
        }
        None if judge.provider == JudgeProvider::Typesafe => {
            issues.push("judge.api_key", "is required for the typesafe provider");
        }
        None if judge.api_key_secret.is_some() => {
            issues.push("judge.api_key_secret", "requires judge.api_key: env:NAME");
        }
        _ => {}
    }
    if !is_http_url(judge.endpoint()) {
        issues.push("judge.endpoint", "must be an http:// or https:// URL");
    }
    if judge.model().trim().is_empty() {
        issues.push("judge.model", "must not be empty");
    }
    if !(1..=100).contains(&judge.threshold_percent) {
        issues.push("judge.threshold_percent", "must be from 1 to 100");
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

fn validate_mcp_server(path: &str, name: &str, server: &McpTransport, issues: &mut Issues) {
    if !is_name(name) {
        issues.push(path, "names use letters, digits, `-` and `_`");
    }
    match server {
        McpTransport::Stdio(stdio) => {
            if stdio.command.trim().is_empty() {
                issues.push(&format!("{path}.command"), "must not be empty");
            }
            for key in stdio.env.keys().filter(|key| !is_variable_name(key)) {
                issues.push(
                    &format!("{path}.stdio.env.{key}"),
                    "must be a variable name",
                );
            }
        }
        McpTransport::Http(http) => {
            if !is_http_url(&http.url) {
                issues.push(&format!("{path}.url"), "must be an http:// or https:// URL");
            }
        }
    }
}

fn validate_actions(actions: &ActionsConfig, issues: &mut Issues) {
    for (name, server) in &actions.servers {
        let path = format!("actions.servers.{name}");
        validate_mcp_server(&path, name, server, issues);
    }
    for (name, secret) in &actions.secrets {
        let path = format!("actions.secrets.{name}");
        if !is_variable_name(name) {
            issues.push(&path, "must be a variable name");
        }
        if matches!(secret, SecretRef::Kubernetes { .. }) {
            issues.push(
                &path,
                "the backend does not read Kubernetes Secrets; map the Secret to a backend \
                 variable (backend.extraEnv in the Helm chart) and use { env: VAR }",
            );
        }
        validate_secret(&path, secret, issues);
    }
    for (name, definition) in &actions.definitions {
        validate_action(
            &format!("actions.definitions.{name}"),
            name,
            definition,
            actions,
            issues,
        );
    }
}

fn validate_action(
    path: &str,
    name: &str,
    definition: &ActionDefinition,
    actions: &ActionsConfig,
    issues: &mut Issues,
) {
    if !is_name(name) {
        issues.push(path, "names use letters, digits, `-` and `_`");
    }
    if !actions.servers.contains_key(&definition.server) {
        issues.push(
            &format!("{path}.server"),
            "is not defined under actions.servers",
        );
    }
    if definition.tool.trim().is_empty() {
        issues.push(&format!("{path}.tool"), "must not be empty");
    }
    if definition.on.is_empty() {
        issues.push(
            &format!("{path}.on"),
            "list the run outcomes that trigger the action",
        );
    }
    validate_arguments(path, &definition.arguments, issues);
    if let Some(argument) = &definition.idempotency_key {
        let path = format!("{path}.idempotency_key");
        if argument.trim().is_empty() {
            issues.push(&path, "name the tool argument that carries the key");
        } else if definition.arguments.get(argument).is_some() {
            issues.push(
                &path,
                &format!("`{argument}` is also set under arguments; Saci fills it with the key"),
            );
        }
    }
}

fn validate_arguments(path: &str, arguments: &serde_json::Value, issues: &mut Issues) {
    if !arguments.is_object() {
        issues.push(
            &format!("{path}.arguments"),
            "must be a mapping of argument names",
        );
    }
    for variable in action::variables(arguments) {
        if !action::VARIABLES.contains(&variable.as_str()) {
            let message = format!(
                "unknown variable `{{{{{variable}}}}}`; available: {}",
                action::VARIABLES.join(", ")
            );
            issues.push(&format!("{path}.arguments"), &message);
        }
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
    use crate::status::Outcome;

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
    fn the_image_pull_policy_uses_the_kubernetes_names() {
        let config = Config::from_yaml(MINIMAL, "t.yaml").unwrap();
        assert_eq!(config.worker.image_pull_policy, None);
        let yaml = format!("{MINIMAL}worker:\n  image_pull_policy: IfNotPresent\n");
        let config = Config::from_yaml(&yaml, "t.yaml").unwrap();
        assert_eq!(
            config.worker.image_pull_policy,
            Some(PullPolicy::IfNotPresent)
        );
        let yaml = format!("{MINIMAL}worker:\n  image_pull_policy: latest\n");
        let error = Config::from_yaml(&yaml, "t.yaml").unwrap_err();
        assert!(error.to_string().contains("IfNotPresent"), "{error}");
    }

    #[test]
    fn missing_required_settings_are_reported() {
        let error = Config::from_yaml("llm:\n  model: m\n", "t.yaml").unwrap_err();
        assert!(error.to_string().contains("base_url"), "{error}");
    }

    fn invalid(result: Result<Config, ConfigError>) -> Vec<String> {
        match result {
            Err(ConfigError::Invalid { issues, .. }) => {
                issues.iter().map(ToString::to_string).collect()
            }
            other => panic!("expected validation issues, got {other:?}"),
        }
    }

    #[test]
    fn env_base_url_reads_the_named_variable() {
        let yaml = "llm:\n  base_url: env:MODEL_URL\n  model: m\n";
        let env = |name: &str| (name == "MODEL_URL").then(|| "http://internal:8000/v1".to_owned());
        let config = Config::from_yaml_with_env(yaml, "t.yaml", env).unwrap();
        assert_eq!(config.llm.base_url, "http://internal:8000/v1");
        assert_eq!(config.llm.settings().base_url, "http://internal:8000/v1");
        let config = Config::from_yaml_with_env(MINIMAL, "t.yaml", env).unwrap();
        assert_eq!(config.llm.base_url, "http://ollama:11434/v1");
    }

    #[test]
    fn env_base_url_reports_an_unset_or_invalid_variable() {
        let yaml = "llm:\n  base_url: env:MODEL_URL\n  model: m\n";
        let found = invalid(Config::from_yaml_with_env(yaml, "t.yaml", |_| None));
        assert_eq!(
            found,
            ["llm.base_url: names MODEL_URL, which is not set in the environment"]
        );
        let env = |_: &str| Some("internal:8000".to_owned());
        let found = invalid(Config::from_yaml_with_env(yaml, "t.yaml", env));
        assert_eq!(found, ["llm.base_url: must be an http:// or https:// URL"]);
    }

    #[test]
    fn env_parameters_read_the_named_variable() {
        let yaml = format!(
            "{MINIMAL}environments:\n  staging:\n    parameters:\n      HOST: env:STAGING_HOST\n      PORT: '8443'\n"
        );
        let env = |name: &str| (name == "STAGING_HOST").then(|| "app.internal".to_owned());
        let config = Config::from_yaml_with_env(&yaml, "t.yaml", env).unwrap();
        let (spec, _) = config.environment("staging").unwrap();
        assert_eq!(spec.parameters["HOST"], "app.internal");
        assert_eq!(spec.parameters["PORT"], "8443");

        let found = invalid(Config::from_yaml_with_env(&yaml, "t.yaml", |_| None));
        assert_eq!(
            found,
            [
                "environments.staging.parameters.HOST: names STAGING_HOST, which is not set in the environment"
            ]
        );
    }

    fn llm_with_key(key: &str) -> LlmConfig {
        let yaml = MINIMAL.replace(
            "model: qwen3\n",
            &format!("model: qwen3\n  api_key: {key}\n"),
        );
        Config::from_yaml(&yaml, "t.yaml").unwrap().llm
    }

    fn env_value(name: &str) -> Option<String> {
        Some(format!("from-{name}"))
    }

    #[test]
    fn env_api_key_is_read_from_the_named_variable() {
        let llm = llm_with_key("env:PROVIDER_KEY");
        assert_eq!(llm.settings().api_key_env.as_deref(), Some("PROVIDER_KEY"));
        assert_eq!(
            llm.lookup("PROVIDER_KEY", env_value).as_deref(),
            Some("from-PROVIDER_KEY")
        );
        let config = Config::from_yaml(MINIMAL, "t.yaml").unwrap();
        assert_eq!(config.llm.settings().api_key_env, None);
    }

    #[test]
    fn literal_api_key_reaches_workers_but_not_the_run_spec() {
        let llm = llm_with_key("sk-literal");
        let settings = llm.settings();
        assert_eq!(settings.api_key_env.as_deref(), Some(LLM_API_KEY_VAR));
        assert!(
            !serde_json::to_string(&settings)
                .unwrap()
                .contains("sk-literal")
        );
        assert_eq!(
            llm.lookup(LLM_API_KEY_VAR, env_value).as_deref(),
            Some("sk-literal")
        );
        assert_eq!(
            llm.lookup("OTHER", env_value).as_deref(),
            Some("from-OTHER")
        );
    }

    #[test]
    fn the_judge_is_off_by_default_and_uses_typesafe_defaults() {
        let config = Config::from_yaml(MINIMAL, "t.yaml").unwrap();
        assert_eq!(config.judge_settings(), None);
        let yaml = format!("{MINIMAL}judge:\n  api_key: env:TYPESAFE_KEY\n");
        let settings = Config::from_yaml(&yaml, "t.yaml")
            .unwrap()
            .judge_settings()
            .unwrap();
        assert_eq!(settings.provider, JudgeProvider::Typesafe);
        assert_eq!(settings.api_key_env.as_deref(), Some("TYPESAFE_KEY"));
        assert_eq!(settings.model, "jev-latest");
        assert_eq!(settings.endpoint, "https://api.typesafe.ai/v1/systemone");
        assert_eq!(settings.threshold_percent, 90);
    }

    #[test]
    fn the_julia_judge_needs_no_key_and_uses_its_own_defaults() {
        let yaml = format!("{MINIMAL}judge:\n  provider: julia-1\n");
        let settings = Config::from_yaml(&yaml, "t.yaml")
            .unwrap()
            .judge_settings()
            .unwrap();
        assert_eq!(settings.provider, JudgeProvider::Julia1);
        assert_eq!(settings.api_key_env, None);
        assert_eq!(settings.model, "julia-1");
        assert_eq!(settings.endpoint, DEFAULT_JULIA_ENDPOINT);
        assert_eq!(settings.threshold_percent, 90);

        let yaml = format!(
            "{MINIMAL}judge:\n  provider: julia-1\n  api_key: env:JULIA_TOKEN\n  \
             endpoint: http://judge:8095/v1/evaluate\n  threshold_percent: 80\n"
        );
        let settings = Config::from_yaml(&yaml, "t.yaml")
            .unwrap()
            .judge_settings()
            .unwrap();
        assert_eq!(settings.api_key_env.as_deref(), Some("JULIA_TOKEN"));
        assert_eq!(settings.endpoint, "http://judge:8095/v1/evaluate");
        assert_eq!(settings.threshold_percent, 80);
    }

    #[test]
    fn the_typesafe_judge_requires_a_key_and_providers_are_checked() {
        let found = issues(&format!("{MINIMAL}judge:\n  threshold_percent: 90\n"));
        assert!(
            found.iter().any(|i| i.starts_with("judge.api_key:")),
            "{found:?}"
        );
        let found = issues(&format!(
            "{MINIMAL}judge:\n  provider: julia-1\n  api_key_secret:\n    name: s\n    key: k\n"
        ));
        assert!(
            found.iter().any(|i| i.starts_with("judge.api_key_secret:")),
            "{found:?}"
        );
        let unknown = format!("{MINIMAL}judge:\n  provider: other\n");
        assert!(Config::from_yaml(&unknown, "t.yaml").is_err());
    }

    #[test]
    fn invalid_judge_settings_are_reported() {
        let judge = "judge:\n  api_key: sk-literal\n  endpoint: ftp://x\n  model: ' '\n  \
                     threshold_percent: 0\n";
        let found = issues(&format!("{MINIMAL}{judge}"));
        for path in ["api_key", "endpoint", "model", "threshold_percent"] {
            let prefix = format!("judge.{path}:");
            assert!(found.iter().any(|i| i.starts_with(&prefix)), "{found:?}");
        }
        let exported = format!(
            "{MINIMAL}judge:\n  api_key: env:JUDGE_KEY\nenvironments:\n  e:\n    parameters:\n      \
             JUDGE_KEY: x\n"
        );
        let found = issues(&exported);
        assert!(
            found.iter().any(|i| i.contains("carries judge.api_key")),
            "{found:?}"
        );
    }

    #[test]
    fn invalid_api_keys_are_reported() {
        let with =
            |extra: &str| MINIMAL.replace("model: qwen3\n", &format!("model: qwen3\n{extra}"));
        assert!(
            issues(&with("  api_key: env:not-a-name\n"))
                .iter()
                .any(|i| i.starts_with("llm.api_key: env:"))
        );
        assert!(
            issues(&with("  api_key: ' '\n"))
                .iter()
                .any(|i| i.starts_with("llm.api_key: must not be empty"))
        );
        let secret = "  api_key_secret:\n    name: s\n    key: k\n";
        assert!(
            issues(&with(&format!("  api_key: sk-literal\n{secret}")))
                .iter()
                .any(|i| i.starts_with("llm.api_key_secret"))
        );
        assert!(Config::from_yaml(&with(&format!("  api_key: env:K\n{secret}")), "t.yaml").is_ok());
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

    #[test]
    fn tool_variables_cannot_shadow_worker_credentials() {
        let yaml = format!(
            "{MINIMAL}environments:\n  local:\n    parameters:\n      SACI_WORKER_TOKEN: x\n    secrets:\n      LLM_KEY:\n        env: K\n"
        )
        .replace("model: qwen3\n", "model: qwen3\n  api_key: env:LLM_KEY\n");
        let found = issues(&yaml);
        assert!(
            found.iter().any(|i| i.contains("SACI_ are reserved")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|i| i.contains("carries llm.api_key")),
            "{found:?}"
        );
    }

    const MCP: &str = "mcp_servers:\n  tools:\n    command: saci-fake-mcp\n    env:\n      TOKEN: ${MCP_TOKEN}\n  remote:\n    url: http://mcp:9090/mcp\n    headers:\n      Authorization: Bearer ${MCP_TOKEN}\nenvironments:\n  local:\n    mcp:\n      tools:\n        tools: [echo]\n      remote: {}\n";

    #[test]
    fn mcp_servers_are_enabled_per_environment() {
        let config = Config::from_yaml(&format!("{MINIMAL}{MCP}"), "test.yaml").unwrap();
        let (spec, _) = config.environment("local").unwrap();
        assert_eq!(
            spec.mcp["tools"].tools.as_deref(),
            Some(&["echo".to_owned()][..])
        );
        assert!(matches!(
            spec.mcp["tools"].transport,
            McpTransport::Stdio(_)
        ));
        assert_eq!(spec.mcp["remote"].tools, None);
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            json["mcp"]["remote"]["transport"]["url"],
            "http://mcp:9090/mcp"
        );
    }

    const ACTIONS: &str = "actions:\n  servers:\n    tickets:\n      url: http://tickets:9090/mcp\n      headers:\n        Authorization: Bearer ${TICKETS_TOKEN}\n  secrets:\n    TICKETS_TOKEN: { env: SACI_TICKETS_TOKEN }\n  definitions:\n    open-ticket:\n      server: tickets\n      tool: create_issue\n      on: [failed, error]\n      approval: required\n      arguments:\n        title: '{{plan.title}}: {{run.outcome}}'\npolicies:\n  nightly:\n    actions: [open-ticket]\n";

    #[test]
    fn policies_select_outcome_actions() {
        let config = Config::from_yaml(&format!("{MINIMAL}{ACTIONS}"), "test.yaml").unwrap();
        let actions = config.policy_actions("nightly").unwrap();
        let ticket = &actions["open-ticket"];
        assert!(ticket.triggered_by(Outcome::Error));
        assert!(!ticket.triggered_by(Outcome::Passed));
        assert_eq!(ticket.approval, crate::action::Approval::Required);
        assert!(config.policy_actions(DEFAULT_POLICY).unwrap().is_empty());
        assert!(config.policy_actions("missing").is_err());
        let policy = serde_json::to_value(config.policy("nightly").unwrap()).unwrap();
        assert!(policy.get("actions").is_none(), "workers never see actions");
    }

    #[test]
    fn outcome_actions_are_validated() {
        let bad = format!(
            "{MINIMAL}actions:\n  secrets:\n    TOKEN: {{ secret: tickets, key: token }}\n  definitions:\n    ticket:\n      server: missing\n      tool: ''\n      on: []\n      idempotency_key: title\n      arguments:\n        title: '{{{{run.secret}}}}'\npolicies:\n  p:\n    actions: [ticket, ghost]\n"
        );
        let found = issues(&bad);
        for expected in [
            "actions.secrets.TOKEN: the backend does not read Kubernetes Secrets",
            "actions.definitions.ticket.server: is not defined",
            "actions.definitions.ticket.tool: must not be empty",
            "actions.definitions.ticket.on: list the run outcomes",
            "actions.definitions.ticket.arguments: unknown variable `{{run.secret}}`",
            "policies.p.actions: `ghost` is not defined",
            "actions.definitions.ticket.idempotency_key: `title` is also set",
        ] {
            assert!(
                found.iter().any(|i| i.starts_with(expected)),
                "{expected}: {found:?}"
            );
        }
    }

    #[test]
    fn mcp_configuration_is_validated() {
        let bad = format!(
            "{MINIMAL}mcp_servers:\n  bad name:\n    command: ''\n  web:\n    url: mcp:9090\nenvironments:\n  local:\n    mcp:\n      missing: {{}}\n      web:\n        tools: []\n"
        );
        let found = issues(&bad);
        for expected in [
            "mcp_servers.bad name: names use",
            "mcp_servers.bad name.command: must not be empty",
            "mcp_servers.web.url: must be",
            "environments.local.mcp.missing: is not defined",
            "environments.local.mcp.web.tools: omit",
        ] {
            assert!(
                found.iter().any(|i| i.starts_with(expected)),
                "{expected}: {found:?}"
            );
        }
    }
}
