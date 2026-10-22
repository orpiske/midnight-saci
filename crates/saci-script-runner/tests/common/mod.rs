//! Test scripts, written once per test binary before any test starts a
//! process. Writing an executable while another thread forks can make the
//! later `exec` fail with "text file busy".

#![allow(dead_code)] // Each test binary uses a different subset.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const SCRIPTS: &[(&str, &str)] = &[
    (
        "args.sh",
        "for a in \"$@\"; do printf '[%s]\\n' \"$a\"; done\n",
    ),
    ("exit3.sh", "echo out; echo err >&2; exit 3\n"),
    ("sleep.sh", "sleep \"${1:-30}\"\n"),
    (
        "ignore-term.sh",
        "trap '' TERM\nwhile :; do sleep 1; done\n",
    ),
    // Large output on both streams; stderr first, so reading the streams
    // one after the other would block the script.
    (
        "big.sh",
        "head -c 300000 /dev/zero | tr '\\0' e >&2\nhead -c 300000 /dev/zero | tr '\\0' o\n",
    ),
    ("binary.sh", "printf 'ok\\377\\376'\n"),
    ("env.sh", "env\n"),
    ("pwd.sh", "pwd\n"),
    // A background process in the script's group; the script waits for it.
    ("orphan-wait.sh", "sleep 300 &\necho $! > \"$1\"\nwait\n"),
    // A background process that keeps stdout open after the script exits.
    ("orphan-exit.sh", "sleep 300 &\necho $! > \"$1\"\nexit 0\n"),
];

/// The directory with every test script.
pub fn scripts() -> &'static Path {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in SCRIPTS {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    })
    .path()
}

pub fn script(name: &str) -> PathBuf {
    scripts().join(name)
}

/// Reads the PID that a script wrote to `file`.
pub async fn read_pid(file: &Path) -> i32 {
    for _ in 0..100 {
        if let Some(pid) = std::fs::read_to_string(file)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no PID in {}", file.display());
}

/// True when the process is gone or a zombie, within a few seconds.
#[cfg(target_os = "linux")]
pub async fn gone(pid: i32) -> bool {
    for _ in 0..50 {
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = state.rsplit(')').next().unwrap_or_default().trim_start();
        if state.is_empty() || state.starts_with('Z') {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}
