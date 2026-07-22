use std::collections::HashSet;

use ai::agent::action_result::{AIAgentActionResultType, RequestComputerUseResult};
use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{AIAgentActionId, AIAgentActionType};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::features::FeatureFlag;
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;

pub struct RequestComputerUseExecutor {
    terminal_view_id: EntityId,
    ambient_agent_task_id: Option<AmbientAgentTaskId>,
    /// Actions that were determined to be auto-executed in should_autoexecute().
    /// Used to determine is_autoexecuted when emitting telemetry in execute().
    autoexecuted_actions: HashSet<AIAgentActionId>,
}

impl RequestComputerUseExecutor {
    pub fn new(terminal_view_id: EntityId) -> Self {
        Self {
            terminal_view_id,
            ambient_agent_task_id: None,
            autoexecuted_actions: HashSet::new(),
        }
    }

    pub fn set_ambient_agent_task_id(&mut self, id: Option<AmbientAgentTaskId>) {
        self.ambient_agent_task_id = id;
    }

    pub(super) fn should_autoexecute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentActionType::RequestComputerUse(_) = &action.action else {
            return false;
        };

        // Check profile permission
        let permission = crate::ai::blocklist::BlocklistAIPermissions::as_ref(ctx)
            .get_computer_use_setting(ctx, Some(self.terminal_view_id));
        if permission.is_always_allow() {
            // Track that this action was auto-executed for telemetry in execute()
            self.autoexecuted_actions.insert(action.id.clone());
            return true;
        }

        // Otherwise require user confirmation for computer use.
        false
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
        let AIAgentActionType::RequestComputerUse(request) = &action.action else {
            return ActionExecution::InvalidAction;
        };

        // If we're executing, that implies that computer use has been approved.
        let is_autoexecuted = self.autoexecuted_actions.remove(&action.id);
        let server_conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.server_conversation_token())
            .map(|t| t.as_str().to_string());
        send_telemetry_from_ctx!(
            TelemetryEvent::ComputerUseApproved {
                client_conversation_id: conversation_id,
                server_conversation_id,
                is_autoexecuted,
                ambient_agent_task_id: self.ambient_agent_task_id,
            },
            ctx
        );

        let screenshot_params = request.screenshot_params;
        // Build the actor here, in the synchronous (main-thread) body of `execute()`, before moving
        // it into the async future below. On macOS this constructs the keycode cache via Carbon
        // Text Input Source APIs that must run on the main thread; keep it out of the spawned
        // future (which runs on a background executor thread) to avoid a libdispatch main-thread
        // assertion. See also `use_computer.rs`.
        let mut actor = computer_use::create_actor();
        let platform = actor.platform();
        // Gate per-window targeting behind the client feature flag. When off, the actor forces the
        // legacy full-screen path so results are identical to the pre-existing implementation. The
        // OS-capability check is folded into the request setting rather than reported in the result.
        let background_enabled = FeatureFlag::BackgroundComputerUse.is_enabled();
        ActionExecution::Async {
            execute_future: Box::pin(async move {
                let result = actor
                    .perform_actions(
                        &[],
                        computer_use::Options {
                            screenshot_params,
                            background_enabled,
                        },
                    )
                    .await;
                (result, platform)
            }),
            on_complete: Box::new(|action_result, _ctx| match action_result {
                (
                    Ok(computer_use::ActionResult {
                        screenshot: Some(screenshot),
                        windows,
                        ..
                    }),
                    Some(platform),
                ) => AIAgentActionResultType::RequestComputerUse(
                    RequestComputerUseResult::Approved {
                        screenshot,
                        platform,
                        windows,
                    },
                ),
                (
                    Ok(computer_use::ActionResult {
                        screenshot: Some(_),
                        ..
                    }),
                    None,
                ) => AIAgentActionResultType::RequestComputerUse(RequestComputerUseResult::Error(
                    "Unknown platform".to_string(),
                )),
                (Ok(_), _) => {
                    AIAgentActionResultType::RequestComputerUse(RequestComputerUseResult::Error(
                        "Failed to capture initial screenshot".to_string(),
                    ))
                }
                (Err(err), _) => AIAgentActionResultType::RequestComputerUse(
                    RequestComputerUseResult::Error(err),
                ),
            }),
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for RequestComputerUseExecutor {
    type Event = ();
}
