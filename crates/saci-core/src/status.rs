//! Run lifecycle and outcome vocabulary.
//!
//! Lifecycle says where a run is; outcome says how a finished run (or a
//! unit, or an attempt) ended. They are deliberately separate types.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Where a run is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// Validated and stored; no worker exists yet.
    Accepted,
    /// A worker is being launched.
    Starting,
    /// The worker is executing the plan.
    Running,
    /// Cancellation or a deadline asked the worker to stop.
    Stopping,
    /// Terminal. The run has an outcome.
    Finished,
}

impl Lifecycle {
    /// Whether moving from `self` to `next` is a permitted transition.
    ///
    /// Runs only move forward. Any non-terminal state may finish directly
    /// (for example when a launch fails) or begin stopping. `Finished` is
    /// terminal, so duplicate terminal events can never reopen a run.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use Lifecycle::{Accepted, Finished, Running, Starting, Stopping};
        matches!(
            (self, next),
            (Accepted, Starting)
                | (Starting, Running)
                | (Accepted | Starting | Running, Stopping)
                | (Accepted | Starting | Running | Stopping, Finished)
        )
    }

    /// Whether the run has finished.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Finished)
    }
}

impl fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Accepted => "accepted",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Finished => "finished",
        };
        f.write_str(text)
    }
}

/// How a run, unit or attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Every expected criterion is supported by evidence.
    Passed,
    /// Evidence contradicts at least one expected criterion.
    Failed,
    /// A prerequisite or required input prevents execution.
    Blocked,
    /// Execution produced insufficient evidence to establish the result.
    Inconclusive,
    /// The agent, worker, tools, provider or a limit prevented completion.
    Error,
    /// The unit was not attempted; a reason is recorded.
    Skipped,
    /// A user or external controller explicitly stopped execution.
    Cancelled,
}

impl Outcome {
    /// Precedence used when summarizing several outcomes: higher wins.
    ///
    /// Order: cancelled, error, failed, blocked, inconclusive, passed.
    /// `Skipped` never wins by itself; incomplete work is summarized as
    /// `Blocked` by [`summarize`].
    #[must_use]
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Cancelled => 6,
            Self::Error => 5,
            Self::Failed => 4,
            Self::Blocked => 3,
            Self::Inconclusive => 2,
            Self::Passed => 1,
            Self::Skipped => 0,
        }
    }

    /// Whether this outcome counts as unsuccessful for run policy.
    #[must_use]
    pub const fn is_unsuccessful(self) -> bool {
        matches!(
            self,
            Self::Failed | Self::Blocked | Self::Inconclusive | Self::Error
        )
    }

    /// Returns the outcome's wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Inconclusive => "inconclusive",
            Self::Error => "error",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Summarizes unit outcomes into a run outcome.
///
/// The highest-precedence outcome wins. A run where any unit was skipped, or
/// with no units at all, cannot pass: it is at least `Blocked`.
#[must_use]
pub fn summarize(outcomes: impl IntoIterator<Item = Outcome>) -> Outcome {
    let mut summary = Outcome::Skipped;
    let mut any = false;
    for outcome in outcomes {
        any = true;
        let effective = if outcome == Outcome::Skipped {
            Outcome::Blocked
        } else {
            outcome
        };
        if effective.precedence() > summary.precedence() {
            summary = effective;
        }
    }
    if any { summary } else { Outcome::Blocked }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_moves_forward_only() {
        assert!(Lifecycle::Accepted.can_transition_to(Lifecycle::Starting));
        assert!(Lifecycle::Running.can_transition_to(Lifecycle::Stopping));
        assert!(Lifecycle::Stopping.can_transition_to(Lifecycle::Finished));
        assert!(Lifecycle::Accepted.can_transition_to(Lifecycle::Finished));
        assert!(!Lifecycle::Running.can_transition_to(Lifecycle::Starting));
        assert!(!Lifecycle::Finished.can_transition_to(Lifecycle::Finished));
        assert!(!Lifecycle::Finished.can_transition_to(Lifecycle::Running));
    }

    #[test]
    fn summary_follows_documented_precedence() {
        use Outcome::{Blocked, Cancelled, Error, Failed, Inconclusive, Passed};
        assert_eq!(summarize([Passed, Passed]), Passed);
        assert_eq!(summarize([Passed, Inconclusive]), Inconclusive);
        assert_eq!(summarize([Inconclusive, Blocked]), Blocked);
        assert_eq!(summarize([Blocked, Failed]), Failed);
        assert_eq!(summarize([Failed, Error]), Error);
        assert_eq!(summarize([Error, Cancelled, Passed]), Cancelled);
    }

    #[test]
    fn skipped_work_prevents_a_pass() {
        assert_eq!(
            summarize([Outcome::Passed, Outcome::Skipped]),
            Outcome::Blocked
        );
        assert_eq!(
            summarize([Outcome::Failed, Outcome::Skipped]),
            Outcome::Failed
        );
        assert_eq!(summarize([]), Outcome::Blocked);
    }

    #[test]
    fn outcomes_use_snake_case_on_the_wire() {
        let json = serde_json::to_string(&Outcome::Inconclusive).unwrap();
        assert_eq!(json, "\"inconclusive\"");
    }
}
