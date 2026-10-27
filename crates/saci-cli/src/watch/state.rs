//! The state file: what the watcher saw, tested and waits for.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context as _;
use saci_core::ids::RunId;
use saci_core::status::Outcome;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;

/// A commit for each repository name.
pub type Snapshot = BTreeMap<String, String>;

/// Everything the watcher must keep across restarts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// The last head seen of each repository.
    pub seen: Snapshot,
    /// The last cycle of each trigger.
    #[serde(default)]
    pub triggers: BTreeMap<String, LastCycle>,
    /// The cycle in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle: Option<Cycle>,
}

/// The last cycle of a trigger, finished or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastCycle {
    /// The commits the cycle tests.
    pub snapshot: Snapshot,
    /// When the cycle started; absent for the baseline of the first start.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    /// The plans submitted so far.
    #[serde(default)]
    pub runs: Vec<PlanRun>,
}

/// What became of one plan of a cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRun {
    /// The plan path from the configuration.
    pub plan: String,
    /// The accepted run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunId>,
    /// The run's outcome once it finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Why the plan was not submitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// The position in the cycle in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cycle {
    /// The trigger whose plans run.
    pub trigger: String,
    /// Index of the plan to submit or wait for.
    pub next: usize,
    /// The run of plan `next` once accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunId>,
}

impl State {
    /// Reads the state file; `None` when it does not exist yet.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        let state = serde_json::from_str(&text)
            .with_context(|| format!("cannot parse {}; fix or remove it", path.display()))?;
        Ok(Some(state))
    }

    /// Writes the state file through a temporary file and a rename, so a
    /// crash leaves the old file or the new one, never a part.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be written.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .with_context(|| format!("cannot replace {}", path.display()))
    }
}

/// A short, stable name for a snapshot, for idempotency keys.
#[must_use]
pub fn snapshot_hash(snapshot: &Snapshot) -> String {
    let mut digest = Sha256::new();
    for (name, commit) in snapshot {
        digest.update(format!("{name}={commit}\n"));
    }
    digest.finalize()[..8]
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(pairs: &[(&str, &str)]) -> Snapshot {
        pairs
            .iter()
            .map(|(name, commit)| ((*name).to_owned(), (*commit).to_owned()))
            .collect()
    }

    #[test]
    fn the_hash_changes_with_any_commit_and_not_with_order() {
        let first = snapshot(&[("a", "1"), ("b", "2")]);
        let reordered = snapshot(&[("b", "2"), ("a", "1")]);
        let changed = snapshot(&[("a", "1"), ("b", "3")]);
        assert_eq!(snapshot_hash(&first), snapshot_hash(&reordered));
        assert_ne!(snapshot_hash(&first), snapshot_hash(&changed));
        assert_eq!(snapshot_hash(&first).len(), 16);
    }

    #[test]
    fn a_saved_state_reads_back_and_a_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        assert_eq!(State::load(&path).unwrap(), None);
        let mut state = State {
            seen: snapshot(&[("a", "1")]),
            ..State::default()
        };
        state.cycle = Some(Cycle {
            trigger: "smoke".into(),
            next: 1,
            run: None,
        });
        state.save(&path).unwrap();
        assert_eq!(State::load(&path).unwrap(), Some(state));
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::write(&path, "{").unwrap();
        assert!(format!("{:#}", State::load(&path).unwrap_err()).contains("fix or remove"));
    }
}
