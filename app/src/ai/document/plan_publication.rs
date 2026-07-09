//! Publishing plan documents to Warp Drive ahead of child-agent launch.
//!
//! Callers first call [`prepare_plan_publications`] to kick off publication of
//! a conversation's plans, then await [`wait_for_plan_publications`] before
//! proceeding with work that requires the plans to be server-backed.
use std::time::Duration;

use warpui::r#async::FutureExt as WarpFutureExt;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel, AIDocumentModelEvent};
use crate::report_error;

/// Bounded wait per plan so a failed or slow publication cannot stall callers
/// indefinitely.
const PLAN_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(30);

/// A plan publication that has been started but is not yet server-backed.
pub(in crate::ai) struct PendingPlanPublication {
    document_id: AIDocumentId,
    save_wait: async_channel::Receiver<()>,
}

/// Publishes every plan owned by the conversation and returns a pending entry
/// for each plan that is not yet server-backed.
pub(in crate::ai) fn prepare_plan_publications<E: Entity>(
    conversation_id: AIConversationId,
    ctx: &mut ModelContext<E>,
) -> Vec<PendingPlanPublication> {
    let document_model = AIDocumentModel::handle(ctx);
    let awaiting_server_backing = document_model.update(ctx, |model, ctx| {
        model.publish_documents_for_conversation(conversation_id, ctx)
    });

    awaiting_server_backing
        .into_iter()
        .filter_map(|document_id| {
            let (save_tx, save_rx) = async_channel::bounded(1);
            ctx.subscribe_to_model(&document_model, move |_, _, event, ctx| {
                let AIDocumentModelEvent::DocumentSaveStatusUpdated(saved_document_id) = event
                else {
                    return;
                };
                if *saved_document_id != document_id {
                    return;
                }
                if AIDocumentModel::as_ref(ctx)
                    .get_document_save_status(&document_id)
                    .is_saved()
                {
                    let _ = save_tx.try_send(());
                }
            });
            if document_model
                .as_ref(ctx)
                .get_document_save_status(&document_id)
                .is_saved()
            {
                None
            } else {
                Some(PendingPlanPublication {
                    document_id,
                    save_wait: save_rx,
                })
            }
        })
        .collect()
}

/// Waits (best-effort, bounded per plan) for each pending publication to
/// become server-backed. Resolves immediately when `pending` is empty.
pub(in crate::ai) async fn wait_for_plan_publications(pending: Vec<PendingPlanPublication>) {
    futures::future::join_all(pending.into_iter().map(|pending| async move {
        match pending
            .save_wait
            .recv()
            .with_timeout(PLAN_PUBLICATION_TIMEOUT)
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                report_error!(
                    "Stopped waiting for plan document before it became server-backed.",
                    extra: { "document_id" => %pending.document_id }
                );
            }
            Err(_) => {
                report_error!(
                    "Timed out waiting for plan document to become server-backed before child-agent launch.",
                    extra: { "document_id" => %pending.document_id }
                );
            }
        }
    }))
    .await;
}
