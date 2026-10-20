//! The Rig hook that enforces Saci's controls at every model and tool
//! boundary of an attempt.
//!
//! Every model request and tool call, including calls Rig rejects as
//! unknown or malformed, is counted before it runs, so neither invalid
//! requests nor several calls in one model reply can exceed the per-step
//! limits. A halted, cancelled or timed-out attempt stops at the next
//! boundary, and an accepted assessment ends the run without another model
//! request.

use std::sync::Arc;

use rig::agent::{
    AgentHook, DispatchAction, DispatchEvent, HookContext, ModelTurnAction, ModelTurnFinished,
    OutcomeAction, OutcomeEvent,
};
use rig::completion::AssistantContent;
use rig::run::{InvalidToolCallAction, InvalidToolCallContext, InvalidToolCallReason};

use super::state::AttemptState;

/// Stop reason once the assessment is accepted.
pub const ASSESSED: &str = "assessment accepted";

/// Feedback for a reply that neither calls a tool nor assesses the step.
const ASSESS_FEEDBACK: &str = "The step is not finished until you call the `assess` tool with \
    a result for every expected result. Continue the authored action if needed, then call `assess`.";

/// Saci's controls, attached to one attempt's agent run.
pub struct Controls {
    state: Arc<AttemptState>,
}

impl Controls {
    pub const fn new(state: Arc<AttemptState>) -> Self {
        Self { state }
    }
}

impl AgentHook for Controls {
    async fn on_dispatch(&self, _ctx: &HookContext, event: DispatchEvent<'_>) -> DispatchAction {
        if let Err(reason) = self.state.check_interrupted() {
            return DispatchAction::stop(reason);
        }
        let counted = if event.completion_request().is_some() {
            let counted = self.state.count_model_call();
            if counted.is_ok() {
                self.state.model_sent();
            }
            counted
        } else if event.tool_name().is_some() {
            // A model turn Rig recovered without an outcome ends here.
            self.state.model_answered();
            self.state.count_tool_call()
        } else {
            Ok(())
        };
        counted.map_or_else(DispatchAction::stop, |()| DispatchAction::proceed())
    }

    async fn on_outcome(&self, _ctx: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        if event.completion().is_some() {
            self.state.model_answered();
        }
        if event.tool_result().is_none() {
            return OutcomeAction::proceed();
        }
        if self.state.assessment().is_some() {
            return OutcomeAction::stop(ASSESSED);
        }
        self.state
            .check_interrupted()
            .map_or_else(OutcomeAction::stop, |()| OutcomeAction::proceed())
    }

    async fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let calls_tool = event
            .content
            .iter()
            .any(|content| matches!(content, AssistantContent::ToolCall(_)));
        if calls_tool || self.state.assessment().is_some() {
            ModelTurnAction::continue_run()
        } else {
            ModelTurnAction::retry_with_feedback(ASSESS_FEEDBACK)
        }
    }

    async fn on_invalid_tool_call(
        &self,
        _ctx: &HookContext,
        event: &InvalidToolCallContext,
    ) -> Option<InvalidToolCallAction> {
        let reason = match &event.reason {
            InvalidToolCallReason::MalformedArguments { error } => format!(
                "the arguments for `{}` are not a JSON object ({error}); call it again with a JSON object",
                event.tool_name
            ),
            _ => format!(
                "there is no tool `{}`; the available tools are {}",
                event.tool_name,
                event.available_tools.join(", ")
            ),
        };
        let arguments = event.args.clone().unwrap_or_default();
        self.state.reject(&event.tool_name, &arguments, &reason);
        let action = match self.state.count_tool_call() {
            Ok(()) => InvalidToolCallAction::Skip { reason },
            Err(stop) => InvalidToolCallAction::Stop { reason: stop },
        };
        Some(action)
    }
}
