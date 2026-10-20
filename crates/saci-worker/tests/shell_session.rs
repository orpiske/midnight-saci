//! Behavior of the persistent shell session.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::time::Duration;

use saci_core::evidence::{ShellEvidence, Termination};
use saci_core::plan::is_shell_language;
use saci_core::redaction::Redactor;
use saci_worker::shell::{ShellConfig, ShellSession};
use tokio_util::sync::CancellationToken;

const LONG: Duration = Duration::from_secs(20);

struct Fixture {
    session: ShellSession,
    workspace: tempfile::TempDir,
    cancel: CancellationToken,
}

impl Fixture {
    fn new() -> Self {
        Self::with(64 * 1024, Redactor::new())
    }

    fn with(output_limit: u64, redactor: Redactor) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let environment = BTreeMap::from([
            ("PATH".to_owned(), std::env::var("PATH").unwrap()),
            ("TARGET_URL".to_owned(), "http://target".to_owned()),
        ]);
        let config = ShellConfig {
            workspace: workspace.path().to_path_buf(),
            environment,
            output_limit,
            redactor,
        };
        Self {
            session: ShellSession::new(config).unwrap(),
            workspace,
            cancel: CancellationToken::new(),
        }
    }

    async fn run_for(&mut self, command: &str, deadline: Duration) -> ShellEvidence {
        let run = self.session.run(command, deadline, &self.cancel).await;
        run.unwrap().evidence
    }

    async fn run(&mut self, command: &str) -> ShellEvidence {
        self.run_for(command, LONG).await
    }
}

#[tokio::test]
async fn context_persists_between_calls() {
    let mut shell = Fixture::new();
    shell
        .run("export GREETING=hello; greet() { echo \"$GREETING $1\"; }")
        .await;
    shell
        .run("mkdir -p sub && cd sub && echo data > file.txt")
        .await;
    let result = shell.run("greet world; pwd; cat file.txt").await;
    let sub = shell.workspace.path().join("sub").canonicalize().unwrap();
    assert_eq!(
        result.stdout.text,
        format!("hello world\n{}\ndata\n", sub.display())
    );
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.termination, Termination::Exited);
    assert!(!result.restarted && !result.session_lost);
}

#[tokio::test]
async fn exit_status_and_streams_are_captured_separately() {
    let mut shell = Fixture::new();
    let failed = shell.run("echo out; echo err >&2; (exit 7)").await;
    assert_eq!(failed.exit_code, Some(7));
    assert_eq!(failed.stdout.text, "out\n");
    assert_eq!(failed.stderr.text, "err\n");
    let no_newline = shell.run("printf partial; false").await;
    assert_eq!(no_newline.stdout.text, "partial");
    assert_eq!(no_newline.exit_code, Some(1));
    let stdin = shell.run("read line; echo \"got:$line\"").await;
    assert_eq!(
        stdin.stdout.text, "got:\n",
        "stdin is empty, not the protocol"
    );
}

#[tokio::test]
async fn the_environment_is_exactly_the_configured_one() {
    let mut shell = Fixture::new();
    let result = shell.run("env | cut -d= -f1 | sort | tr '\\n' ' '").await;
    for name in ["HOME", "PATH", "TARGET_URL"] {
        assert!(result.stdout.text.contains(name), "{}", result.stdout.text);
    }
    assert!(
        !result.stdout.text.contains("CARGO"),
        "{}",
        result.stdout.text
    );
}

#[tokio::test]
async fn output_is_bounded_with_visible_truncation_and_redacted() {
    let mut redactor = Redactor::new();
    redactor.add("TOKEN", "super-secret");
    let mut shell = Fixture::with(16, redactor);
    let result = shell.run("echo super-secret; seq 1 1000").await;
    assert!(result.stdout.truncated);
    assert!(result.stdout.total_bytes > 1000);
    assert!(result.stdout.text.starts_with("[REDACTED:TOKEN]"));
    let command = shell.run("echo 'super-secret' >/dev/null").await.command;
    assert_eq!(command, "echo '[REDACTED:TOKEN]' >/dev/null");
}

#[tokio::test]
async fn deadlines_kill_descendants_and_keep_the_session() {
    let mut shell = Fixture::new();
    shell.run("KEEP=yes").await;
    let short = Duration::from_secs(1);
    let result = shell
        .run_for("sleep 60 & echo $! > bg.pid; sleep 60", short)
        .await;
    assert_eq!(result.termination, Termination::TimedOut);
    assert!(!result.session_lost);
    let after = shell
        .run("echo $KEEP; kill -0 $(cat bg.pid) 2>/dev/null || echo gone")
        .await;
    assert_eq!(after.stdout.text, "yes\ngone\n");
    assert!(!after.restarted);
}

#[tokio::test]
async fn a_shell_that_cannot_be_interrupted_is_replaced_explicitly() {
    let mut shell = Fixture::new();
    shell.run("KEEP=yes; touch kept.txt").await;
    let short = Duration::from_secs(1);
    let stuck = shell.run_for("while :; do :; done", short).await;
    assert_eq!(stuck.termination, Termination::TimedOut);
    assert!(stuck.session_lost);
    let next = shell.run("echo \"[${KEEP:-unset}]\"; ls").await;
    assert!(next.restarted);
    assert_eq!(next.stdout.text, "[unset]\nkept.txt\n");
}

#[tokio::test]
async fn exit_ends_the_session_visibly() {
    let mut shell = Fixture::new();
    let ended = shell.run("echo bye; exit 3").await;
    assert_eq!(ended.termination, Termination::SessionEnded);
    assert_eq!(ended.exit_code, Some(3));
    assert_eq!(ended.stdout.text, "bye\n");
    assert!(ended.session_lost);
    let next = shell.run("echo again").await;
    assert!(next.restarted);
    assert_eq!(next.stdout.text, "again\n");
}

#[tokio::test]
async fn cancellation_stops_the_command() {
    let mut shell = Fixture::new();
    let cancel = shell.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let result = shell.run("sleep 60").await;
    assert_eq!(result.termination, Termination::Cancelled);
    shell.session.close().await;
}

#[test]
fn supported_languages_are_bash_compatible() {
    assert!(is_shell_language("bash"));
    assert!(is_shell_language("Shell"));
    assert!(is_shell_language("sh"));
    assert!(!is_shell_language("python"));
    assert!(!is_shell_language("console"));
}
