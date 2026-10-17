//! Event-stream fixtures for successful, failed and interrupted runs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use saci_core::event::RunEvent;
use saci_core::ids::{RunId, UnitKey};
use saci_core::run::{ContractError, OutcomeCounts, ReportHeader, RunReport, UnitIdentity};
use saci_core::status::{Lifecycle, Outcome};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    header: ReportHeader,
    units: Vec<UnitIdentity>,
    events: Vec<RunEvent>,
    expect: Expect,
}

#[derive(Deserialize)]
struct Expect {
    outcome: Outcome,
    counts: OutcomeCounts,
    cleanup_not_run: Vec<UnitKey>,
    attempts: BTreeMap<String, usize>,
}

fn load(name: &str) -> Fixture {
    let path = format!("{}/tests/fixtures/runs/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn replay(fixture: &Fixture) -> RunReport {
    let mut report = RunReport::new(fixture.header.clone(), fixture.units.clone());
    for event in &fixture.events {
        report.apply(event).unwrap();
    }
    report
}

fn check(name: &str) -> RunReport {
    let fixture = load(name);
    let report = replay(&fixture);
    assert_eq!(report.lifecycle, Lifecycle::Finished);
    assert_eq!(report.outcome, Some(fixture.expect.outcome));
    assert_eq!(report.counts, fixture.expect.counts);
    assert_eq!(report.cleanup_not_run, fixture.expect.cleanup_not_run);
    for unit in &report.units {
        let expected = fixture.expect.attempts[unit.identity.key.as_str()];
        assert_eq!(
            unit.attempts.len(),
            expected,
            "attempts of {}",
            unit.identity.key
        );
    }
    let json = serde_json::to_string(&report).unwrap();
    assert_eq!(serde_json::from_str::<RunReport>(&json).unwrap(), report);
    report
}

#[test]
fn successful_run_passes_every_unit() {
    let report = check("success.json");
    assert_eq!(report.evidence.len(), 3);
}

#[test]
fn failed_run_stops_and_reports_cleanup_not_run() {
    let report = check("failure.json");
    assert!(
        report.units[1]
            .reason
            .as_deref()
            .unwrap()
            .contains("stopped")
    );
}

#[test]
fn interrupted_run_keeps_every_attempt() {
    let report = check("interrupted.json");
    let retried = &report.units[0];
    assert_eq!(retried.attempts[0].outcome, Some(Outcome::Error));
    assert_eq!(retried.attempts[1].outcome, Some(Outcome::Passed));
    assert_eq!(report.units[1].outcome, Some(Outcome::Cancelled));
    assert_eq!(report.units[2].outcome, Some(Outcome::Skipped));
}

#[test]
fn duplicate_and_out_of_order_events_are_rejected() {
    let fixture = load("success.json");
    let mut report = RunReport::new(fixture.header.clone(), fixture.units.clone());
    report.apply(&fixture.events[0]).unwrap();
    let duplicate = report.apply(&fixture.events[0]).unwrap_err();
    assert!(matches!(
        duplicate,
        ContractError::OutOfOrder {
            expected: 2,
            found: 1
        }
    ));
    let skipped = report.apply(&fixture.events[2]).unwrap_err();
    assert!(matches!(skipped, ContractError::OutOfOrder { .. }));
}

#[test]
fn terminal_state_is_protected() {
    let fixture = load("success.json");
    let mut report = replay(&fixture);
    let mut late = fixture.events.last().unwrap().clone();
    late.seq += 1;
    assert_eq!(report.apply(&late).unwrap_err(), ContractError::Finished);
    assert!(report.transition(Lifecycle::Running).is_err());
}

#[test]
fn events_for_unknown_units_or_other_runs_are_rejected() {
    let fixture = load("success.json");
    let mut report = RunReport::new(fixture.header.clone(), fixture.units.clone());
    report.apply(&fixture.events[0]).unwrap();
    let mut stray = fixture.events[1].clone();
    stray.unit = Some(UnitKey::new("9.9"));
    assert!(matches!(
        report.apply(&stray),
        Err(ContractError::UnknownUnit(_))
    ));
    let mut foreign = fixture.events[1].clone();
    foreign.run_id = RunId::generate();
    assert!(matches!(
        report.apply(&foreign),
        Err(ContractError::WrongRun { .. })
    ));
}
