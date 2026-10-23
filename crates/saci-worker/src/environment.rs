//! Describes where the worker executes (`docs/worker.md#environment-details`):
//! the Saci build, the operating system and the versions of the tools on
//! `PATH`. The worker reports it in `run_started`.

use std::process::Stdio;
use std::time::Duration;

use saci_core::build_info::BuildInfo;
use saci_core::run::{ToolVersion, WorkerEnvironment};
use tokio::process::Command;

/// The tools whose versions the worker reports, with the arguments that
/// print the version. Tools that are not installed are left out.
pub const TOOLS: &[(&str, &[&str])] = &[
    ("bash", &["--version"]),
    ("curl", &["--version"]),
    ("git", &["--version"]),
    ("jq", &["--version"]),
    ("ripwire", &["--version"]),
    ("ssh", &["-V"]),
    ("nc", &["--version"]),
    ("dig", &["-v"]),
    ("kubectl", &["version", "--client"]),
    ("oc", &["version", "--client"]),
    ("helm", &["version", "--short"]),
    ("podman", &["--version"]),
    ("docker", &["--version"]),
    ("java", &["-version"]),
    ("mvn", &["--version"]),
    ("jbang", &["--version"]),
    ("node", &["--version"]),
    ("python3", &["--version"]),
];

/// How long one command may take to print its version.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Longest reported version line, in characters.
const MAX_LINE: usize = 200;

/// Collects the environment of this worker. Every command runs at the same
/// time, so the slowest tool sets the duration, [`TIMEOUT`] at most.
pub async fn collect() -> WorkerEnvironment {
    let (os, kernel, tools) = tokio::join!(os(), first_line("uname", &["-sr"]), tools(TOOLS));
    WorkerEnvironment {
        saci: BuildInfo::current(),
        os,
        kernel,
        arch: std::env::consts::ARCH.to_owned(),
        tools,
    }
}

/// The versions of the installed `tools`, in the order given.
pub async fn tools(tools: &[(&str, &[&str])]) -> Vec<ToolVersion> {
    let mut lookups = tokio::task::JoinSet::new();
    for (index, (name, args)) in tools.iter().enumerate() {
        let name = (*name).to_owned();
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        lookups.spawn(async move {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let version = first_line(&name, &args).await;
            (index, version.map(|version| ToolVersion { name, version }))
        });
    }
    let mut found: Vec<(usize, ToolVersion)> = lookups
        .join_all()
        .await
        .into_iter()
        .filter_map(|(index, tool)| Some((index, tool?)))
        .collect();
    found.sort_by_key(|(index, _)| *index);
    found.into_iter().map(|(_, tool)| tool).collect()
}

/// The operating system's name and version: `PRETTY_NAME` from
/// `/etc/os-release` on Linux, `sw_vers` on macOS.
async fn os() -> Option<String> {
    if let Ok(text) = tokio::fs::read_to_string("/etc/os-release").await {
        return pretty_name(&text);
    }
    let name = first_line("sw_vers", &["-productName"]).await?;
    let version = first_line("sw_vers", &["-productVersion"]).await?;
    Some(format!("{name} {version}"))
}

/// The `PRETTY_NAME` of an `os-release` file.
fn pretty_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|value| value.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|value| !value.is_empty())
}

/// The first non-empty line `program args` prints, on standard output or
/// else on standard error (`ssh -V` and `java -version` use it). `None`
/// when the program is missing, fails or does not finish in time.
async fn first_line(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME"))
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    [&output.stdout, &output.stderr]
        .into_iter()
        .find_map(|bytes| version_line(&String::from_utf8_lossy(bytes)))
}

/// The first non-empty line of `text`, trimmed and cut to [`MAX_LINE`].
fn version_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.chars().take(MAX_LINE).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pretty_name_comes_from_os_release() {
        let text = "NAME=\"Fedora Linux\"\nVERSION_ID=44\nPRETTY_NAME=\"Fedora Linux 44 (Container Image)\"\n";
        assert_eq!(
            pretty_name(text).as_deref(),
            Some("Fedora Linux 44 (Container Image)")
        );
        assert_eq!(pretty_name("NAME=Alpine\n"), None);
    }

    #[test]
    fn the_version_is_the_first_non_empty_line_cut_to_length() {
        assert_eq!(
            version_line("\n  GNU bash, version 5.2\nCopyright\n").as_deref(),
            Some("GNU bash, version 5.2")
        );
        assert_eq!(version_line(" \n"), None);
        let long = "x".repeat(MAX_LINE + 10);
        assert_eq!(version_line(&long).map(|line| line.len()), Some(MAX_LINE));
    }

    #[tokio::test]
    async fn missing_and_failing_tools_are_left_out() {
        let found = tools(&[
            ("sh", &["-c", "echo 'tool 1.0'"]),
            ("saci-no-such-tool", &["--version"]),
            ("sh", &["-c", "exit 3"]),
            ("sh", &["-c", "echo 'only on stderr 2.0' >&2"]),
        ])
        .await;
        let lines: Vec<_> = found.iter().map(|tool| tool.version.as_str()).collect();
        assert_eq!(lines, ["tool 1.0", "only on stderr 2.0"]);
    }

    #[tokio::test]
    async fn the_environment_names_this_build_and_platform() {
        let environment = collect().await;
        assert_eq!(environment.saci, BuildInfo::current());
        assert_eq!(environment.arch, std::env::consts::ARCH);
        assert!(environment.kernel.is_some());
        assert!(environment.tools.iter().any(|tool| tool.name == "bash"));
    }
}
