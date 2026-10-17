//! `saci new test-plan` works offline, never overwrites, and produces a
//! plan the parser accepts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Output};

use saci_core::parser::{DocumentRole, parse_document};

fn saci(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_saci"))
        .args(args)
        .output()
        .unwrap()
}

fn new_plan(path: &std::path::Path) -> Output {
    saci(&["new".as_ref(), "test-plan".as_ref(), path.as_os_str()])
}

#[test]
fn creates_a_valid_plan_titled_after_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saci-on-the-cloud.md");
    let output = new_plan(&path);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("saci-on-the-cloud.md"));
    let text = std::fs::read_to_string(&path).unwrap();
    let plan = parse_document("saci-on-the-cloud.md", &text, DocumentRole::Entry).unwrap();
    assert_eq!(plan.title, "Saci on the cloud");
}

#[test]
fn refuses_to_overwrite_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.md");
    std::fs::write(&path, "keep me").unwrap();
    let output = new_plan(&path);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
}

#[test]
fn reports_a_missing_directory() {
    let dir = tempfile::tempdir().unwrap();
    let output = new_plan(&dir.path().join("missing/plan.md"));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
}
