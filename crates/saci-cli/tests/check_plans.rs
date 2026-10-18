//! `saci check` validates plan bundles offline and warns about content that
//! would never run.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Output};

const UNIT: &str = "### Test 1.1: Health\n\nRun `true`.\n\n**Expected result:**\n\n- It exits 0.\n";

fn check(plan: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_saci"))
        .arg("check")
        .arg(plan)
        .args(extra)
        .output()
        .unwrap()
}

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn summarizes_a_valid_bundle_with_its_procedures() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "common/setup.md",
        &format!(
            "# Procedure: Setup\n\n## Phase 1: Setup\n\n{}",
            UNIT.replace("Test", "Step")
        ),
    );
    let plan = write(
        dir.path(),
        "plan.md",
        "# Test Plan: Example\n\n## Phase 1: Checks\n\n### Step 1.1: Prepare\n\nFollow [common/setup.md](common/setup.md).\n\n**Expected result:**\n\n- Setup is done.\n\n### Test 1.2: Health\n\nRun `true`.\n\n**Expected result:**\n\n- It exits 0.\n",
    );
    let output = check(&plan, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        stdout.contains("\"Example\", 3 units (1 test, 2 steps), 1 procedure, 0 warnings."),
        "{stdout}"
    );
}

#[test]
fn reports_every_problem_and_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let plan = write(
        dir.path(),
        "plan.md",
        "# Test Plan: Example\n\n## Phase 1: Checks\n\n### Test 1.1: A\n\nRun `true`.\n\n### Test 1.2: B\n\nRun `true`.\n",
    );
    let output = check(&plan, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr.contains("plan.md:5: Test 1.1 has no expected result"),
        "{stderr}"
    );
    assert!(
        stderr.contains("plan.md:9: Test 1.2 has no expected result"),
        "{stderr}"
    );
}

#[test]
fn warns_about_phases_that_never_run_and_fails_them_when_strict() {
    let dir = tempfile::tempdir().unwrap();
    let plan = write(
        dir.path(),
        "plan.md",
        &format!(
            "# Test Plan: Example\n\n## Phase 0: Login\n\nFollow [login.md](login.md).\n\n## Phase 1: Checks\n\n{UNIT}"
        ),
    );
    let output = check(&plan, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        stderr.contains("warning: plan.md:3: Phase 0 has no tests or steps"),
        "{stderr}"
    );
    assert!(
        stderr.contains("warning: plan.md:5: `Follow [login.md]` is outside"),
        "{stderr}"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 warnings"));
    assert_eq!(check(&plan, &["--strict"]).status.code(), Some(2));
}

const STEP: &str = "### Step 1.1: Set up\n\n```bash\nexport A=1\n```\n\n**Expected result:**\n\n- It exits 0 (inferred).\n\n";
const TEST: &str =
    "### Test 1.2: Health\n\n```bash\ntrue\n```\n\n**Expected result:**\n\n- It exits 0.\n";

/// An original plan with a block in Prerequisites, and a converted copy
/// whose last test is `test`. Returns (original dir, converted dir, plan).
fn converted(test: &str) -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let original = tempfile::tempdir().unwrap();
    write(
        original.path(),
        "plan.md",
        "# Plan\n\n## Prerequisites\n\n```bash\nexport A=1\n```\n\n### Test 1.1: Health\n\n```bash\ntrue\n```\n",
    );
    let converted = tempfile::tempdir().unwrap();
    let plan = write(
        converted.path(),
        "plan.md",
        &format!("# Test Plan: Plan\n\n## Phase 1: Checks\n\n{STEP}{test}"),
    );
    (original, converted, plan)
}

#[test]
fn accepts_code_blocks_that_moved_but_did_not_change() {
    let (original, _converted, plan) = converted(TEST);
    let output = check(&plan, &["--original", original.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 code blocks in 1 document match"));
}

#[test]
fn reports_changed_code_blocks_on_both_sides() {
    let (original, _converted, plan) = converted(&TEST.replace("true", "true # fixed"));
    let output = check(&plan, &["--original", original.path().to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr.contains("plan.md:11: original code block missing or changed: `true`"),
        "{stderr}"
    );
    assert!(
        stderr.contains("plan.md:17: code block not in the originals: `true # fixed`"),
        "{stderr}"
    );
}
