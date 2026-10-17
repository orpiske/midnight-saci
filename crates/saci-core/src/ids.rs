//! Stable identities for runs, execution units, attempts and evidence.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies one run. Serialized as `run-<uuid>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Creates a new random run identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("run-{}", Uuid::new_v4().simple()))
    }

    /// Accepts an identifier received from a client or a stored record.
    ///
    /// # Errors
    ///
    /// Returns [`IdError`] when the value is not a `run-` identifier made of
    /// lowercase ASCII letters and digits.
    pub fn parse(value: &str) -> Result<Self, IdError> {
        let valid = value
            .strip_prefix("run-")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(is_id_char));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(IdError::new("run", value))
        }
    }

    /// Returns the identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

const fn is_id_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

/// Identifies one execution unit within a run.
///
/// Units authored in the entry document use their heading ID (`1.1`). Units
/// reached through a procedure invocation are prefixed by the invoking
/// unit's key and the procedure path, for example
/// `1.1>common/setup.md#2.1`, so repeated invocations stay distinct.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitKey(String);

impl UnitKey {
    /// Creates a key from its textual form.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Creates the key of a unit reached through a procedure invocation.
    #[must_use]
    pub fn nested(&self, procedure: &str, unit_id: &str) -> Self {
        Self(format!("{}>{procedure}#{unit_id}", self.0))
    }

    /// Returns the key as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnitKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifies one recorded observation within a run. Serialized as `ev-<n>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvidenceId(String);

impl EvidenceId {
    /// Creates the identifier for the `n`th observation of a run.
    #[must_use]
    pub fn from_index(index: u64) -> Self {
        Self(format!("ev-{index}"))
    }

    /// Accepts an identifier referenced by an assessment or a client.
    ///
    /// # Errors
    ///
    /// Returns [`IdError`] unless the value is `ev-` followed by digits.
    pub fn parse(value: &str) -> Result<Self, IdError> {
        let valid = value
            .strip_prefix("ev-")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(IdError::new("evidence", value))
        }
    }

    /// Returns the identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EvidenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An identifier did not have the expected form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} identifier `{value}`")]
pub struct IdError {
    kind: &'static str,
    value: String,
}

impl IdError {
    fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_run_ids_parse_back() {
        let id = RunId::generate();
        assert_eq!(RunId::parse(id.as_str()).unwrap(), id);
    }

    #[test]
    fn malformed_ids_are_rejected() {
        assert!(RunId::parse("run-").is_err());
        assert!(RunId::parse("../etc").is_err());
        assert!(EvidenceId::parse("ev-x").is_err());
        assert_eq!(
            EvidenceId::parse("ev-7").unwrap(),
            EvidenceId::from_index(7)
        );
    }

    #[test]
    fn repeated_procedure_invocations_get_distinct_keys() {
        let first = UnitKey::new("1.1").nested("common/setup.md", "1");
        let second = UnitKey::new("2.1").nested("common/setup.md", "1");
        assert_ne!(first, second);
        assert_eq!(first.as_str(), "1.1>common/setup.md#1");
    }
}
