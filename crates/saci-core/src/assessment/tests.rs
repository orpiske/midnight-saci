use super::*;
use crate::plan::SourceLocation;

fn criteria(count: usize) -> Vec<Criterion> {
    (0..count)
        .map(|index| Criterion {
            index,
            text: format!("criterion {index}"),
            location: SourceLocation::new("plan.md", 10 + index),
        })
        .collect()
}

/// `ev-1` to `ev-3` answered; `ev-4` got no response.
fn recorded() -> Recorded {
    let mut recorded: Recorded = (1..=3).map(|i| (EvidenceId::from_index(i), None)).collect();
    recorded.insert(
        EvidenceId::from_index(4),
        Some("the request got no response (connection refused)".into()),
    );
    recorded
}

fn result(criterion: usize, status: CriterionStatus, evidence: &[u64]) -> CriterionResult {
    CriterionResult {
        criterion,
        status,
        basis: Basis::Mechanical,
        evidence: evidence
            .iter()
            .copied()
            .map(EvidenceId::from_index)
            .collect(),
        explanation: "seen in the output".into(),
    }
}

fn assessment(outcome: Outcome, criteria: Vec<CriterionResult>) -> Assessment {
    Assessment {
        outcome,
        criteria,
        explanation: "summary".into(),
    }
}

fn problems(assessment: &Assessment, count: usize) -> Vec<String> {
    validate(assessment, &criteria(count), &recorded())
        .err()
        .unwrap_or_default()
}

use CriterionStatus::{Contradicted, Satisfied, Undetermined};

#[test]
fn a_supported_pass_is_accepted() {
    let pass = assessment(
        Outcome::Passed,
        vec![result(0, Satisfied, &[1]), result(1, Satisfied, &[2, 3])],
    );
    assert!(problems(&pass, 2).is_empty());
}

#[test]
fn passes_need_every_criterion_satisfied_by_evidence() {
    let partial = assessment(Outcome::Passed, vec![result(0, Satisfied, &[1])]);
    let found = problems(&partial, 2);
    assert!(
        found
            .iter()
            .any(|p| p.contains("criterion 1 (\"criterion 1\") is not assessed"))
    );
    let unsupported = assessment(Outcome::Passed, vec![result(0, Satisfied, &[])]);
    assert!(
        problems(&unsupported, 1)
            .iter()
            .any(|p| p.contains("must cite the evidence"))
    );
    let undetermined = assessment(Outcome::Passed, vec![result(0, Undetermined, &[])]);
    assert!(
        problems(&undetermined, 1)
            .iter()
            .any(|p| p.contains("passed requires"))
    );
}

#[test]
fn nonexistent_evidence_is_rejected() {
    let invented = assessment(Outcome::Passed, vec![result(0, Satisfied, &[9])]);
    let found = problems(&invented, 1);
    assert_eq!(
        found,
        ["criterion 0: evidence `ev-9` was not recorded in this run"]
    );
}

#[test]
fn failures_blocked_and_inconclusive_follow_the_criteria() {
    let failed = assessment(
        Outcome::Failed,
        vec![result(0, Satisfied, &[1]), result(1, Contradicted, &[2])],
    );
    assert!(problems(&failed, 2).is_empty());
    let blocked = assessment(Outcome::Blocked, vec![result(0, Undetermined, &[])]);
    assert!(problems(&blocked, 1).is_empty());
    let inconclusive = assessment(Outcome::Inconclusive, vec![result(0, Undetermined, &[1])]);
    assert!(problems(&inconclusive, 1).is_empty());
    let unfounded = assessment(Outcome::Failed, vec![result(0, Undetermined, &[])]);
    assert!(
        problems(&unfounded, 1)
            .iter()
            .any(|p| p.contains("failed requires"))
    );
    let hidden = assessment(Outcome::Blocked, vec![result(0, Contradicted, &[1])]);
    assert!(
        problems(&hidden, 1)
            .iter()
            .any(|p| p.contains("outcome is failed"))
    );
    let groundless = assessment(Outcome::Inconclusive, vec![result(0, Satisfied, &[1])]);
    assert!(
        problems(&groundless, 1)
            .iter()
            .any(|p| p.contains("undetermined"))
    );
}

#[test]
fn infrastructure_outcomes_and_malformed_results_are_rejected() {
    let error = assessment(Outcome::Error, vec![result(0, Undetermined, &[])]);
    assert!(
        problems(&error, 1)
            .iter()
            .any(|p| p.contains("cannot be assessed"))
    );
    let twice = assessment(
        Outcome::Passed,
        vec![result(0, Satisfied, &[1]), result(0, Satisfied, &[1])],
    );
    assert!(
        problems(&twice, 1)
            .iter()
            .any(|p| p.contains("assessed twice"))
    );
    let unknown = assessment(Outcome::Passed, vec![result(5, Satisfied, &[1])]);
    assert!(
        problems(&unknown, 1)
            .iter()
            .any(|p| p.contains("does not exist"))
    );
    let mut silent = assessment(Outcome::Passed, vec![result(0, Satisfied, &[1])]);
    silent.explanation = " ".into();
    assert!(
        problems(&silent, 1)
            .iter()
            .any(|p| p.contains("needs an explanation"))
    );
}

#[test]
fn submissions_parse_strictly() {
    let json = r#"{"outcome":"passed","criteria":[{"criterion":0,"status":"satisfied","basis":"model_judgment","evidence":["ev-1"],"explanation":"ok"}],"explanation":"ok"}"#;
    let parsed: Assessment = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.criteria[0].basis, Basis::ModelJudgment);
    let extra = json.replacen(
        "\"explanation\":\"ok\"}",
        "\"explanation\":\"ok\",\"x\":1}",
        1,
    );
    assert!(serde_json::from_str::<Assessment>(&extra).is_err());
    let criterion = &schema()["properties"]["criteria"]["items"]["properties"]["criterion"];
    assert!(
        criterion.get("maximum").is_none(),
        "no per-unit bound: {criterion}"
    );
}

#[test]
fn errors_win_over_assessments() {
    let pass = assessment(Outcome::Passed, vec![]);
    assert_eq!(attempt_outcome(Some(&pass), false), Outcome::Passed);
    assert_eq!(attempt_outcome(Some(&pass), true), Outcome::Error);
    assert_eq!(attempt_outcome(None, false), Outcome::Error);
}

#[test]
fn unanswered_evidence_does_not_mechanically_satisfy_a_criterion() {
    let refused = assessment(Outcome::Passed, vec![result(0, Satisfied, &[4])]);
    let found = problems(&refused, 1);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].starts_with("criterion 0: the cited evidence holds no answer from the target (`ev-4`: the request got no response (connection refused))"),
        "{found:?}"
    );
    let answered = assessment(Outcome::Passed, vec![result(0, Satisfied, &[4, 1])]);
    assert!(problems(&answered, 1).is_empty());
    let mut judged = result(0, Satisfied, &[4]);
    judged.basis = Basis::ModelJudgment;
    let expected_failure = assessment(Outcome::Passed, vec![judged]);
    assert!(problems(&expected_failure, 1).is_empty());
    let contradicted = assessment(Outcome::Failed, vec![result(0, Contradicted, &[4])]);
    assert!(problems(&contradicted, 1).is_empty());
}
