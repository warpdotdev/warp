use ai::agent::action_result::AIAgentActionResultType;
use computer_use::{Action, OverlayKind, TargetedAction};
use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{AIAgentActionType, UseComputerResult};
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
use crate::features::FeatureFlag;

/// Maps a `UseComputer` call to a keyboard overlay entry, or `None` for input
/// that is visible on screen (pointer/scroll) or is a no-op. Only keyboard input
/// is annotated; kind comes from the structured actions and the `Key` label from
/// the server-authored summary (the typed payload is never surfaced).
fn overlay_entry_for(
    actions: &[TargetedAction],
    action_summary: &str,
) -> Option<(OverlayKind, String)> {
    let mut has_key = false;
    for targeted in actions {
        match &targeted.action {
            Action::TypeText { .. } => {
                return Some((OverlayKind::Type, "typing\u{2026}".to_string()))
            }
            Action::KeyDown { .. } | Action::KeyUp { .. } => has_key = true,
            _ => {}
        }
    }
    has_key.then(|| (OverlayKind::Key, key_label_from_summary(action_summary)))
}

/// Extracts the key combo from a `Key "<combo>"` summary, falling back to the
/// trimmed summary (or a generic label) when it isn't quoted.
fn key_label_from_summary(summary: &str) -> String {
    let quoted = summary
        .find('"')
        .zip(summary.rfind('"'))
        .filter(|(first, last)| last > first)
        .map(|(first, last)| summary[first + 1..last].to_string());
    if let Some(label) = quoted {
        return label;
    }
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        "key".to_string()
    } else {
        trimmed.to_string()
    }
}

pub struct UseComputerExecutor;

impl UseComputerExecutor {
    pub fn new() -> Self {
        Self
    }

    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentActionType::UseComputer(_) = &action.action else {
            return false;
        };

        // We unconditionally return true here because this action is only executed by
        // the computer use subagent, which cannot begin without the user approving it via
        // a `RequestComputerUse` action, and the approval extends to all computer use
        // actions within that computer use subagent.
        true
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentActionType::UseComputer(request) = &action.action else {
            return ActionExecution::InvalidAction;
        };

        // Record a keyboard overlay entry for this call if a recording is active.
        // Pointer/scroll and no-op actions are visible on screen and produce none.
        if let Some((kind, label)) = overlay_entry_for(&request.actions, &request.action_summary) {
            RecordingController::handle(ctx).update(ctx, |controller, _| {
                controller.record_action(kind, label);
            });
        }

        let actions = request.actions.clone();
        let screenshot_params = request.screenshot_params;
        // Gate per-window targeting behind the client feature flag. When off, the actor forces the
        // legacy full-screen path so results are identical to the pre-existing implementation.
        let background_enabled = FeatureFlag::BackgroundComputerUse.is_enabled();
        // Build the actor here, in the synchronous (main-thread) body of `execute()`, and move it
        // into the async future below. On macOS, constructing the actor builds the keycode cache,
        // which calls Carbon Text Input Source APIs that assert they run on the main thread; doing
        // it inside the future would run it on a background executor thread and abort with a
        // libdispatch main-thread assertion. This mirrors `request_computer_use.rs`.
        let mut actor = computer_use::create_actor();
        ActionExecution::new_async(
            async move {
                match actor
                    .perform_actions(
                        &actions,
                        computer_use::Options {
                            screenshot_params,
                            background_enabled,
                        },
                    )
                    .await
                {
                    Ok(result) => UseComputerResult::Success(result),
                    Err(error) => UseComputerResult::Error(error),
                }
            },
            |res, _ctx| AIAgentActionResultType::UseComputer(res),
        )
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for UseComputerExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "use_computer_tests.rs"]
mod tests;
