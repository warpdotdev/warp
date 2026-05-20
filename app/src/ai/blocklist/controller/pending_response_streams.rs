use std::collections::HashMap;

use itertools::Itertools;
use warpui::{AppContext, ModelContext, ModelHandle, SingletonEntity};

use crate::{
    ai::agent::{conversation::AIConversationId, CancellationReason},
    BlocklistAIHistoryModel,
};

use super::{
    response_stream::{ResponseStream, ResponseStreamId},
    BlocklistAIController,
};

pub(super) struct PendingResponseStreams {
    streams: HashMap<ResponseStreamId, ModelHandle<ResponseStream>>,
}

impl PendingResponseStreams {
    pub fn new() -> Self {
        Self {
            streams: HashMap::new(),
        }
    }

    pub fn has_active_stream_for_conversation(
        &self,
        conversation_id: AIConversationId,
        app: &AppContext,
    ) -> bool {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let Some(conversation) = history_model.conversation(&conversation_id) else {
            return false;
        };
        self.streams
            .keys()
            .any(|stream_id| conversation.is_processing_response_stream(stream_id))
    }

    pub fn register_new_stream(
        &mut self,
        stream_id: ResponseStreamId,
        conversation_id: AIConversationId,
        stream: ModelHandle<ResponseStream>,
        reason: CancellationReason,
        ctx: &mut ModelContext<BlocklistAIController>,
    ) {
        self.try_cancel_streams_for_conversation(conversation_id, reason, ctx);
        self.streams.insert(stream_id, stream);
    }

    pub fn cleanup_stream(&mut self, stream_id: &ResponseStreamId) {
        self.streams.remove(stream_id);
    }

    pub fn try_cancel_stream(
        &mut self,
        stream_id: &ResponseStreamId,
        reason: CancellationReason,
        ctx: &mut ModelContext<BlocklistAIController>,
    ) -> bool {
        if let Some(stream) = self.streams.remove(stream_id) {
            // Look up which conversation owns this stream
            let Some(conversation_id) =
                BlocklistAIHistoryModel::as_ref(ctx).conversation_for_response_stream(stream_id)
            else {
                log::warn!("Could not find conversation for stream {stream_id:?}, cannot cancel");
                return false;
            };

            stream.update(ctx, |stream, ctx| {
                stream.cancel(reason, conversation_id, ctx)
            });
            return true;
        }
        false
    }

    /// Cancels all streams for the given conversation
    pub fn try_cancel_streams_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        reason: CancellationReason,
        ctx: &mut ModelContext<BlocklistAIController>,
    ) -> bool {
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let Some(conversation) = history_model.conversation(&conversation_id) else {
            return false;
        };

        let streams_to_cancel = self
            .streams
            .extract_if(|stream_id, _| conversation.is_processing_response_stream(stream_id))
            .map(|(_, stream)| stream)
            .collect_vec();

        if streams_to_cancel.is_empty() {
            false
        } else {
            for response_stream in streams_to_cancel.into_iter() {
                log::info!(
                    "Canceling active stream for conversation_id={conversation_id:?}, \
                     reason={reason}, backtrace=\n{}",
                    std::backtrace::Backtrace::force_capture()
                );
                response_stream.update(ctx, |stream, ctx| {
                    stream.cancel(reason, conversation_id, ctx)
                });
            }
            true
        }
    }
}
