//! The immutable run specification handed to a worker, and the settings it
//! is assembled from.
//!
//! Every type here is safe to store and show: secrets appear only as
//! references, never as values.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::RunId;
use crate::plan::PlanBundle;
use crate::status::Outcome;

/// Version of the run specification, event and report formats.
pub const SCHEMA_VERSION: u32 = 1;

/// Everything a worker needs to execute one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    /// Format version; see [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The run being executed.
    pub run_id: RunId,
    /// When the backend accepted the run.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// The validated plan documents.
    pub bundle: PlanBundle,
    /// The selected target environment (secret references only).
    pub environment: EnvironmentSpec,
    /// How to respond to outcomes.
    pub policy: RunPolicy,
    /// Effective execution limits.
    pub limits: ExecutionLimits,
    /// The model connection.
    pub llm: LlmSettings,
}

/// Where a secret value comes from. Only the reference is ever stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum SecretRef {
    /// An environment variable of the backend process.
    Env {
        /// Variable name.
        env: String,
    },
    /// A key of a Kubernetes Secret in the backend's namespace.
    Kubernetes {
        /// Secret name.
        secret: String,
        /// Key within the Secret.
        key: String,
    },
}

/// A target environment as recorded with a run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSpec {
    /// Environment name from the configuration.
    pub name: String,
    /// Non-secret parameters, exported to the shell and shown to the model.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, String>,
    /// Secrets exported to the shell under these names; values are injected
    /// into the worker and redacted from evidence.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, SecretRef>,
    /// Worker container resources.
    #[serde(default)]
    pub worker: WorkerResources,
}

/// Resources requested for the worker container.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerResources {
    /// CPU limit, for example `"1"` or `"500m"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    /// Memory limit, for example `"1Gi"` or `"512m"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

/// The OpenAI-compatible model connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmSettings {
    /// API base URL including any prefix such as `/v1`; `chat/completions`
    /// is appended once.
    pub base_url: String,
    /// Explicit tool-capable model identifier.
    pub model: String,
    /// Environment variable holding the API credential. When omitted, no
    /// `Authorization` header is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Kubernetes Secret key that supplies `api_key_env` to cluster workers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_secret: Option<KubernetesSecretKey>,
}

/// A key within a Kubernetes Secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KubernetesSecretKey {
    /// Secret name.
    pub name: String,
    /// Key within the Secret.
    pub key: String,
}

/// Finite execution limits. Every limit is configurable; see
/// `docs/configuration.md` for the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    /// Deadline for the whole run, enforced by the backend and the worker.
    pub run_timeout_secs: u64,
    /// Deadline for one attempt of one unit.
    pub step_timeout_secs: u64,
    /// Deadline for one tool operation (shell command, HTTP request, MCP call).
    pub operation_timeout_secs: u64,
    /// Deadline for one model request.
    pub model_request_timeout_secs: u64,
    /// Model requests allowed per attempt, including rejected ones.
    pub model_calls_per_step: u32,
    /// Tool calls allowed per attempt, including rejected ones.
    pub tool_calls_per_step: u32,
    /// Bytes of each captured output stream or body kept as evidence.
    pub output_limit_bytes: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            run_timeout_secs: 3600,
            step_timeout_secs: 600,
            operation_timeout_secs: 120,
            model_request_timeout_secs: 120,
            model_calls_per_step: 25,
            tool_calls_per_step: 40,
            output_limit_bytes: 64 * 1024,
        }
    }
}

/// What to do after an attempt finishes unsuccessfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureAction {
    /// Stop and mark remaining units skipped (the default).
    #[default]
    Stop,
    /// Record the result and continue with the next unit.
    Continue,
}

/// Explicit retry behavior. The default is a single attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    /// Attempts allowed per unit, including the first.
    pub max_attempts: u32,
    /// Outcomes that permit another attempt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on: Vec<Outcome>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            on: Vec::new(),
        }
    }
}

/// How a run responds to outcomes. Separate from the environment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunPolicy {
    /// Policy name from the configuration.
    #[serde(default)]
    pub name: String,
    /// Behavior after an unsuccessful unit.
    #[serde(default)]
    pub on_failure: FailureAction,
    /// Retry behavior.
    #[serde(default)]
    pub retry: RetryPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_one_attempt_and_stop() {
        let policy = RunPolicy::default();
        assert_eq!(policy.on_failure, FailureAction::Stop);
        assert_eq!(policy.retry.max_attempts, 1);
        assert!(policy.retry.on.is_empty());
    }

    #[test]
    fn secret_references_never_hold_values() {
        let secret: SecretRef = serde_json::from_str(r#"{"env":"TOKEN"}"#).unwrap();
        assert_eq!(
            secret,
            SecretRef::Env {
                env: "TOKEN".into()
            }
        );
        let kube: SecretRef = serde_json::from_str(r#"{"secret":"s","key":"k"}"#).unwrap();
        assert!(matches!(kube, SecretRef::Kubernetes { .. }));
        assert!(serde_json::from_str::<SecretRef>(r#"{"value":"hunter2"}"#).is_err());
    }
}
