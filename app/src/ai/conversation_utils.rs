use anyhow::Context as _;
use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warpui::{AppContext, EntityId, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::server::server_api::ServerApiProvider;

/// Delete a conversation from the blocklist, local storage, and the cloud.
pub fn delete_conversation(
    conversation_id: AIConversationId,
    terminal_view_id: Option<EntityId>,
    ctx: &mut AppContext,
) {
    let server_conversation_token = get_server_conversation_token(&conversation_id, ctx);
    let server_api = ServerApiProvider::as_ref(ctx).get_ai_client();

    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, model_ctx| {
        history.delete_conversation(conversation_id, terminal_view_id, model_ctx);

        if let Some(token) = server_conversation_token {
            if FeatureFlag::CloudConversations.is_enabled() {
                // Delete the conversation from the cloud.
                let server_api = server_api.clone();
                model_ctx.spawn(
                    async move {
                        match server_api
                            .delete_ai_conversation(token.clone())
                            .await
                            .context("Failed to delete conversation from cloud")
                        {
                            Err(e) => {
                                report_error!(e);
                            }
                            _ => {
                                log::info!("Successfully deleted conversation from cloud: {token}");
                            }
                        }
                    },
                    |_, _, _| {},
                );
            }
        } else {
            log::info!(
                "No server conversation token found for conversation to delete: {conversation_id}"
            );
        }
    });

    // Make sure the agent conversations model is up to date.
    AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
        model.sync_conversations(ctx);
    });
}

/// Remove a conversation from the blocklist and optionally from the cloud.
///
/// This is similar to `delete_conversation`, but it is only used for ephemeral/empty conversations
/// (i.e. conversations from closed empty agent mode views, or passive code diffs).
pub fn remove_conversation(
    conversation_id: AIConversationId,
    terminal_view_id: EntityId,
    // Set this to true if the conversation has some exchanges (i.e. is not empty),
    // and should thus also be cleaned up from the cloud
    delete_from_cloud: bool,
    ctx: &mut AppContext,
) {
    let (server_conversation_token, server_api) = if delete_from_cloud {
        (
            get_server_conversation_token(&conversation_id, ctx),
            Some(ServerApiProvider::as_ref(ctx).get_ai_client()),
        )
    } else {
        (None, None)
    };

    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, model_ctx| {
        history.remove_conversation(conversation_id, terminal_view_id, model_ctx);

        if let (Some(token), Some(server_api)) = (server_conversation_token, server_api)
            && FeatureFlag::CloudConversations.is_enabled() {
                // Delete the conversation from the cloud.
                model_ctx.spawn(
                    async move {
                        if let Err(e) = server_api.delete_ai_conversation(token).await {
                            log::warn!("Failed to delete conversation from cloud during remove_conversation: {e:?}");
                        }
                    },
                    |_, _, _| {},
                );
            }
    });
}

/// Get the server conversation token for a historical or live conversation.
fn get_server_conversation_token(
    conversation_id: &AIConversationId,
    ctx: &AppContext,
) -> Option<String> {
    let history = BlocklistAIHistoryModel::as_ref(ctx);

    history
        .conversation(conversation_id)
        .and_then(|c| {
            c.server_conversation_token()
                .map(|t| t.as_str().to_string())
        })
        .or_else(|| {
            history
                .get_conversation_metadata(conversation_id)
                .and_then(|m| m.server_conversation_token.as_ref())
                .map(|t| t.as_str().to_string())
        })
}
