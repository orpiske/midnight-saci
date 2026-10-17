//! Every Saci entry point reports its version and help without side effects.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

fn run(binary: &str, arg: &str) -> String {
    let output = Command::new(binary).arg(arg).output().unwrap();
    assert!(output.status.success(), "{binary} {arg} failed");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn cli_reports_version_and_help() {
    let version = run(env!("CARGO_BIN_EXE_saci"), "--version");
    assert!(version.starts_with("saci "));
    assert!(run(env!("CARGO_BIN_EXE_saci"), "--help").contains("Usage: saci"));
}
