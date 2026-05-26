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
        ActionExecution::new_async(
            async move {
                // Diagnostics for the agent-driven coordinate-conversion investigation. Gated on
                // COMPUTER_USE_DEBUG; routed through `log` so it lands in the app's log file.
                let debug = std::env::var_os("COMPUTER_USE_DEBUG").is_some();
                if debug {
                    log::info!(
                        "[computer_use] executor: {} action(s), screenshot_params={screenshot_params:?}",
                        actions.len(),
                    );
                }
                let mut actor = computer_use::create_actor();
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
                    Ok(result) => {
                        if debug {
                            // The model is shown the `sent` image; the agent's coordinates are
                            // remapped as if they were `native` window pixels. Log both so a
                            // missing downscale-inverse is visible.
                            let screenshot_sent_native = result
                                .screenshot
                                .as_ref()
                                .map(|s| (s.width, s.height, s.original_width, s.original_height));
                            log::info!(
                                "[computer_use] executor result: \
                                 screenshot(sent_wxh,native_wxh)={screenshot_sent_native:?} \
                                 captured_window={:?} windows={}",
                                result.captured_window,
                                result.windows.len(),
                            );
                        }
                        UseComputerResult::Success(result)
                    }
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
