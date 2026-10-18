//! Read-only views served to clients such as the web console: the
//! environments and policies a run may name, and the plan a run executes.
//!
//! Every view is built from an allowlist of fields. Secrets, secret
//! references, environment parameters, MCP connection details and action
//! argument templates never appear.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::action::Approval;
use crate::bundle::expand;
use crate::config::{Config, DEFAULT_POLICY, PolicyConfig};
use crate::ids::{RunId, UnitKey};
use crate::parser::Diagnostic;
use crate::plan::{CodeBlock, Criterion, PlanBundle, ProcedureRef, SourceLocation, UnitKind};
use crate::spec::{ExecutionLimits, FailureAction, RetryPolicy};
use crate::status::Outcome;

/// A collection served whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Items<T> {
    /// The items.
    pub items: Vec<T>,
}

/// A configured environment, as `GET /api/v1/environments` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    /// Name to submit runs with.
    pub name: String,
    /// The configured description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Effective limits for runs in this environment.
    pub limits: ExecutionLimits,
    /// Names of the MCP servers steps may use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<String>,
}

/// A run policy, as `GET /api/v1/policies` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyInfo {
    /// Name to submit runs with.
    pub name: String,
    /// False for the implied `default` policy.
    pub configured: bool,
    /// Behavior after an unsuccessful unit.
    pub on_failure: FailureAction,
    /// Retry behavior.
    pub retry: RetryPolicy,
    /// Outcome actions considered when a run under this policy finishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionInfo>,
}

/// What a policy's outcome action does, without its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionInfo {
    /// Configured action name.
    pub name: String,
    /// Server under `actions.servers`.
    pub server: String,
    /// Tool called.
    pub tool: String,
    /// Run outcomes that trigger it.
    pub on: Vec<Outcome>,
    /// Whether a person approves it first.
    pub approval: Approval,
}

/// Every configured environment, by name.
#[must_use]
pub fn environments(config: &Config) -> Vec<EnvironmentInfo> {
    config
        .environments
        .iter()
        .map(|(name, environment)| EnvironmentInfo {
            name: name.clone(),
            description: environment.description.clone(),
            limits: environment.limits.apply(config.default_limits()),
            mcp_servers: environment.mcp.keys().cloned().collect(),
        })
        .collect()
}

/// Every policy by name, including `default` when it is only implied.
#[must_use]
pub fn policies(config: &Config) -> Vec<PolicyInfo> {
    let implied = (!config.policies.contains_key(DEFAULT_POLICY))
        .then(|| (DEFAULT_POLICY.to_owned(), PolicyConfig::default(), false));
    let configured = config
        .policies
        .iter()
        .map(|(name, policy)| (name.clone(), policy.clone(), true));
    let mut all: Vec<_> = implied.into_iter().chain(configured).collect();
    all.sort_by(|a, b| a.0.cmp(&b.0));
    all.into_iter()
        .map(|(name, policy, configured)| PolicyInfo {
            name,
            configured,
            on_failure: policy.on_failure,
            retry: policy.retry,
            actions: action_infos(config, &policy.actions),
        })
        .collect()
}

fn action_infos(config: &Config, names: &[String]) -> Vec<ActionInfo> {
    names
        .iter()
        .filter_map(|name| {
            let definition = config.actions.definitions.get(name)?;
            Some(ActionInfo {
                name: name.clone(),
                server: definition.server.clone(),
                tool: definition.tool.clone(),
                on: definition.on.clone(),
                approval: definition.approval,
            })
        })
        .collect()
}

/// A plan problem at a source location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDiagnostic {
    /// Bundle-relative path of the document.
    pub document: String,
    /// One-based line number.
    pub line: usize,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl From<Diagnostic> for PlanDiagnostic {
    fn from(diagnostic: Diagnostic) -> Self {
        Self {
            document: diagnostic.location.document,
            line: diagnostic.location.line,
            message: diagnostic.message,
        }
    }
}

/// The plan a run executes, as `GET /api/v1/runs/{run_id}/plan` serves it:
/// the documents submitted with the run and its units in execution order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPlan {
    /// The run.
    pub run_id: RunId,
    /// Plan title.
    pub title: String,
    /// Bundle-relative path of the entry document.
    pub entry: String,
    /// The submitted Markdown sources, by bundle-relative path.
    pub documents: BTreeMap<String, String>,
    /// Units in execution order.
    pub units: Vec<PlannedUnit>,
    /// Why the retained bundle no longer expands, when it does not (for
    /// example after a parser change); `units` is then empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PlanDiagnostic>,
}

/// One authored unit in execution order, with its run-unique key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedUnit {
    /// Run-unique key, as used in reports and events.
    pub key: UnitKey,
    /// Authored ID.
    pub id: String,
    /// Test or step.
    pub kind: UnitKind,
    /// Authored title.
    pub title: String,
    /// Title of the enclosing phase.
    pub phase: String,
    /// Location of the unit heading.
    pub location: SourceLocation,
    /// Whether the unit or its phase is cleanup work.
    pub cleanup: bool,
    /// The action as authored Markdown.
    pub action: String,
    /// Fenced code blocks of the action.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CodeBlock>,
    /// The authored expected results.
    pub criteria: Vec<Criterion>,
    /// The authored verification text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    /// Procedures this unit invokes, which run before it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedures: Vec<ProcedureRef>,
    /// Keys of the units whose invocations led here, outermost first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invoked_by: Vec<UnitKey>,
}

/// The plan view of a run's retained `bundle`, expanded with the same
/// parser that accepted it.
#[must_use]
pub fn run_plan(run_id: RunId, title: &str, bundle: &PlanBundle) -> RunPlan {
    let (units, diagnostics) = match expand(bundle) {
        Ok(expanded) => (
            expanded.units.into_iter().map(planned).collect(),
            Vec::new(),
        ),
        Err(error) => (
            Vec::new(),
            error.diagnostics.into_iter().map(Into::into).collect(),
        ),
    };
    RunPlan {
        run_id,
        title: title.to_owned(),
        entry: bundle.entry.clone(),
        documents: bundle.documents.clone(),
        units,
        diagnostics,
    }
}

fn planned(unit: crate::bundle::ExecutionUnit) -> PlannedUnit {
    let identity = unit.identity;
    let authored = unit.unit;
    PlannedUnit {
        key: identity.key,
        id: identity.id,
        kind: identity.kind,
        title: identity.title,
        phase: identity.phase,
        location: identity.location,
        cleanup: identity.cleanup,
        action: authored.action,
        commands: authored.commands,
        criteria: authored.expected,
        verification: authored.verification,
        procedures: authored.procedures,
        invoked_by: unit.invoked_by,
    }
}

#[cfg(test)]
mod tests;
