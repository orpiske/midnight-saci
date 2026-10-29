//! The `git` program: branch heads and the watcher's clones.
//!
//! Every command is an argument list, never a shell line, so values from a
//! repository cannot change a command. `git` never prompts for credentials.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, bail};
use tokio::process::Command;

/// Upper bound for one `git` command.
const TIMEOUT: Duration = Duration::from_secs(600);

/// The commit at the head of `branch` in `url`.
///
/// # Errors
///
/// Fails when `git` fails or the branch does not exist.
pub async fn head(url: &str, branch: &str) -> anyhow::Result<String> {
    let reference = format!("refs/heads/{branch}");
    let output = Box::pin(git(None, &["ls-remote", "--", url, &reference])).await?;
    output
        .lines()
        .find_map(|line| {
            let (commit, name) = line.split_once('\t')?;
            (name == reference).then(|| commit.to_owned())
        })
        .with_context(|| format!("branch `{branch}` does not exist in {url}"))
}

/// Puts the clone in `dir` at `commit` of `branch`: clones it when it is
/// missing, fetches, checks out the commit and removes every other file.
///
/// # Errors
///
/// Fails when a `git` command fails, for example when the commit is no
/// longer on the branch.
pub async fn checkout(dir: &Path, url: &str, branch: &str, commit: &str) -> anyhow::Result<()> {
    if !dir.join(".git").exists() {
        if dir.exists() {
            bail!("{} exists and is not a clone; remove it", dir.display());
        }
        let target = dir.to_string_lossy();
        Box::pin(git(None, &["clone", "--no-checkout", "--", url, &target])).await?;
    }
    Box::pin(git(Some(dir), &["remote", "set-url", "origin", url])).await?;
    let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    Box::pin(git(Some(dir), &["fetch", "--no-tags", "origin", &refspec])).await?;
    Box::pin(git(Some(dir), &["checkout", "--force", "--detach", commit])).await?;
    Box::pin(git(Some(dir), &["clean", "-ffdx"])).await?;
    Ok(())
}

async fn git(dir: Option<&Path>, args: &[&str]) -> anyhow::Result<String> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        // Its own process group: Ctrl-C in a terminal stops the watcher
        // after this command instead of breaking a fetch.
        .process_group(0);
    let output = tokio::time::timeout(TIMEOUT, command.output())
        .await
        .with_context(|| {
            format!(
                "`git {}` did not end in {} s",
                args.join(" "),
                TIMEOUT.as_secs()
            )
        })?
        .context("cannot start `git`; install Git or put it on the PATH")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`git {}` failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
