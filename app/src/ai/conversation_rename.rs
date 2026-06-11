use warpui::{SingletonEntity, View, ViewContext};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::history_model::CloudConversationData;
use crate::ai::blocklist::{BeginConversationRenameError, BlocklistAIHistoryModel};
use crate::server::server_api::ServerApiProvider;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

pub(crate) const CONVERSATION_TITLE_MAX_CHARS: usize = 500;

/// Renames a conversation locally and triggers a conversation rename on the server.
pub(crate) fn rename_conversation<T: View>(
    conversation_id: AIConversationId,
    title: String,
    conversation_not_found_message: &'static str,
    ctx: &mut ViewContext<T>,
) -> bool {
    let title = match validate_conversation_title(title) {
        Ok(title) => title,
        Err(message) => {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(DismissibleToast::error(message), window_id, ctx);
            });
            return true;
        }
    };
    if conversation_already_has_title(conversation_id, &title, ctx) {
        return true;
    }

    // `load_conversation_data` resolves immediately when the conversation is already in
    // memory, so loaded and unloaded conversations share this single load-then-rename path.
    let history = BlocklistAIHistoryModel::handle(ctx);
    let future = history
        .as_ref(ctx)
        .load_conversation_data(conversation_id, ctx);
    ctx.spawn(future, move |_, conversation, ctx| {
        match conversation {
            Some(CloudConversationData::Oz(conversation)) => {
                history.update(ctx, |history, _| {
                    // The load resolves with a clone when the conversation is already in
                    // memory; re-registering it would overwrite newer in-memory state.
                    if history.conversation(&conversation_id).is_none() {
                        history.register_loaded_conversation(*conversation);
                    }
                });
            }
            Some(CloudConversationData::CLIAgent(_)) => {
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(
                            "Conversations created by CLI agents like Claude Code can't be renamed"
                                .to_owned(),
                        ),
                        window_id,
                        ctx,
                    );
                });
                return;
            }
            None => {
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(
                            "Failed to load conversation for renaming".to_owned(),
                        ),
                        window_id,
                        ctx,
                    );
                });
                return;
            }
        }
        if conversation_already_has_title(conversation_id, &title, ctx) {
            return;
        }
        begin_conversation_rename(conversation_id, title, conversation_not_found_message, ctx);
    });

    true
}

/// Returns whether the conversation's current local title already matches `title`,
/// making the rename a no-op.
fn conversation_already_has_title<T: View>(
    conversation_id: AIConversationId,
    title: &str,
    ctx: &ViewContext<T>,
) -> bool {
    let history = BlocklistAIHistoryModel::as_ref(ctx);
    history
        .conversation(&conversation_id)
        .and_then(|conversation| conversation.title())
        .or_else(|| {
            history
                .get_conversation_metadata(&conversation_id)
                .map(|metadata| metadata.title.clone())
        })
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
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(
                    DismissibleToast::error(message.to_owned()),
                    window_id,
                    ctx,
                );
            });
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
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(format!("Failed to rename conversation: {e}")),
                            window_id,
                            ctx,
                        );
                    });
                }
            }
        },
    );
}
