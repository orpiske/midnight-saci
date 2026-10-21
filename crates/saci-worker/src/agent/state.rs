//! What one attempt's tools and hook share: limits, counters, the accepted
//! assessment and why the attempt was halted.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use rig::completion::Usage;
use saci_core::assessment::Assessment;
use saci_core::evidence::{Evidence, EvidenceRecord, RejectedCall};
use saci_core::plan::Criterion;
use saci_core::run::{AttemptTiming, ErrorKind, ExecutionError, UnitIdentity};
use saci_core::spec::ExecutionLimits;
use tokio_util::sync::CancellationToken;

use crate::events::Recorder;

/// State of one attempt.
#[derive(Debug)]
pub struct AttemptState {
    pub unit: UnitIdentity,
    pub attempt: u32,
    pub criteria: Vec<Criterion>,
    pub limits: ExecutionLimits,
    pub deadline: Instant,
    /// Cancelled with the run, or when the attempt deadline passes.
    pub cancel: CancellationToken,
    pub recorder: Recorder,
    model_calls: AtomicU32,
    tool_calls: AtomicU32,
    tool_running: AtomicBool,
    assessment: Mutex<Option<Assessment>>,
    /// Why a limit or the tool infrastructure ended the attempt.
    halt: Mutex<Option<ExecutionError>>,
    problems: Mutex<Vec<String>>,
    timing: Mutex<Clock>,
}

/// Where the attempt spent its time so far.
#[derive(Debug, Default)]
struct Clock {
    model_requests: u32,
    model: Duration,
    /// When the model request in flight was sent.
    model_since: Option<Instant>,
    tool: Duration,
    input_tokens: u64,
    cached_tokens: Option<u64>,
    output_tokens: u64,
}

impl Clock {
    fn settle_model(&mut self) {
        if let Some(since) = self.model_since.take() {
            self.model += since.elapsed();
        }
    }
}

/// What an attempt starts from.
#[derive(Debug)]
pub struct Setup {
    pub unit: UnitIdentity,
    pub attempt: u32,
    pub criteria: Vec<Criterion>,
    pub limits: ExecutionLimits,
    pub deadline: Instant,
    pub cancel: CancellationToken,
    pub recorder: Recorder,
}

/// Marks a tool as running while alive.
pub struct Running<'a>(&'a AtomicBool);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AttemptState {
    pub fn new(setup: Setup) -> Self {
        let Setup {
            unit,
            attempt,
            criteria,
            limits,
            deadline,
            cancel,
            recorder,
        } = setup;
        Self {
            unit,
            attempt,
            criteria,
            limits,
            deadline,
            cancel,
            recorder,
            model_calls: AtomicU32::new(0),
            tool_calls: AtomicU32::new(0),
            tool_running: AtomicBool::new(false),
            assessment: Mutex::new(None),
            halt: Mutex::new(None),
            problems: Mutex::new(Vec::new()),
            timing: Mutex::new(Clock::default()),
        }
    }

    /// Deadline of one tool operation: the operation limit, capped by the
    /// time left in the attempt.
    pub fn operation_deadline(&self) -> Duration {
        let operation = Duration::from_secs(self.limits.operation_timeout_secs);
        operation.min(self.deadline.saturating_duration_since(Instant::now()))
    }

    /// Counts a model request; `Err` once the per-step limit is exceeded.
    pub fn count_model_call(&self) -> Result<(), String> {
        let calls = self.model_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let limit = self.limits.model_calls_per_step;
        if calls > limit {
            return Err(self.limit(format!(
                "the attempt used all {limit} model requests allowed per step (limits.model_calls_per_step)"
            )));
        }
        Ok(())
    }

    /// Counts a tool request, valid or not; `Err` once the limit is exceeded.
    pub fn count_tool_call(&self) -> Result<(), String> {
        let calls = self.tool_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let limit = self.limits.tool_calls_per_step;
        if calls > limit {
            return Err(self.limit(format!(
                "the attempt used all {limit} tool calls allowed per step (limits.tool_calls_per_step)"
            )));
        }
        Ok(())
    }

