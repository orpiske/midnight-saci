use std::collections::BTreeMap;

use super::*;

const CONFIG: &str = "llm:
  base_url: http://model:11434/v1
  model: test-model
  api_key: env:LLM_KEY
limits:
  step_timeout_secs: 300
environments:
  staging:
    description: Shared staging cluster
    parameters:
      URL: http://internal.example
    secrets:
      TARGET_TOKEN:
        env: TARGET_TOKEN_SOURCE
    limits:
      run_timeout_secs: 900
    mcp:
      tickets: {}
mcp_servers:
  tickets:
    url: http://tickets.internal/mcp
    headers:
      Authorization: Bearer ${TARGET_TOKEN}
policies:
  nightly:
    on_failure: continue
    retry:
      max_attempts: 2
      on: [failed]
    actions: [file-bug]
actions:
  servers:
    tracker:
      url: http://tracker.internal/mcp
  definitions:
    file-bug:
      server: tracker
      tool: create_issue
      on: [failed]
      approval: required
      arguments:
        title: 'Saci: {{plan.title}}'
";

fn config() -> Config {
    Config::from_yaml(CONFIG, "test.yaml").unwrap()
}

#[test]
fn environments_show_limits_and_server_names_but_no_secrets() {
    let environments = environments(&config());
    assert_eq!(environments.len(), 1);
    let staging = &environments[0];
    assert_eq!(
        staging.description.as_deref(),
        Some("Shared staging cluster")
    );
    assert_eq!(staging.limits.run_timeout_secs, 900);
    assert_eq!(staging.limits.step_timeout_secs, 300);
    assert_eq!(staging.mcp_servers, ["tickets"]);
    let json = serde_json::to_string(&environments).unwrap();
    for hidden in [
        "TARGET_TOKEN",
        "internal.example",
        "tickets.internal",
        "Bearer",
    ] {
        assert!(!json.contains(hidden), "{hidden} leaked: {json}");
    }
}

#[test]
fn policies_include_the_implied_default_and_action_metadata_only() {
    let policies = policies(&config());
    let names: Vec<_> = policies.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["default", "nightly"]);
    assert!(!policies[0].configured);
    assert_eq!(policies[0].retry.max_attempts, 1);
    assert_eq!(policies[0].on_failure, FailureAction::Stop);
    let nightly = &policies[1];
    assert_eq!(nightly.retry.max_attempts, 2);
    assert_eq!(nightly.actions[0].tool, "create_issue");
    assert_eq!(nightly.actions[0].approval, Approval::Required);
    let json = serde_json::to_string(&policies).unwrap();
    assert!(!json.contains("plan.title"), "arguments leaked: {json}");
    assert!(!json.contains("tracker.internal"), "server leaked: {json}");
}

fn bundle(entry: &str) -> PlanBundle {
    let setup = "# Procedure: Setup\n\n## Phase 1: Main\n\n### Step 1: Prepare\n\nRun `prepare`.\n\n**Expected result:**\n\n- Ready.\n";
    PlanBundle {
        entry: "plan.md".into(),
        documents: BTreeMap::from([
            ("plan.md".to_owned(), entry.to_owned()),
            ("common/setup.md".to_owned(), setup.to_owned()),
        ]),
    }
}

#[test]
fn the_plan_view_keeps_repeated_procedure_invocations_distinct() {
    let entry = "# Test Plan: Entry\n\n## Phase 1: Main\n\n### Test 1.1: First\n\nFollow [setup](common/setup.md).\n\n**Expected result:**\n\n- One.\n\n### Test 1.2: Second\n\nFollow [setup](common/setup.md).\n\n**Expected result:**\n\n- Two.\n\n**Verification:** check twice.\n";
    let plan = run_plan(RunId::generate(), "Entry", &bundle(entry));
    assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
    let keys: Vec<_> = plan.units.iter().map(|u| u.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "1.1>common/setup.md#1",
            "1.1",
            "1.2>common/setup.md#1",
            "1.2"
        ]
    );
    let nested = &plan.units[2];
    assert_eq!(nested.location.document, "common/setup.md");
    assert_eq!(nested.invoked_by, [UnitKey::new("1.2")]);
    assert_eq!(nested.criteria[0].text, "Ready.");
    let second = &plan.units[3];
    assert_eq!(second.procedures[0].target, "common/setup.md");
    assert!(second.verification.is_some());
    assert_eq!(plan.documents.len(), 2);
}

#[test]
fn a_bundle_that_no_longer_expands_reports_diagnostics() {
    let plan = run_plan(
        RunId::generate(),
        "Broken",
        &bundle("# Test Plan: Broken\n"),
    );
    assert!(plan.units.is_empty());
    assert_eq!(plan.diagnostics[0].document, "plan.md");
    assert!(plan.diagnostics[0].line >= 1);
    assert_eq!(plan.documents.len(), 2);
}
