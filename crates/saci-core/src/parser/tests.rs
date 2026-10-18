use super::*;

fn parse(source: &str) -> Plan {
    parse_document("plan.md", source, DocumentRole::Entry).unwrap()
}

const PLAN: &str = "# Test Plan: Example\n\n## Overview\n\nGoal.\n\n## Phase 1: Checks\n\n### Test 1.1: Health\n\nSend a GET request.\n\n```bash\ncurl -s \"${SERVICE_URL}/health\"\n```\n\n**Expected result:**\n\n- Status is 200.\n- Body says `up`.\n\n**Verification:**\n\nRecord the status.\n";

#[test]
fn units_keep_action_commands_criteria_and_locations() {
    let plan = parse(PLAN);
    assert_eq!(plan.title, "Example");
    assert_eq!(plan.overview.as_deref(), Some("Goal."));
    let unit = plan.units().next().unwrap();
    assert_eq!(
        (unit.kind, unit.id.as_str(), unit.title.as_str()),
        (UnitKind::Test, "1.1", "Health")
    );
    assert_eq!(unit.location.line, 9);
    assert!(unit.action.starts_with("Send a GET request."));
    assert!(unit.action.contains("```bash"));
    assert!(!unit.action.contains("Expected"));
    assert_eq!(unit.commands[0].language, "bash");
    assert_eq!(unit.commands[0].code, "curl -s \"${SERVICE_URL}/health\"\n");
    let criteria: Vec<_> = unit.expected.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(criteria, ["Status is 200.", "Body says `up`."]);
    assert_eq!(unit.expected[1].index, 1);
    assert_eq!(unit.verification.as_deref(), Some("Record the status."));
}

#[test]
fn procedure_title_is_required_for_procedures() {
    let source = PLAN.replace("# Test Plan: Example", "# Procedure: Setup");
    assert!(parse_document("setup.md", &source, DocumentRole::Procedure).is_ok());
    let error = parse_document("setup.md", PLAN, DocumentRole::Procedure).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("expected `# Procedure: <title>`"),
        "{error}"
    );
}

#[test]
fn only_follow_paragraphs_invoke_procedures() {
    let source = PLAN.replace(
        "Send a GET request.",
        "Follow [setup](common/setup.md).\n\nSee [notes](common/notes.md) first.",
    );
    let plan = parse(&source);
    let unit = plan.units().next().unwrap();
    assert_eq!(unit.procedures.len(), 1);
    assert_eq!(unit.procedures[0].target, "common/setup.md");
    assert_eq!(unit.procedures[0].location.line, 11);
}

#[test]
fn units_follow_document_order_not_id_order() {
    let source = "# Test Plan: Order\n\n## Phase 2: Later\n\n### Step 2.1: B\n\nDo B.\n\n**Expected result:**\n\n- B.\n\n## Phase 1: Earlier\n\n### Step 1.1: A\n\nDo A.\n\n**Expected result:**\n\n- A.\n";
    let ids: Vec<_> = parse(source).units().map(|unit| unit.id.clone()).collect();
    assert_eq!(ids, ["2.1", "1.1"]);
}

#[test]
fn every_problem_is_reported_at_once() {
    let source =
        "# Test Plan: Many\n\n## Phase 1: Checks\n\n### Test 1.1: A\n\n### Test 1.2: B\n\nDo B.\n";
    let error = parse_document("plan.md", source, DocumentRole::Entry).unwrap_err();
    let lines: Vec<_> = error.diagnostics.iter().map(|d| d.location.line).collect();
    assert_eq!(lines, [5, 5, 7]);
}

#[test]
fn split_labeled_requires_id_colon_and_title() {
    assert_eq!(
        split_labeled("Test 1.1: Health", "Test"),
        Some(("1.1".into(), "Health".into()))
    );
    assert_eq!(split_labeled("Test 1.1 Health", "Test"), None);
    assert_eq!(split_labeled("Test : Health", "Test"), None);
    assert_eq!(split_labeled("Testing: x", "Test"), None);
}

#[test]
fn verification_text_on_the_marker_line_is_kept() {
    let verification = |marker: &str| {
        let source = PLAN.replace("**Verification:**\n\nRecord the status.", marker);
        let plan = parse(&source);
        plan.units().next().unwrap().verification.clone()
    };
    let same_line = verification("**Verification:** Record the status.");
    assert_eq!(same_line.as_deref(), Some("Record the status."));
    let outside = verification("**Verification**: Record the status.\n\nThen compare.");
    assert_eq!(
        outside.as_deref(),
        Some("Record the status.\n\nThen compare.")
    );
    let separate = verification("**Verification:**\n\nRecord the status.");
    assert_eq!(separate.as_deref(), Some("Record the status."));
    assert_eq!(verification("**Verification:**"), None);
}

#[test]
fn warns_about_content_that_never_runs() {
    let source = "# Test Plan: Example\n\n## Prerequisites\n\nFollow [common/env.md](common/env.md).\n\n## Phase 0: Login\n\nFollow [common/login.md](common/login.md).\n\n## Phase 1: Checks\n\n```bash\nexport A=1\n```\n\n### Test 1.1: Health\n\nRun `true`.\n\n**Expected result:**\n\n- It exits 0.\n";
    let (plan, warnings) =
        parse_document_with_warnings("plan.md", source, DocumentRole::Entry).unwrap();
    assert_eq!(plan.units().count(), 1);
    assert!(plan.units().all(|unit| unit.procedures.is_empty()));
    let found: Vec<_> = warnings
        .iter()
        .map(|w| {
            (
                w.location.line,
                w.message.split_whitespace().next().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [(5, "`Follow"), (7, "Phase"), (9, "`Follow"), (13, "this")],
        "{warnings:?}"
    );
}

#[test]
fn warns_about_prerequisite_commands_and_blocks_after_the_criteria() {
    let source = "# Test Plan: Example\n\n## Prerequisites\n\n```bash\nexport A=1\n```\n\n## Phase 1: Checks\n\n### Test 1.1: Health\n\nRun `true`.\n\n**Expected result:**\n\n- It exits 0.\n\n```bash\necho late\n```\n\n> Note: kept?\n";
    let (plan, warnings) =
        parse_document_with_warnings("plan.md", source, DocumentRole::Entry).unwrap();
    assert!(plan.units().all(|unit| unit.commands.is_empty()));
    let lines: Vec<_> = warnings.iter().map(|w| w.location.line).collect();
    assert_eq!(lines, [5, 19, 23], "{warnings:?}");
    assert!(warnings[0].message.contains("Prerequisites"));
    assert!(warnings[1].message.contains("after `**Expected result:**`"));
}

#[test]
fn a_well_formed_plan_has_no_warnings() {
    let (_, warnings) = parse_document_with_warnings("plan.md", PLAN, DocumentRole::Entry).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn code_blocks_are_found_at_any_depth() {
    let source = "# Notes\n\n```bash\necho top\n```\n\n- item:\n\n  ```sh\n  echo listed\n  ```\n\n> ```\n> quoted\n> ```\n";
    let found: Vec<_> = code_blocks("notes.md", source)
        .into_iter()
        .map(|block| (block.language, block.code, block.location.line))
        .collect();
    assert_eq!(
        found,
        [
            ("bash".to_owned(), "echo top\n".to_owned(), 3),
            ("sh".to_owned(), "echo listed\n".to_owned(), 9),
            (String::new(), "quoted\n".to_owned(), 13),
        ]
    );
}
