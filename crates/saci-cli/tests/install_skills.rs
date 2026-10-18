//! `saci skill` installs the embedded skills where each agent finds them and
//! never replaces a changed skill without --force.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Output};

const SKILL: &str = "saci-convert-test-plan";

fn saci(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_saci"))
        .current_dir(dir)
        .env("HOME", dir.join("home"))
        .args(args)
        .output()
        .unwrap()
}

fn installed(path: &Path) -> String {
    std::fs::read_to_string(path.join(SKILL).join("SKILL.md")).unwrap()
}

#[test]
fn lists_the_embedded_skills_with_their_descriptions() {
    let dir = tempfile::tempdir().unwrap();
    let output = saci(dir.path(), &["skill", "list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.starts_with("saci-convert-test-plan\tConvert an existing Markdown test plan"),
        "{stdout}"
    );
}

#[test]
fn installs_into_each_agents_project_directory() {
    let dir = tempfile::tempdir().unwrap();
    let output = saci(
        dir.path(),
        &[
            "skill", "install", SKILL, "-a", "claude", "-a", "codex", "-a", "copilot",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let claude = installed(&dir.path().join(".claude/skills"));
    assert!(claude.starts_with("---\nname: saci-convert-test-plan\n"));
    assert_eq!(installed(&dir.path().join(".agents/skills")), claude);
    assert_eq!(installed(&dir.path().join(".github/skills")), claude);
}

#[test]
fn installs_into_the_home_directory_with_global() {
    let dir = tempfile::tempdir().unwrap();
    let output = saci(
        dir.path(),
        &[
            "skill", "install", "--all", "-a", "copilot", "-a", "opencode", "--global",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    installed(&dir.path().join("home/.copilot/skills"));
    installed(&dir.path().join("home/.config/opencode/skills"));
    assert!(!dir.path().join(".github").exists());
}

#[test]
fn installs_into_any_directory_with_dir() {
    let dir = tempfile::tempdir().unwrap();
    let output = saci(
        dir.path(),
        &["skill", "install", SKILL, "--dir", "tools/skills"],
    );
    assert!(output.status.success(), "{output:?}");
    installed(&dir.path().join("tools/skills"));
}

#[test]
fn keeps_a_changed_skill_unless_forced() {
    let dir = tempfile::tempdir().unwrap();
    let args = ["skill", "install", SKILL, "-a", "claude"];
    assert!(saci(dir.path(), &args).status.success());
    let up_to_date = saci(dir.path(), &args);
    assert!(String::from_utf8_lossy(&up_to_date.stdout).contains("is up to date"));

    let file = dir
        .path()
        .join(".claude/skills")
        .join(SKILL)
        .join("SKILL.md");
    std::fs::write(&file, "my edits").unwrap();
    let refused = saci(dir.path(), &args);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("use --force"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "my edits");

    assert!(
        saci(dir.path(), &[&args[..], &["--force"]].concat())
            .status
            .success()
    );
    assert!(std::fs::read_to_string(&file).unwrap().starts_with("---\n"));
}

#[test]
fn rejects_unknown_skills_and_a_missing_destination() {
    let dir = tempfile::tempdir().unwrap();
    let unknown = saci(dir.path(), &["skill", "install", "nope", "-a", "claude"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("available: saci-convert-test-plan"));
    let nowhere = saci(dir.path(), &["skill", "install", SKILL]);
    assert_eq!(nowhere.status.code(), Some(2));
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}
