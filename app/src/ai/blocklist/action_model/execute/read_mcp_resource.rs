use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
#[cfg(not(target_family = "wasm"))]
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::terminal::model::session::active_session::ActiveSession;
use futures::{future::BoxFuture, FutureExt};
use warpui::{Entity, EntityId, ModelContext, ModelHandle};

#[cfg(not(target_family = "wasm"))]
use crate::ai::{
    agent::{AIAgentActionResultType, ReadMCPResourceResult},
    blocklist::{
        action_model::{AIAgentAction, AIAgentActionType},
        BlocklistAIPermissions,
    },
};
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;

pub struct ReadMCPResourceExecutor {
    _active_session: ModelHandle<ActiveSession>,
    #[cfg_attr(target_family = "wasm", expect(unused))]
    terminal_view_id: EntityId,
}

impl ReadMCPResourceExecutor {
    pub fn new(_active_session: ModelHandle<ActiveSession>, terminal_view_id: EntityId) -> Self {
        Self {
            _active_session,
            terminal_view_id,
        }
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables), allow(dead_code))]
    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        #[cfg(target_family = "wasm")]
        {
            false
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let ExecuteActionInput {
                action:
                    AIAgentAction {
                        action:
                            AIAgentActionType::ReadMCPResource {
                                server_id,
                                name,
                                uri,
                                ..
                            },
                        ..
                    },
                conversation_id,
            } = input
            else {
                return false;
            };

            BlocklistAIPermissions::as_ref(ctx).can_read_mcp_resource(
                server_id.as_ref(),
                name.as_str(),
                uri.as_deref(),
                &conversation_id,
                Some(self.terminal_view_id),
                ctx,
            )
        }
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        #[cfg(target_family = "wasm")]
        {
            ActionExecution::<()>::InvalidAction
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let ExecuteActionInput { action, .. } = input;
            let AIAgentAction {
                action:
                    AIAgentActionType::ReadMCPResource {
                        server_id: _,
                        name,
                        uri,
                    },
                ..
            } = action
            else {
                return ActionExecution::InvalidAction;
            };

            let templatable_mcp_client = TemplatableMCPServerManager::as_ref(ctx);

            let resource = match uri {
                Some(uri) => templatable_mcp_client
                    .resources()
                    .find(|resource| &resource.uri == uri),
                None => templatable_mcp_client
                    .resources()
                    .find(|resource| &resource.name == name),
            };

            let Some(resource) = resource else {
                return ActionExecution::Sync(AIAgentActionResultType::ReadMCPResource(
                    ReadMCPResourceResult::Error("MCP server resource not found".to_owned()),
                ));
            };

            let uri = resource.uri.clone();

            let Some(reconnecting_peer) = templatable_mcp_client.server_with_resource(resource)
            else {
                return ActionExecution::Sync(AIAgentActionResultType::ReadMCPResource(
                    ReadMCPResourceResult::Error("MCP server for resource not found".to_owned()),
                ));
            };

            ActionExecution::new_async(
                async move {
                    reconnecting_peer
                        .read_resource(rmcp::model::ReadResourceRequestParams::new(uri))
                        .await
                },
                |res, _ctx| handle_read_resource_result(res),
            )
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for ReadMCPResourceExecutor {
    type Event = ();
}

/// Handles the result of a read_resource request, converting it to an AIAgentActionResultType.
#[cfg(not(target_family = "wasm"))]
fn handle_read_resource_result(
    res: Result<rmcp::model::ReadResourceResult, rmcp::ServiceError>,
) -> AIAgentActionResultType {
    let action_result = match res {
        Ok(response) => ReadMCPResourceResult::Success {
            resource_contents: response.contents,
        },
        Err(e) => ReadMCPResourceResult::Error(e.to_string()),
    };
    AIAgentActionResultType::ReadMCPResource(action_result)
}
