use warpui::{SingletonEntity, View, ViewContext};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::history_model::CloudConversationData;
use crate::ai::blocklist::{BeginConversationRenameError, BlocklistAIHistoryModel};
use crate::server::server_api::ServerApiProvider;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

pub(crate) const CONVERSATION_TITLE_MAX_CHARS: usize = 500;

/// Renames a conversation using the same optimistic path as `/rename-conversation`.
pub(crate) fn rename_conversation<T: View>(
    conversation_id: AIConversationId,
    title: String,
    conversation_not_found_message: &'static str,
    ctx: &mut ViewContext<T>,
) -> bool {
    let Some(title) = validate_conversation_title(title, ctx) else {
        return true;
    };
    if conversation_title_matches(conversation_id, &title, ctx) {
        return true;
    }

    if BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&conversation_id)
        .is_some()
    {
        begin_loaded_conversation_rename(
            conversation_id,
            title,
            conversation_not_found_message,
            ctx,
        );
        return true;
    }

    let history = BlocklistAIHistoryModel::handle(ctx);
    let future = history
        .as_ref(ctx)
        .load_conversation_data(conversation_id, ctx);
    ctx.spawn(future, move |_, conversation, ctx| match conversation {
        Some(CloudConversationData::Oz(conversation)) => {
            history.update(ctx, |history, _| {
                history.cache_loaded_conversation(*conversation);
            });
            if conversation_title_matches(conversation_id, &title, ctx) {
                return;
            }
            begin_loaded_conversation_rename(
                conversation_id,
                title,
                conversation_not_found_message,
                ctx,
            );
        }
        Some(CloudConversationData::CLIAgent(_)) => {
            show_error_toast(
                "This conversation can't be renamed from this client.".to_string(),
                ctx,
            );
        }
        None => {
            show_error_toast("Failed to load conversation for renaming".to_string(), ctx);
        }
    });

    true
}

/// Returns whether the requested title already matches local conversation state.
fn conversation_title_matches<T: View>(
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

fn validate_conversation_title<T: View>(title: String, ctx: &mut ViewContext<T>) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        show_error_toast(
            "Please provide a title after /rename-conversation".to_owned(),
            ctx,
        );
        return None;
    }

    if title.chars().count() > CONVERSATION_TITLE_MAX_CHARS {
        show_error_toast(
            format!(
                "Conversation title must be {CONVERSATION_TITLE_MAX_CHARS} characters or fewer",
            ),
            ctx,
        );
        return None;
    }

    Some(title.to_owned())
}

fn begin_loaded_conversation_rename<T: View>(
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
            show_begin_error_toast(err, conversation_not_found_message, ctx);
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
        move |_, result, ctx| match result {
            Ok(response) => {
                let title = response.title;
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.complete_conversation_rename(conversation_id, title.clone(), ctx);
                });
                let window_id = ctx.window_id();
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
        },
    );
}

fn show_begin_error_toast<T: View>(
    err: BeginConversationRenameError,
    conversation_not_found_message: &'static str,
    ctx: &mut ViewContext<T>,
) {
    let message = match err {
        BeginConversationRenameError::MissingServerConversationToken => {
            "Your conversation hasn't synced to the cloud yet. Try sending another message, then rename it again."
        }
        BeginConversationRenameError::RenameInProgress => {
            "A rename is already in progress for this conversation"
        }
        BeginConversationRenameError::ConversationNotFound => conversation_not_found_message,
        BeginConversationRenameError::ConversationNotReady => {
            "Your conversation is still syncing. Try renaming it again in a moment."
        }
    };
    show_error_toast(message.to_owned(), ctx);
}

fn show_error_toast<T: View>(message: String, ctx: &mut ViewContext<T>) {
    let window_id = ctx.window_id();
    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
        toast_stack.add_ephemeral_toast(DismissibleToast::error(message), window_id, ctx);
    });
}
