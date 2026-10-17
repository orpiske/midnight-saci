//! Ordered progress events and the shared evidence recorder of a run.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use saci_core::event::{EventKind, RunEvent};
use saci_core::evidence::{Evidence, EvidenceRecord};
use saci_core::ids::{EvidenceId, RunId};
use saci_core::redaction::Redactor;
use saci_core::run::UnitIdentity;
use time::OffsetDateTime;
use tokio::sync::mpsc;

/// Receives the events of a run, in sequence order.
pub type EventReceiver = mpsc::UnboundedReceiver<RunEvent>;

/// Numbers the events of one run and hands them on in that order.
#[derive(Debug)]
pub struct EventSink {
    run_id: RunId,
    next: Mutex<(u64, mpsc::UnboundedSender<RunEvent>)>,
}

impl EventSink {
    /// Creates the sink of `run_id` and the receiver of its events.
    #[must_use]
    pub fn new(run_id: RunId) -> (Self, EventReceiver) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let sink = Self {
            run_id,
            next: Mutex::new((1, sender)),
        };
        (sink, receiver)
    }

    /// The run the events belong to.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Emits an event about `unit` (and `attempt`), or about the run.
    /// Returns its sequence number.
    pub fn emit(&self, unit: Option<&UnitIdentity>, attempt: Option<u32>, kind: EventKind) -> u64 {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        let seq = next.0;
        next.0 += 1;
        let event = RunEvent {
            run_id: self.run_id.clone(),
            seq,
            at: OffsetDateTime::now_utc(),
            unit: unit.map(|unit| unit.key.clone()),
            attempt,
            source: unit.map(|unit| unit.location.clone()),
            kind,
        };
        if next.1.send(event).is_err() {
            tracing::warn!(seq, "the event receiver is gone; event dropped");
        }
        seq
    }
}

/// Assigns run-unique evidence identifiers, redacts every observation with
/// the run's credentials, and reports it as an event. Every tool, including
/// MCP tools, records through this one path.
#[derive(Debug, Clone)]
pub struct Recorder {
    events: Arc<EventSink>,
    redactor: Redactor,
    recorded: Arc<Mutex<BTreeSet<EvidenceId>>>,
}

impl Recorder {
    /// Creates the recorder of a run.
    #[must_use]
    pub fn new(events: Arc<EventSink>, redactor: Redactor) -> Self {
        Self {
            events,
            redactor,
            recorded: Arc::default(),
        }
    }

    /// The run's event sink.
    #[must_use]
    pub fn events(&self) -> &EventSink {
        &self.events
    }

    /// The run's redactor.
    #[must_use]
    pub const fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    /// Records one observation of `attempt` of `unit`.
    pub fn record(
        &self,
        unit: &UnitIdentity,
        attempt: u32,
        duration: Duration,
        mut record: EvidenceRecord,
    ) -> Evidence {
        self.redactor.redact_evidence(&mut record);
        let id = {
            let mut recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
            let id = EvidenceId::from_index(recorded.len() as u64 + 1);
            recorded.insert(id.clone());
            id
        };
        let evidence = Evidence {
            id,
            unit: unit.key.clone(),
            attempt,
            recorded_at: OffsetDateTime::now_utc(),
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            record,
        };
        let kind = EventKind::Observation {
            evidence: Box::new(evidence.clone()),
        };
        self.events.emit(Some(unit), Some(attempt), kind);
        evidence
    }

    /// Identifiers of every observation recorded in the run so far.
    #[must_use]
    pub fn recorded(&self) -> BTreeSet<EvidenceId> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
