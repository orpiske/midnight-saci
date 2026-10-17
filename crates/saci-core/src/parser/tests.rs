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
