//! An investigation run's events build its report: one unit, one attempt,
//! and the checked conclusion that decides the attempt's outcome.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use saci_core::event::{EventKind, RunEvent};
use saci_core::evidence::{CapturedText, Evidence, EvidenceRecord, ShellEvidence, Termination};
use saci_core::ids::{EvidenceId, RunId, UnitKey};
use saci_core::investigation::{
    CodeLocation, Confidence, Hypothesis, INVESTIGATION_UNIT, InvestigationOutcome,
    InvestigationReport, InvestigationSpec, Source,
};
use saci_core::run::{ContractError, ReportHeader, RunReport};
use saci_core::status::Outcome;
use time::OffsetDateTime;

const SPEC: &str = r#"{"parent_run":"run-2b","plan_title":"Orders",
    "unit":{"key":"1.2","id":"1.2","kind":"step","title":"Check orders","phase":"Smoke",
      "location":{"document":"plan.md","line":9},"cleanup":false,"outcome":"failed"},
    "repositories":[{"name":"orders","url":"https://example.com/o.git","ref":"v2"}]}"#;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

struct Run {
    report: RunReport,
    seq: u64,
}

impl Run {
    fn new() -> Self {
        let spec: InvestigationSpec = serde_json::from_str(SPEC).unwrap();
        let header = ReportHeader {
            run_id: RunId::parse("run-1a").unwrap(),
            plan_title: "Orders".into(),
            entry: String::new(),
            environment: "investigator".into(),
            policy: "default".into(),
            model: "m".into(),
            created_at: OffsetDateTime::now_utc(),
        };
        let mut run = Self {
            report: RunReport::new(header, vec![spec.unit_identity()]),
            seq: 0,
        };
        let model = "m".to_owned();
        run.apply(
            None,
            EventKind::RunStarted {
                model,
                environment: None,
            },
        )
        .unwrap();
        run.apply(Some(1), EventKind::AttemptStarted).unwrap();
        let observation = EventKind::Observation {
            evidence: Box::new(evidence()),
        };
        run.apply(Some(1), observation).unwrap();
        run
    }

    fn apply(&mut self, attempt: Option<u32>, kind: EventKind) -> Result<(), ContractError> {
        let unit = attempt.map(|_| UnitKey::new(INVESTIGATION_UNIT));
        let event = RunEvent {
            run_id: self.report.header.run_id.clone(),
            seq: self.seq + 1,
            at: OffsetDateTime::now_utc(),
            unit,
            attempt,
            source: None,
            kind,
        };
        self.report.apply(&event)?;
        self.seq += 1;
        Ok(())
    }

    fn conclude(&mut self, report: InvestigationReport) -> Result<(), ContractError> {
        let kind = EventKind::InvestigationConcluded {
            report: Box::new(report),
        };
        self.apply(Some(1), kind)
    }

    fn finish_attempt(&mut self, outcome: Outcome) -> Result<(), ContractError> {
        let kind = EventKind::AttemptFinished {
            outcome,
            assessment: None,
            error: None,
            timing: None,
        };
        self.apply(Some(1), kind)
    }
}

fn evidence() -> Evidence {
    Evidence {
        id: EvidenceId::from_index(1),
        unit: UnitKey::new(INVESTIGATION_UNIT),
        attempt: 1,
        recorded_at: OffsetDateTime::now_utc(),
        duration_ms: 5,
        record: EvidenceRecord::Shell(ShellEvidence {
            command: "git clone ...".into(),
            stdout: CapturedText::complete(COMMIT),
            stderr: CapturedText::complete(""),
            exit_code: Some(0),
            termination: Termination::Exited,
            restarted: false,
            session_lost: false,
        }),
    }
}

fn concluded() -> InvestigationReport {
    InvestigationReport {
        outcome: InvestigationOutcome::Concluded,
        summary: "The handler drops the currency.".into(),
        sources: vec![Source {
            name: "orders".into(),
            commit: COMMIT.into(),
        }],
        hypotheses: vec![Hypothesis {
            statement: "`total` ignores the currency.".into(),
            confidence: Confidence::High,
            evidence: vec![EvidenceId::from_index(1)],
            code: vec![CodeLocation {
                repository: "orders".into(),
                commit: COMMIT.into(),
                path: "src/total.rs".into(),
                start_line: 3,
                end_line: 9,
            }],
        }],
        bug: None,
        no_bug_reason: Some("Filing is off in this test.".into()),
    }
}

#[test]
fn a_conclusion_passes_the_investigation_run() {
    let mut run = Run::new();
    run.conclude(concluded()).unwrap();
    run.finish_attempt(Outcome::Passed).unwrap();
    run.apply(
        Some(1),
        EventKind::UnitFinished {
            outcome: Outcome::Passed,
        },
    )
    .unwrap();
    let finished = EventKind::RunFinished {
        outcome: Outcome::Passed,
        reason: "concluded".into(),
    };
    run.apply(None, finished).unwrap();
    assert_eq!(run.report.outcome, Some(Outcome::Passed));
    assert_eq!(run.report.investigation.as_deref(), Some(&concluded()));
}

#[test]
fn the_attempt_outcome_follows_the_conclusion() {
    let mut run = Run::new();
    let mut report = concluded();
    report.outcome = InvestigationOutcome::Inconclusive;
    run.conclude(report).unwrap();
    assert!(run.finish_attempt(Outcome::Passed).is_err());
    run.finish_attempt(Outcome::Inconclusive).unwrap();
}

#[test]
fn without_a_conclusion_an_attempt_can_only_end_in_error() {
    let mut run = Run::new();
    assert!(run.finish_attempt(Outcome::Passed).is_err());
    run.finish_attempt(Outcome::Error).unwrap();
    assert!(run.report.investigation.is_none());
}

#[test]
fn invalid_conclusions_are_rejected_and_change_nothing() {
    let mut run = Run::new();
    let mut unknown = concluded();
    unknown.hypotheses[0].evidence = vec![EvidenceId::from_index(9)];
    let error = run.conclude(unknown).unwrap_err().to_string();
    assert!(error.contains("unknown evidence ev-9"), "{error}");

    let mut uncloned = concluded();
    uncloned.hypotheses[0].code[0].commit = "f".repeat(40);
    assert!(run.conclude(uncloned).is_err());

    let mut claimed = concluded();
    claimed.outcome = InvestigationOutcome::Error;
    assert!(run.conclude(claimed).is_err());
    assert!(run.report.investigation.is_none());

    run.conclude(concluded()).unwrap();
    let error = run.conclude(concluded()).unwrap_err().to_string();
    assert!(error.contains("already concluded"), "{error}");
}
