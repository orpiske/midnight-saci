use super::*;

fn unit_plan(title: &str, prefix: &str, body: &str) -> String {
    format!(
        "# {prefix}: {title}\n\n## Phase 1: Main\n\n### Step 1: Do it\n\n{body}\n\n**Expected result:**\n\n- It is done.\n"
    )
}

fn write(dir: &Path, path: &str, text: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

fn plan_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let entry = "# Test Plan: Entry\n\n## Phase 1: Setup\n\n### Step 1.1: Prepare\n\nFollow [setup](common/setup.md).\n\n**Expected result:**\n\n- Ready.\n\n### Test 1.2: Again\n\nFollow [setup](./common/setup.md).\n\n**Expected result:**\n\n- Still ready.\n\n## Phase 2: Cleanup\n\n### Step 2.1: Remove data\n\nRemove the data.\n\n**Expected result:**\n\n- Gone.\n";
    write(dir.path(), "plan.md", entry);
    write(
        dir.path(),
        "common/setup.md",
        &unit_plan("Setup", "Procedure", "Follow [login](../shared/login.md)."),
    );
    write(
        dir.path(),
        "shared/login.md",
        &unit_plan("Login", "Procedure", "Run `login`."),
    );
    dir
}

#[test]
fn bundles_include_invoked_procedures_and_work_without_the_checkout() {
    let dir = plan_dir();
    let bundle = load(&dir.path().join("plan.md")).unwrap();
    let paths: Vec<_> = bundle.documents.keys().cloned().collect();
    assert_eq!(paths, ["common/setup.md", "plan.md", "shared/login.md"]);
    drop(dir);
    let expanded = expand(&bundle).unwrap();
    assert_eq!(expanded.procedures.len(), 2);
}

#[test]
fn procedures_run_before_the_invoking_unit_with_distinct_keys() {
    let dir = plan_dir();
    let expanded = expand(&load(&dir.path().join("plan.md")).unwrap()).unwrap();
    let keys: Vec<_> = expanded
        .units
        .iter()
        .map(|u| u.identity.key.as_str().to_owned())
        .collect();
    assert_eq!(
        keys,
        [
            "1.1>common/setup.md#1>shared/login.md#1",
            "1.1>common/setup.md#1",
            "1.1",
            "1.2>common/setup.md#1>shared/login.md#1",
            "1.2>common/setup.md#1",
            "1.2",
            "2.1",
        ]
    );
    assert_eq!(expanded.units[0].document, "shared/login.md");
    assert_eq!(expanded.units[0].invoked_by.len(), 2);
    assert!(expanded.units[6].identity.cleanup);
    assert!(!expanded.units[2].identity.cleanup);
}

fn error_of(bundle: &PlanBundle) -> String {
    expand(bundle).unwrap_err().to_string()
}

fn bundle(documents: &[(&str, &str)]) -> PlanBundle {
    PlanBundle {
        entry: "plan.md".into(),
        documents: documents
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

#[test]
fn missing_escaping_and_external_procedures_are_rejected() {
    let missing = bundle(&[(
        "plan.md",
        &unit_plan("P", "Test Plan", "Follow [x](missing.md)."),
    )]);
    assert!(error_of(&missing).contains("plan.md:7: `missing.md` is not in the bundle"));
    let escape = bundle(&[(
        "plan.md",
        &unit_plan("P", "Test Plan", "Follow [x](../outside.md)."),
    )]);
    assert!(error_of(&escape).contains("the path leaves the bundle"));
    let external = bundle(&[(
        "plan.md",
        &unit_plan("P", "Test Plan", "Follow [x](https://example.com/p.md)."),
    )]);
    assert!(error_of(&external).contains("external links cannot be invoked"));
    let bad_key = bundle(&[
        ("plan.md", &unit_plan("P", "Test Plan", "Do it.")),
        ("../x.md", "x"),
    ]);
    assert!(error_of(&bad_key).contains("../x.md:1: bundle paths must be relative"));
}

#[test]
fn recursive_invocations_are_rejected() {
    let recursive = bundle(&[
        ("plan.md", &unit_plan("P", "Test Plan", "Follow [a](a.md).")),
        ("a.md", &unit_plan("A", "Procedure", "Follow [b](b.md).")),
        ("b.md", &unit_plan("B", "Procedure", "Follow [a](a.md).")),
    ]);
    assert!(
        error_of(&recursive)
            .contains("recursive procedure invocation: plan.md -> a.md -> b.md -> a.md")
    );
}

#[test]
fn ordinary_links_are_references_not_invocations() {
    let reference = bundle(&[(
        "plan.md",
        &unit_plan("P", "Test Plan", "See [notes](notes.md) before you start."),
    )]);
    let expanded = expand(&reference).unwrap();
    assert_eq!(expanded.units.len(), 1);
}

#[test]
fn loading_rejects_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "plan/plan.md",
        &unit_plan("P", "Test Plan", "Follow [x](gone.md)."),
    );
    let error = load(&dir.path().join("plan/plan.md"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("plan.md:7: cannot read `gone.md`"),
        "{error}"
    );
}

#[test]
fn loading_rejects_escaping_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "outside.md",
        &unit_plan("O", "Procedure", "Do it."),
    );
    write(
        dir.path(),
        "plan/plan.md",
        &unit_plan("P", "Test Plan", "Follow [x](link.md)."),
    );
    std::os::unix::fs::symlink(
        dir.path().join("outside.md"),
        dir.path().join("plan/link.md"),
    )
    .unwrap();
    let error = load(&dir.path().join("plan/plan.md"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("resolves outside the bundle directory"),
        "{error}"
    );
}

#[test]
fn a_procedure_followed_twice_by_one_unit_gets_a_key_per_occurrence() {
    let entry = "# Test Plan: Twice\n\n## Phase 1: Main\n\n### Step 1.1: Log in twice\n\nFollow [login](login.md).\n\nRun `check`.\n\nFollow [login](login.md).\n\n**Expected result:**\n\n- Logged in.\n";
    let login = unit_plan("Login", "Procedure", "Run `login`.");
    let expanded = expand(&bundle(&[("plan.md", entry), ("login.md", &login)])).unwrap();
    let keys: Vec<_> = expanded
        .units
        .iter()
        .map(|u| u.identity.key.as_str().to_owned())
        .collect();
    assert_eq!(keys, ["1.1>login.md#1", "1.1>login.md@2#1", "1.1"]);
}
