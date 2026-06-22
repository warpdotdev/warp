use ai::agent::action_result::AIAgentActionResultType;
use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, ModelContext};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{AIAgentActionType, UseComputerResult};

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
        ActionExecution::new_async(
            async move {
                let mut actor = computer_use::create_actor();
                match actor
                    .perform_actions(&actions, computer_use::Options { screenshot_params })
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
