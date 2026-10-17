//! Representative valid and invalid plans, plus the bundled starter template.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use saci_core::parser::{DocumentRole, parse_document};

fn fixtures(kind: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/plans")
        .join(kind);
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn valid_plans_parse() {
    for path in fixtures("valid") {
        let source = std::fs::read_to_string(&path).unwrap();
        let plan = parse_document(&name(&path), &source, DocumentRole::Entry)
            .unwrap_or_else(|error| panic!("{}:\n{error}", path.display()));
        assert!(plan.units().all(|unit| !unit.expected.is_empty()));
    }
}

#[test]
fn invalid_plans_report_the_expected_diagnostic() {
    for path in fixtures("invalid") {
        let source = std::fs::read_to_string(&path).unwrap();
        let expected = source
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("<!-- expect: "))
            .and_then(|line| line.strip_suffix(" -->"))
            .unwrap_or_else(|| panic!("{} lacks an expect comment", path.display()));
        let error = parse_document(&name(&path), &source, DocumentRole::Entry).unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains(expected),
            "{}: wanted `{expected}`, got:\n{text}",
            path.display()
        );
    }
}

#[test]
fn mixed_plan_ignores_fenced_headings_and_explanatory_sections() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plans/valid/mixed.md");
    let plan = parse_document(
        "mixed.md",
        &std::fs::read_to_string(path).unwrap(),
        DocumentRole::Entry,
    )
    .unwrap();
    let ids: Vec<_> = plan.units().map(|unit| unit.id.as_str()).collect();
    assert_eq!(ids, ["1.1", "2.1"]);
    assert_eq!(
        plan.units().next().unwrap().procedures[0].target,
        "common/setup.md"
    );
}

#[test]
fn starter_template_parses() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/templates/test-plan.md");
    let source = std::fs::read_to_string(path).unwrap();
    let plan = parse_document("test-plan.md", &source, DocumentRole::Entry).unwrap();
    assert_eq!(plan.title, "Service health");
}
