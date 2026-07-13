use ai::agent::action_result::AIAgentActionResultType;
use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, ModelContext};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{AIAgentActionType, UseComputerResult};
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
        _ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentActionType::UseComputer(request) = &action.action else {
            return ActionExecution::InvalidAction;
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
