//! The `saci-start-backend` skill's `ensure-backend.sh` starts a backend only
//! when none answers, and reports why it could not start one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SACI: &str = env!("CARGO_BIN_EXE_saci");

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/saci-start-backend/ensure-backend.sh")
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn ensure(dir: &Path, args: &[&str]) -> Output {
    Command::new("bash")
        .arg(script())
        .args(args)
        .current_dir(dir)
        .env("SACI", SACI)
        .env_remove("SACI_BACKEND_URL")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Kills the backend named in the pid file on drop.
struct Started(PathBuf);

impl Drop for Started {
    fn drop(&mut self) {
        if let Ok(pid) = std::fs::read_to_string(&self.0) {
            let _ = Command::new("kill").arg(pid.trim()).status();
        }
    }
}

/// Writes a valid `saci.yaml` for a free local port and returns its URL.
fn configure(dir: &Path) -> String {
    let port = free_port();
    std::fs::write(
        dir.join("saci.yaml"),
        format!(
            "backend:\n  listen: 127.0.0.1:{port}\n  data_dir: data\nllm:\n  \
             base_url: http://model:11434/v1\n  model: test-model\n"
        ),
    )
    .unwrap();
    format!("http://127.0.0.1:{port}")
}

#[test]
fn starts_a_backend_only_when_none_answers() {
    let dir = tempfile::tempdir().unwrap();
    let url = configure(dir.path());
    let pidfile = dir.path().join("saci-backend.pid");
    let _started = Started(pidfile.clone());

    let first = ensure(dir.path(), &["--backend", &url]);
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    assert!(
        text(&first.stdout).contains("Started a Saci backend"),
        "{first:?}"
    );
    let pid = std::fs::read_to_string(&pidfile).unwrap();
    assert!(
        dir.path().join("data").is_dir(),
        "relative paths start from the current directory"
    );

    let second = ensure(dir.path(), &["--backend", &url]);
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    assert!(
        text(&second.stdout).contains("already answers"),
        "{second:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&pidfile).unwrap(),
        pid,
        "no second backend"
    );
}

#[test]
fn a_backend_that_stops_shows_its_log() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("saci.yaml"), "llm: {}\n").unwrap();
    let url = format!("http://127.0.0.1:{}", free_port());

    let output = ensure(dir.path(), &["--backend", &url]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let stderr = text(&output.stderr);
    assert!(stderr.contains("stopped before it answered"), "{stderr}");
    assert!(
        stderr.contains("llm"),
        "the log tail names the bad setting: {stderr}"
    );
    assert!(!dir.path().join("saci-backend.pid").exists());
}

#[test]
fn refuses_to_start_without_a_configuration_or_for_a_remote_url() {
    let dir = tempfile::tempdir().unwrap();
    let local = format!("http://127.0.0.1:{}", free_port());

    let missing = ensure(dir.path(), &["--backend", &local]);
    assert_eq!(missing.status.code(), Some(2), "{missing:?}");
    assert!(
        text(&missing.stderr).contains("saci.yaml does not exist"),
        "{missing:?}"
    );

    let remote = ensure(dir.path(), &["--backend", "http://saci.invalid:8080"]);
    assert_eq!(remote.status.code(), Some(3), "{remote:?}");
    assert!(
        text(&remote.stderr).contains("starts only local backends"),
        "{remote:?}"
    );
    assert!(
        !dir.path().join("saci-backend.log").exists(),
        "nothing started"
    );
}
