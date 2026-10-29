//! The `saci watch` configuration file and its checks.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, bail};
use serde::Deserialize;

/// The whole configuration file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchConfig {
    /// The backend to submit runs to; the CLI options apply when omitted.
    #[serde(default)]
    pub backend: BackendSettings,
    /// Directory for the clones and the state file.
    pub workdir: PathBuf,
    /// Seconds between two polls.
    #[serde(default = "default_poll")]
    pub poll_interval_secs: u64,
    /// Seconds without a change before a cycle starts.
    #[serde(default = "default_quiet")]
    pub quiet_period_secs: u64,
    /// Every repository of the snapshot.
    pub repositories: Vec<Repository>,
    /// What to run when repositories change, in the order they run.
    pub triggers: Vec<Trigger>,
}

/// The `backend` section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendSettings {
    /// Backend base URL.
    pub url: Option<String>,
    /// The client token.
    pub token: Option<SecretRef>,
    /// Backend configuration file. When set and no backend answers at the
    /// URL, the watcher starts `saci backend start --config <file>`. A
    /// relative path is relative to the watch configuration file.
    pub config: Option<PathBuf>,
}

/// A credential, in the format of the Saci backend configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
    /// Name of an environment variable of the watcher process.
    pub env: String,
}

/// A watched repository.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    /// Name of the clone directory and of the repository in plan paths.
    pub name: String,
    /// Anything `git clone` accepts.
    pub url: String,
    /// The watched branch.
    pub branch: String,
}

/// Plans to run when some repositories change.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    /// Trigger name, part of the idempotency keys.
    pub name: String,
    /// Repositories whose changes start the trigger.
    pub on: Vec<String>,
    /// Plans, submitted one after the other.
    pub plans: Vec<PlanEntry>,
}

/// One plan of a trigger.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEntry {
    /// `<repository>/<path in the repository>` of the entry document.
    pub plan: String,
    /// Configured environment of the backend.
    pub environment: String,
    /// Configured run policy; the backend uses `default` when omitted.
    pub policy: Option<String>,
}

const fn default_poll() -> u64 {
    60
}

const fn default_quiet() -> u64 {
    180
}

impl WatchConfig {
    /// Reads and checks the file. A leading `~/` in `workdir` is the home
    /// directory.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or parsed, or a check fails.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let mut config: Self = serde_yaml::from_str(&text)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        config.workdir = expand_home(&config.workdir, home().as_deref())?;
        if let Some(backend) = &config.backend.config {
            let backend = expand_home(backend, home().as_deref())?;
            let base = path.parent().unwrap_or_else(|| Path::new(""));
            config.backend.config = Some(base.join(backend));
        }
        config
            .validate(home().as_deref())
            .with_context(|| format!("invalid configuration {}", path.display()))?;
        Ok(config)
    }

    /// The configuration checks; `home` is the user's home directory.
    ///
    /// # Errors
    ///
    /// Names the first value that is not permitted.
    pub fn validate(&self, home: Option<&Path>) -> anyhow::Result<()> {
        if self.poll_interval_secs == 0 {
            bail!("poll_interval_secs must be at least 1");
        }
        if self.repositories.is_empty() || self.triggers.is_empty() {
            bail!("the configuration needs at least one repository and one trigger");
        }
        check_workdir(&self.workdir, home)?;
        let mut names = HashSet::new();
        for repository in &self.repositories {
            check_name("repository", &repository.name)?;
            if !names.insert(repository.name.as_str()) {
                bail!("repository `{}` is listed twice", repository.name);
            }
            if repository.url.is_empty() || repository.branch.is_empty() {
                bail!("repository `{}` needs a url and a branch", repository.name);
            }
        }
        let mut triggers = HashSet::new();
        for trigger in &self.triggers {
            if !triggers.insert(trigger.name.as_str()) {
                bail!("trigger `{}` is listed twice", trigger.name);
            }
            trigger.validate(&names)?;
        }
        Ok(())
    }

    /// The directory of the clones.
    #[must_use]
    pub fn repos_dir(&self) -> PathBuf {
        self.workdir.join("repos")
    }

    /// The state file.
    #[must_use]
    pub fn state_file(&self) -> PathBuf {
        self.workdir.join("state.json")
    }

    /// The configured trigger called `name`.
    #[must_use]
    pub fn trigger(&self, name: &str) -> Option<&Trigger> {
        self.triggers.iter().find(|trigger| trigger.name == name)
    }
}

impl Trigger {
    fn validate(&self, repositories: &HashSet<&str>) -> anyhow::Result<()> {
        check_name("trigger", &self.name)?;
        if self.on.is_empty() || self.plans.is_empty() {
            bail!("trigger `{}` needs `on` and `plans`", self.name);
        }
        for name in &self.on {
            if !repositories.contains(name.as_str()) {
                bail!(
                    "trigger `{}`: `on` names unknown repository `{name}`",
                    self.name
                );
            }
        }
        for entry in &self.plans {
            let repository = plan_repository(&entry.plan)
                .with_context(|| format!("trigger `{}`, plan `{}`", self.name, entry.plan))?;
            if !repositories.contains(repository) {
                bail!(
                    "trigger `{}`, plan `{}`: `{repository}` is not a configured repository",
                    self.name,
                    entry.plan
                );
            }
        }
        Ok(())
    }
}

