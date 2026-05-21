use crate::ai::agent::conversation::AIConversation;
use crate::ai::agent::conversation_yaml;
use crate::ai::agent::AIAgentActionResultType;
use crate::ai::blocklist::history_model::CloudConversationData;
use ai::agent::action_result::FetchConversationResult;
use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::AIAgentActionType;
use crate::BlocklistAIHistoryModel;

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};

pub struct FetchConversationExecutor;

impl FetchConversationExecutor {
    pub fn new() -> Self {
        Self
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        true
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentActionType::FetchConversation { conversation_id } = &action.action else {
            return ActionExecution::<Option<CloudConversationData>>::InvalidAction;
        };

        let conversation_id = conversation_id.clone();
        let server_token = ServerConversationToken::new(conversation_id.clone());

        let load_future = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.load_conversation_by_server_token(&server_token, ctx)
        });

        ActionExecution::new_async(load_future, move |cloud_conversation, _ctx| {
            // TODO(REMOTE-1203): FetchConversation can't materialize non-Oz conversation transcripts yet.
            let conversation = cloud_conversation.and_then(|cc| match cc {
                CloudConversationData::Oz(c) => Some(c),
                CloudConversationData::CLIAgent(_) => {
                    log::warn!("FetchConversation does not support CLI agent conversations");
                    None
                }
            });
            materialize_conversation(conversation.map(|c| *c), &conversation_id)
        })
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

/// Materializes a loaded conversation's tasks into YAML files on disk.
fn materialize_conversation(
    conversation: Option<AIConversation>,
    server_conversation_id: &str,
) -> AIAgentActionResultType {
    let Some(conversation) = conversation else {
        log::warn!("FetchConversation: failed to load conversation {server_conversation_id}");
        return AIAgentActionResultType::FetchConversation(FetchConversationResult::Error(
            format!("Failed to load conversation {server_conversation_id}"),
        ));
    };

    let tasks: Vec<warp_multi_agent_api::Task> = conversation
        .all_tasks()
        .filter_map(|task| task.source().cloned())
        .collect();
    log::info!(
        "FetchConversation: materializing {} tasks for conversation {server_conversation_id}",
        tasks.len(),
    );
    match conversation_yaml::materialize_tasks_to_yaml(&tasks) {
        Ok(directory_path) => {
            log::info!(
                "FetchConversation: wrote YAML to {directory_path} \
                 for conversation {server_conversation_id}"
            );
            AIAgentActionResultType::FetchConversation(FetchConversationResult::Success {
                directory_path,
            })
        }
        Err(e) => {
            log::error!("FetchConversation: failed to materialize YAML: {e}");
            AIAgentActionResultType::FetchConversation(FetchConversationResult::Error(format!(
                "Failed to materialize conversation: {e}"
            )))
        }
    }
}

impl Entity for FetchConversationExecutor {
    type Event = ();
}
