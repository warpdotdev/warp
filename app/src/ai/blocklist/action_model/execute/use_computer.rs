use ai::agent::action_result::AIAgentActionResultType;
use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{AIAgentActionType, UseComputerResult};
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
use crate::features::FeatureFlag;

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
    ) -> impl Into<AnyActionExecution> + use<> {
        let ExecuteActionInput {
            action,
            conversation_id,
        } = input;
        let AIAgentActionType::UseComputer(request) = &action.action else {
            return ActionExecution::InvalidAction;
        };

        let labels = computer_use::overlay_labels_for(&request.actions, &request.action_summary);
        let meaningful = computer_use::is_meaningful_action_group(&request.actions);
        // Reserve a group start (pending) for meaningful calls so the recording's
        // post-stop smart cut keeps the call's real action window at 1x and drops
        // only blocked/thinking gaps. A pointer-only group commits with empty
        // labels; a wait-only/no-op batch (e.g. a screenshot-only `Wait(0)`) is
        // not meaningful and is ignored. The finish offset is captured from the
        // returned capture start instant when the actor future returns.
        let recording_started_at = if meaningful {
            RecordingController::handle(ctx).update(ctx, |controller, _| {
                controller.begin_action_group(conversation_id, labels)
            })
        } else {
            None
        };

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
        // Tag this session's background-window activations with the owning conversation so its
        // teardown (on completion or cancellation) only tears down this conversation's windows.
        actor.set_background_session_owner(Some(conversation_id.to_string()));
        ActionExecution::new_async(
            async move {
                let result = match actor
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
                };
                // Capture the finish offset immediately after the complete
                // sequential batch (including any explicit waits and the
                // post-action screenshot) returns, measured against the
                // recording's capture start instant.
                let finish_offset = recording_started_at
                    .and_then(|started| instant::Instant::now().checked_duration_since(started));
                (result, finish_offset)
            },
            move |(result, finish_offset), ctx| {
                if meaningful {
                    RecordingController::handle(ctx).update(ctx, |controller, _| match result {
                        UseComputerResult::Success(_) => {
                            if let Some(finish_offset) = finish_offset {
                                controller.commit_action_group(conversation_id, finish_offset);
                            } else {
                                controller.discard_action_group(conversation_id);
                            }
                        }
                        UseComputerResult::Error(_) | UseComputerResult::Cancelled => {
                            controller.discard_action_group(conversation_id);
                        }
                    });
                }
                AIAgentActionResultType::UseComputer(result)
            },
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