    /// Model requests made so far.
    pub fn model_calls(&self) -> u32 {
        self.model_calls.load(Ordering::SeqCst)
    }

    /// `Err` with a stop reason once the attempt was cancelled, timed out or
    /// halted.
    pub fn check_interrupted(&self) -> Result<(), String> {
        if let Some(halt) = lock(&self.halt).as_ref() {
            return Err(halt.message.clone());
        }
        if self.cancel.is_cancelled() || Instant::now() >= self.deadline {
            return Err("the attempt was interrupted".to_owned());
        }
        Ok(())
    }

    fn limit(&self, message: String) -> String {
        self.halt(ExecutionError::new(ErrorKind::LimitExceeded, message))
    }

    /// Halts the attempt; the first reason wins. Returns the stop reason.
    pub fn halt(&self, error: ExecutionError) -> String {
        lock(&self.halt).get_or_insert(error).message.clone()
    }

    /// The halt reason, if any.
    pub fn halted(&self) -> Option<ExecutionError> {
        lock(&self.halt).clone()
    }

    /// Accepts the validated assessment.
    pub fn accept(&self, assessment: Assessment) {
        *lock(&self.assessment) = Some(assessment);
    }

    /// The accepted assessment, if any.
    pub fn assessment(&self) -> Option<Assessment> {
        lock(&self.assessment).clone()
    }

    /// Remembers why the latest submitted assessment was refused.
    pub fn refuse(&self, problems: Vec<String>) {
        *lock(&self.problems) = problems;
    }

    /// Why the latest submitted assessment was refused.
    pub fn problems(&self) -> Vec<String> {
        lock(&self.problems).clone()
    }

    /// Marks a tool as running until the guard is dropped.
    pub fn running(&self) -> Running<'_> {
        self.tool_running.store(true, Ordering::SeqCst);
        Running(&self.tool_running)
    }

    /// Whether a tool is running now.
    pub fn tool_running(&self) -> bool {
        self.tool_running.load(Ordering::SeqCst)
    }

    /// Records an observation of this attempt.
    pub fn record(&self, duration: Duration, record: EvidenceRecord) -> Evidence {
        lock(&self.timing).tool += duration;
        self.recorder
            .record(&self.unit, self.attempt, duration, record)
    }

    /// Notes that a model request was sent.
    pub fn model_sent(&self) {
        let mut clock = lock(&self.timing);
        clock.settle_model();
        clock.model_requests += 1;
        clock.model_since = Some(Instant::now());
    }

    /// Notes that the model request in flight was answered.
    pub fn model_answered(&self) {
        lock(&self.timing).settle_model();
    }

    /// Adds the token usage the provider reported for a model response.
    pub fn model_usage(&self, usage: &Usage) {
        let mut clock = lock(&self.timing);
        clock.input_tokens += usage.input_tokens.unwrap_or_default();
        clock.output_tokens += usage.output_tokens.unwrap_or_default();
        if let Some(cached) = usage.cached_input_tokens {
            *clock.cached_tokens.get_or_insert(0) += cached;
        }
    }

    /// Where the attempt spent its time. A request still in flight (one that
    /// failed or was interrupted) counts until now.
    pub fn timing(&self) -> AttemptTiming {
        let mut clock = lock(&self.timing);
        clock.settle_model();
        AttemptTiming {
            model_requests: clock.model_requests,
            model_ms: millis(clock.model),
            tool_ms: millis(clock.tool),
            input_tokens: clock.input_tokens,
            cached_tokens: clock.cached_tokens,
            output_tokens: clock.output_tokens,
        }
    }

    /// Records a tool request that was refused before it ran.
    pub fn reject(&self, name: &str, arguments: &str, reason: &str) -> Evidence {
        let record = EvidenceRecord::Rejected(RejectedCall {
            name: name.to_owned(),
            arguments: arguments.to_owned(),
            reason: reason.to_owned(),
        });
        self.record(Duration::ZERO, record)
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