/// The repository part of a plan path, after checking that the path stays
/// in the clones.
fn plan_repository(plan: &str) -> anyhow::Result<&str> {
    let path = Path::new(plan);
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("the path must be relative, without `.` or `..`");
    }
    match plan.split_once('/') {
        Some((repository, rest)) if !rest.is_empty() => Ok(repository),
        _ => bail!("write the path as <repository>/<path in the repository>"),
    }
}

fn check_name(kind: &str, name: &str) -> anyhow::Result<()> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if name.is_empty() || name.starts_with('.') || !name.chars().all(allowed) {
        bail!(
            "{kind} name `{name}` must use letters, digits, `-`, `_` or `.`, and not start with `.`"
        );
    }
    Ok(())
}

/// The watcher deletes files in its clones, so the work directory must not
/// be a person's directory or a Git working tree.
fn check_workdir(workdir: &Path, home: Option<&Path>) -> anyhow::Result<()> {
    if !workdir.is_absolute() {
        bail!("workdir must be an absolute path or start with `~/`");
    }
    if home.is_some_and(|home| home == workdir) || workdir.parent().is_none() {
        bail!(
            "workdir {} must not be the home or root directory",
            workdir.display()
        );
    }
    if let Some(tree) = workdir.ancestors().find(|dir| dir.join(".git").exists()) {
        bail!(
            "workdir {} is in the Git working tree {}; use a directory outside it",
            workdir.display(),
            tree.display()
        );
    }
    Ok(())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn expand_home(path: &Path, home: Option<&Path>) -> anyhow::Result<PathBuf> {
    let Ok(rest) = path.strip_prefix("~") else {
        return Ok(path.to_path_buf());
    };
    let Some(home) = home else {
        bail!("workdir starts with `~`, but HOME is not set");
    };
    Ok(home.join(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = "workdir: /var/lib/saci-watch
repositories:
  - { name: server, url: https://example.com/server.git, branch: main }
  - { name: tests, url: https://example.com/tests.git, branch: main }
triggers:
  - name: smoke
    on: [server]
    plans:
      - { plan: tests/plans/smoke.md, environment: local }
";

    fn parse(yaml: &str) -> WatchConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn error(yaml: &str) -> String {
        format!("{:#}", parse(yaml).validate(None).unwrap_err())
    }

    #[test]
    fn a_valid_file_gets_the_default_intervals() {
        let config = parse(YAML);
        config.validate(Some(Path::new("/home/me"))).unwrap();
        assert_eq!(config.poll_interval_secs, 60);
        assert_eq!(config.quiet_period_secs, 180);
        assert_eq!(config.trigger("smoke").unwrap().plans.len(), 1);
    }

    #[test]
    fn triggers_must_name_configured_repositories() {
        let unknown = YAML.replace("on: [server]", "on: [operator]");
        assert!(error(&unknown).contains("unknown repository `operator`"));
        let plan = YAML.replace("tests/plans", "docs/plans");
        assert!(error(&plan).contains("`docs` is not a configured repository"));
    }

    #[test]
    fn plan_paths_cannot_leave_the_clones() {
        for path in [
            "../x/plan.md",
            "/etc/plan.md",
            "tests/../../plan.md",
            "tests",
        ] {
            let yaml = YAML.replace("tests/plans/smoke.md", path);
            assert!(!error(&yaml).is_empty(), "{path} was accepted");
        }
    }

    #[test]
    fn names_must_be_safe_directory_names_and_unique() {
        let bad = YAML.replace("name: server", "name: ../server");
        assert!(error(&bad).contains("must use letters"));
        let twice = YAML.replace("name: tests", "name: server");
        assert!(error(&twice).contains("listed twice"));
    }

    #[test]
    fn the_workdir_is_not_home_or_a_working_tree() {
        let home = YAML.replace("/var/lib/saci-watch", "/home/me");
        let config = parse(&home);
        let refused = config.validate(Some(Path::new("/home/me"))).unwrap_err();
        assert!(refused.to_string().contains("home"));
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir(tree.path().join(".git")).unwrap();
        let inside = tree.path().join("watch");
        let yaml = YAML.replace("/var/lib/saci-watch", &inside.display().to_string());
        assert!(error(&yaml).contains("Git working tree"));
    }

    #[test]
    fn a_leading_tilde_is_the_home_directory() {
        let home = Path::new("/home/me");
        let path = expand_home(Path::new("~/.local/state/w"), Some(home)).unwrap();
        assert_eq!(path, Path::new("/home/me/.local/state/w"));
        assert!(expand_home(Path::new("~/w"), None).is_err());
        assert_eq!(expand_home(Path::new("/w"), None).unwrap(), Path::new("/w"));
    }
}
