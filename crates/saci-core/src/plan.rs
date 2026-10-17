//! The parsed shape of a Markdown test plan.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A position in a submitted document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceLocation {
    /// Bundle-relative path of the document, using `/` separators.
    pub document: String,
    /// One-based line number.
    pub line: usize,
}

impl SourceLocation {
    /// Creates a location.
    #[must_use]
    pub fn new(document: impl Into<String>, line: usize) -> Self {
        Self {
            document: document.into(),
            line,
        }
    }
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.document, self.line)
    }
}

/// A parsed test plan document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Text after `# Test Plan:`.
    pub title: String,
    /// Bundle-relative path of this document.
    pub document: String,
    /// Body of `## Overview`, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    /// Body of `## Prerequisites`, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<String>,
    /// Phases in document order.
    pub phases: Vec<Phase>,
}

impl Plan {
    /// Iterates over every unit in document order.
    pub fn units(&self) -> impl Iterator<Item = &Unit> {
        self.phases.iter().flat_map(|phase| phase.units.iter())
    }
}

/// A `## Phase <id>: <title>` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    /// Authored phase label.
    pub id: String,
    /// Authored phase title.
    pub title: String,
    /// Location of the heading.
    pub location: SourceLocation,
    /// Tests and steps in document order.
    pub units: Vec<Unit>,
}

/// Whether a unit was authored as a test or a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    /// `### Test <id>: <title>`.
    Test,
    /// `### Step <id>: <title>`.
    Step,
}

/// A `### Test` or `### Step` section: one action with expected results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    /// Test or step.
    pub kind: UnitKind,
    /// Authored label, unique within its document.
    pub id: String,
    /// Authored title.
    pub title: String,
    /// Location of the heading.
    pub location: SourceLocation,
    /// The action as authored Markdown, including any fenced commands.
    pub action: String,
    /// Fenced code blocks found in the action, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CodeBlock>,
    /// Criteria listed under `**Expected result:**`.
    pub expected: Vec<Criterion>,
    /// Text under `**Verification:**`, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    /// Procedures this unit explicitly invokes, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedures: Vec<ProcedureRef>,
}

/// A fenced code block inside an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeBlock {
    /// Info-string language, such as `bash`; empty when unspecified.
    pub language: String,
    /// Block contents without the fences.
    pub code: String,
    /// Location of the opening fence.
    pub location: SourceLocation,
}

/// One expected-result criterion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Criterion {
    /// Zero-based position within the unit's expected results.
    pub index: usize,
    /// The criterion text.
    pub text: String,
    /// Location of the criterion.
    pub location: SourceLocation,
}

/// An explicit `Follow [path](path).` procedure invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureRef {
    /// Link target as written, relative to the invoking document.
    pub target: String,
    /// Location of the invocation.
    pub location: SourceLocation,
}

/// The submitted Markdown documents of one plan: the entry document plus
/// every procedure it invokes, keyed by bundle-relative path.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlanBundle {
    /// Bundle-relative path of the entry document.
    pub entry: String,
    /// Document sources keyed by bundle-relative path.
    pub documents: BTreeMap<String, String>,
}
