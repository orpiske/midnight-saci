#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Process execution: exit codes, deadlines, cancellation, output bounds and
//! process group cleanup.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use common::{script, scripts};
use saci_script_runner::executor::{self, Execution, KILL_GRACE, Status};

async fn run_with(
    executable: &Path,
    arguments: &[&str],
    timeout: Duration,
    cancelled: impl Future<Output = ()>,
) -> Execution {
    let arguments: Vec<String> = arguments.iter().map(|&a| a.to_owned()).collect();
    let environment = BTreeMap::from([
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("SACI_TEST".to_owned(), "1".to_owned()),
    ]);
    let request = executor::Request {
        executable,
        arguments: &arguments,
        working_directory: scripts(),
        environment: &environment,
        timeout,
        max_output_bytes: 1000,
    };
    executor::run(&request, cancelled).await
}

async fn run(name: &str, arguments: &[&str], timeout: Duration) -> Execution {
    run_with(&script(name), arguments, timeout, std::future::pending()).await
}

const LONG: Duration = Duration::from_secs(20);

#[tokio::test]
async fn a_successful_script_completes_with_its_output() {
    let result = run("args.sh", &["a b", "$(id)", "; rm -rf /"], LONG).await;
    assert_eq!(result.status, Status::Completed);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, "[a b]\n[$(id)]\n[; rm -rf /]\n");
    assert_eq!(result.stderr, "");
    assert!(!result.stdout_truncated);
}

#[tokio::test]
async fn a_nonzero_exit_fails_and_keeps_both_streams() {
    let result = run("exit3.sh", &[], LONG).await;
    assert_eq!(result.status, Status::Failed);
    assert_eq!(result.exit_code, Some(3));
    assert_eq!(result.stdout, "out\n");
    assert_eq!(result.stderr, "err\n");
}

#[tokio::test]
async fn a_missing_executable_is_a_spawn_failure() {
    let result = run("no-such-script.sh", &[], LONG).await;
    assert_eq!(result.status, Status::SpawnFailed);
    assert_eq!(result.exit_code, None);
    assert!(result.error.unwrap().contains("cannot start"));
}

#[tokio::test]
async fn the_environment_is_only_the_configured_one() {
    let result = run("env.sh", &[], LONG).await;
    assert!(result.stdout.contains("SACI_TEST=1"), "{}", result.stdout);
    assert!(!result.stdout.contains("CARGO"), "{}", result.stdout);
    let result = run("pwd.sh", &[], LONG).await;
    let expected = scripts().canonicalize().unwrap();
    assert_eq!(
        Path::new(result.stdout.trim()).canonicalize().unwrap(),
        expected
    );
}

#[tokio::test]
async fn a_script_past_its_deadline_times_out() {
    let started = std::time::Instant::now();
    let result = run("sleep.sh", &["30"], Duration::from_millis(300)).await;
    assert_eq!(result.status, Status::TimedOut);
    assert_eq!(result.exit_code, None);
    assert_eq!(result.signal, Some(15));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_script_that_ignores_sigterm_is_killed() {
    let started = std::time::Instant::now();
    let result = run("ignore-term.sh", &[], Duration::from_millis(200)).await;
    assert_eq!(result.status, Status::TimedOut);
    assert_eq!(result.signal, Some(9));
    assert!(started.elapsed() >= KILL_GRACE);
    assert!(started.elapsed() < KILL_GRACE + Duration::from_secs(3));
}

#[tokio::test]
async fn cancellation_stops_the_script() {
    let started = std::time::Instant::now();
    let cancelled = tokio::time::sleep(Duration::from_millis(200));
    let result = run_with(&script("sleep.sh"), &["30"], LONG, cancelled).await;
    assert_eq!(result.status, Status::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn large_output_is_bounded_on_both_streams() {
    let result = run("big.sh", &[], LONG).await;
    assert_eq!(result.status, Status::Completed);
    assert_eq!(result.stdout, "o".repeat(1000));
    assert_eq!(result.stderr, "e".repeat(1000));
    assert!(result.stdout_truncated);
    assert!(result.stderr_truncated);
}

#[tokio::test]
async fn non_utf8_output_is_lossy_utf8() {
    let result = run("binary.sh", &[], LONG).await;
    assert_eq!(result.stdout, "ok\u{fffd}\u{fffd}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_timeout_kills_the_whole_process_group() {
    let pid_file = tempfile::NamedTempFile::new().unwrap();
    let path = pid_file.path().to_str().unwrap();
    let result = run("orphan-wait.sh", &[path], Duration::from_millis(500)).await;
    assert_eq!(result.status, Status::TimedOut);
    let pid = common::read_pid(pid_file.path()).await;
    assert!(common::gone(pid).await, "background process {pid} survived");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn processes_left_after_exit_are_killed() {
    let pid_file = tempfile::NamedTempFile::new().unwrap();
    let path = pid_file.path().to_str().unwrap();
    let started = std::time::Instant::now();
    let result = run("orphan-exit.sh", &[path], LONG).await;
    assert_eq!(result.status, Status::Completed);
    // The leftover process holds stdout open; it must not delay the result.
    assert!(started.elapsed() < Duration::from_secs(5));
    let pid = common::read_pid(pid_file.path()).await;
    assert!(common::gone(pid).await, "background process {pid} survived");
}

#[test]
fn status_names_match_the_serialized_form() {
    for status in [
        Status::Completed,
        Status::Failed,
        Status::TimedOut,
        Status::Cancelled,
        Status::SpawnFailed,
    ] {
        assert_eq!(serde_json::to_value(status).unwrap(), status.as_str());
    }
}
