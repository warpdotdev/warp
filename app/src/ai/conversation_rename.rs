use warpui::{SingletonEntity, View, ViewContext};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{BeginConversationRenameError, BlocklistAIHistoryModel};
use crate::server::server_api::ServerApiProvider;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

pub(crate) const CONVERSATION_TITLE_MAX_CHARS: usize = 500;

/// Renames a conversation locally and triggers a conversation rename on the server.
///
/// Renaming is only exposed for open conversations, so the conversation is expected
/// to already be loaded in the history model.
pub(crate) fn rename_conversation<T: View>(
    conversation_id: AIConversationId,
    title: String,
    conversation_not_found_message: &'static str,
    ctx: &mut ViewContext<T>,
) {
    let title = match validate_conversation_title(title) {
        Ok(title) => title,
        Err(message) => {
            show_error_toast(message, ctx);
            return;
        }
    };
    if conversation_already_has_title(conversation_id, &title, ctx) {
        return;
    }
    begin_conversation_rename(conversation_id, title, conversation_not_found_message, ctx);
}

/// Returns whether the conversation's current local title already matches `title`,
/// making the rename a no-op.
fn conversation_already_has_title<T: View>(
    conversation_id: AIConversationId,
    title: &str,
    ctx: &ViewContext<T>,
) -> bool {
    BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&conversation_id)
        .and_then(|conversation| conversation.title())
        .is_some_and(|current_title| current_title == title)
}

/// Trims and validates a requested conversation title, returning a user-facing
/// error message when the title is invalid.
fn validate_conversation_title(title: String) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Please provide a title after /rename-conversation".to_owned());
    }

    if title.chars().count() > CONVERSATION_TITLE_MAX_CHARS {
        return Err(format!(
            "Conversation title must be {CONVERSATION_TITLE_MAX_CHARS} characters or fewer",
        ));
    }

    Ok(title.to_owned())
}

/// Starts an optimistic rename for a loaded conversation and syncs it to the server.
fn begin_conversation_rename<T: View>(
    conversation_id: AIConversationId,
    title: String,
    conversation_not_found_message: &'static str,
    ctx: &mut ViewContext<T>,
) {
    let history = BlocklistAIHistoryModel::handle(ctx);
    let server_conversation_id = match history.update(ctx, |history, ctx| {
        history.begin_conversation_rename(conversation_id, title.clone(), ctx)
    }) {
        Ok(server_conversation_id) => server_conversation_id,
        Err(err) => {
            let message = match err {
                BeginConversationRenameError::MissingServerConversationToken => {
                    "Your conversation hasn't synced to the cloud yet. Try sending another message, then rename it again."
                }
                BeginConversationRenameError::RenameInProgress => {
                    "A rename is already in progress for this conversation"
                }
                BeginConversationRenameError::ConversationNotFound => {
                    conversation_not_found_message
                }
                BeginConversationRenameError::ConversationNotReady => {
                    "Your conversation is still syncing. Try renaming it again in a moment."
                }
            };
            show_error_toast(message.to_owned(), ctx);
            return;
        }
    };

    let server_api = ServerApiProvider::as_ref(ctx).get_ai_client();
    ctx.spawn(
        async move {
            server_api
                .rename_conversation(server_conversation_id, title)
                .await
        },
        move |_, result, ctx| {
            let window_id = ctx.window_id();
            match result {
                Ok(response) => {
                    let title = response.title;
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        history.complete_conversation_rename(conversation_id, title.clone(), ctx);
                    });
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::success(format!("Conversation renamed to {title}")),
                            window_id,
                            ctx,
                        );
                    });
                }
                Err(e) => {
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        history.fail_conversation_rename(conversation_id, ctx);
                    });
                    show_error_toast(format!("Failed to rename conversation: {e}"), ctx);
                }
            }
        },
    );
}

/// Shows an ephemeral error toast in the current window.
fn show_error_toast<T: View>(message: String, ctx: &mut ViewContext<T>) {
    let window_id = ctx.window_id();
    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
        toast_stack.add_ephemeral_toast(DismissibleToast::error(message), window_id, ctx);
    });
}
