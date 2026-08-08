use warpui::{SingletonEntity, View, ViewContext};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{BeginConversationRenameError, BlocklistAIHistoryModel};
use crate::server::server_api::ServerApiProvider;
use crate::view_components::DismissibleToast;
use crate::workspace::{ToastStack, WorkspaceAction};

const CONVERSATION_TITLE_MAX_CHARS: usize = 500;

const EMPTY_TITLE_MESSAGE: &str = "Please provide a conversation title";
const EMPTY_CONVERSATION_MESSAGE: &str = "You can't rename an empty conversation";
const CONVERSATION_NOT_FOUND_MESSAGE: &str = "Conversation not found";
const NOT_SYNCED_MESSAGE: &str = "Your conversation hasn't synced to the cloud yet. Try sending another message, then rename it again.";
const RENAME_IN_PROGRESS_MESSAGE: &str = "A rename is already in progress for this conversation";
const CONVERSATION_NOT_READY_MESSAGE: &str =
    "Your conversation is still syncing. Try renaming it again in a moment.";

/// Whether a rename reports its outcome to the user.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RenameFeedback {
    /// The user asked for this rename, so every outcome is worth a toast.
    Reported,
    /// The rename is a side effect of another action, so a conversation that cannot be
    /// renamed is not a failure the user needs to hear about.
    Quiet,
}

/// Renames a conversation locally and triggers a conversation rename on the server.
///
/// Renaming is only exposed for open conversations, so the conversation is expected
/// to already be loaded in the history model.
pub(crate) fn rename_conversation<T: View>(
    conversation_id: AIConversationId,
    title: String,
    ctx: &mut ViewContext<T>,
) {
    rename_conversation_with_feedback(conversation_id, title, RenameFeedback::Reported, ctx);
}

/// Renames a conversation the user renamed indirectly — e.g. by renaming the pane hosting
/// it — without surfacing any toast. A conversation that cannot be renamed (not yet synced,
/// empty, still loading) leaves the triggering action intact and reports nothing.
pub(crate) fn rename_conversation_quietly<T: View>(
    conversation_id: AIConversationId,
    title: String,
    ctx: &mut ViewContext<T>,
) {
    rename_conversation_with_feedback(conversation_id, title, RenameFeedback::Quiet, ctx);
}

fn rename_conversation_with_feedback<T: View>(
    conversation_id: AIConversationId,
    title: String,
    feedback: RenameFeedback,
    ctx: &mut ViewContext<T>,
) {
    let title = match validate_conversation_title(title) {
        Ok(title) => title,
        Err(message) => {
            show_toast(feedback, DismissibleToast::error(message), ctx);
            return;
        }
    };
    if BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&conversation_id)
        .is_some_and(|conversation| conversation.is_empty())
    {
        show_toast(
            feedback,
            DismissibleToast::error(EMPTY_CONVERSATION_MESSAGE.to_owned()),
            ctx,
        );
        return;
    }
    if conversation_already_has_title(conversation_id, &title, ctx) {
        return;
    }

    let history = BlocklistAIHistoryModel::handle(ctx);
    let begin_result = history.update(ctx, |history, ctx| {
        history.begin_conversation_rename(conversation_id, title.clone(), ctx)
    });
    if matches!(
        begin_result,
        Err(BeginConversationRenameError::RenameInProgress)
    ) && queue_rename_behind_in_flight(conversation_id, title.clone(), feedback, ctx)
    {
        return;
    }
    let server_conversation_id = match begin_result {
        Ok(server_conversation_id) => server_conversation_id,
        Err(err) => {
            let message = match err {
                BeginConversationRenameError::MissingServerConversationToken => NOT_SYNCED_MESSAGE,
                BeginConversationRenameError::RenameInProgress => RENAME_IN_PROGRESS_MESSAGE,
                BeginConversationRenameError::ConversationNotFound => {
                    CONVERSATION_NOT_FOUND_MESSAGE
                }
                BeginConversationRenameError::ConversationNotReady => {
                    CONVERSATION_NOT_READY_MESSAGE
                }
            };
            show_toast(feedback, DismissibleToast::error(message.to_owned()), ctx);
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
            let history = BlocklistAIHistoryModel::handle(ctx);
            match result {
                Ok(response) => {
                    let title = response.title;
                    let queued_title = history.update(ctx, |history, ctx| {
                        history.complete_conversation_rename(conversation_id, title.clone(), ctx)
                    });
                    let applied_title = queued_title.clone().unwrap_or(title);
                    show_toast(
                        feedback,
                        DismissibleToast::success(format!(
                            "Conversation renamed to {applied_title}"
                        )),
                        ctx,
                    );
                    resume_queued_rename(conversation_id, queued_title, ctx);
                }
                Err(e) => {
                    let queued_title = history.update(ctx, |history, ctx| {
                        history.fail_conversation_rename(conversation_id, ctx)
                    });
                    show_toast(
                        feedback,
                        DismissibleToast::error(format!("Failed to rename conversation: {e}")),
                        ctx,
                    );
                    resume_queued_rename(conversation_id, queued_title, ctx);
                }
            }
        },
    );
}

/// Holds `title` as the conversation's newest requested name while an earlier rename is
/// still in flight, so a burst of renames settles on the last one instead of the first.
///
/// Only an indirect rename is coalesced. A direct rename already tells the user it collided
/// with an in-flight one, so it keeps reporting that rather than silently deferring.
/// Returns whether the title was queued.
fn queue_rename_behind_in_flight<T: View>(
    conversation_id: AIConversationId,
    title: String,
    feedback: RenameFeedback,
    ctx: &mut ViewContext<T>,
) -> bool {
    if feedback != RenameFeedback::Quiet {
        return false;
    }
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
        history.queue_conversation_rename(conversation_id, title, ctx)
    })
}

/// Sends the title that was queued behind the rename that just settled. It is already
/// applied locally, so this only brings the server in line with what the user sees.
fn resume_queued_rename<T: View>(
    conversation_id: AIConversationId,
    queued_title: Option<String>,
    ctx: &mut ViewContext<T>,
) {
    let Some(title) = queued_title else {
        return;
    };
    rename_conversation_with_feedback(conversation_id, title, RenameFeedback::Quiet, ctx);
}

fn show_toast<T: View>(
    feedback: RenameFeedback,
    toast: DismissibleToast<WorkspaceAction>,
    ctx: &mut ViewContext<T>,
) {
    if feedback == RenameFeedback::Quiet {
        return;
    }
    let window_id = ctx.window_id();
    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
        toast_stack.add_ephemeral_toast(toast, window_id, ctx);
    });
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
        return Err(EMPTY_TITLE_MESSAGE.to_owned());
    }

    if title.chars().count() > CONVERSATION_TITLE_MAX_CHARS {
        return Err(format!(
            "Conversation title must be {CONVERSATION_TITLE_MAX_CHARS} characters or fewer",
        ));
    }

    Ok(title.to_owned())
}
