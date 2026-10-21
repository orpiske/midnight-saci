//! The version and commit of a Saci build (docs/development.md#build-commit).

use serde::{Deserialize, Serialize};

/// Which Saci build a process runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfo {
    /// Package version.
    pub version: String,
    /// Commit the binary was built from, when the build knew it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

impl BuildInfo {
    /// The build of the running binary.
    #[must_use]
    pub fn current() -> Self {
        Self {
            version: crate::VERSION.to_owned(),
            commit: COMMIT.map(str::to_owned),
        }
    }
}

/// The commit `build.rs` recorded, if any.
pub const COMMIT: Option<&str> = option_env!("SACI_BUILD_COMMIT");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_build_records_the_commit_of_the_checkout() {
        let build = BuildInfo::current();
        assert_eq!(build.version, env!("CARGO_PKG_VERSION"));
        let expected = std::env::var("SACI_BUILD_COMMIT").ok().or_else(|| {
            let output = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        });
        assert_eq!(build.commit, expected);
    }

    #[test]
    fn an_unknown_commit_is_left_out_of_the_json() {
        let build = BuildInfo {
            version: "1.2.3".into(),
            commit: None,
        };
        assert_eq!(
            serde_json::to_value(&build).unwrap(),
            serde_json::json!({"version": "1.2.3"})
        );
    }
}
