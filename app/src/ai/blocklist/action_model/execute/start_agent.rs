use std::collections::HashMap;

use warp_errors::report_error;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{LifecycleEventType, StartAgentExecutionMode};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};

/// Per-request outcome of a StartAgent dispatch.
#[derive(Debug, Clone)]
pub enum StartAgentOutcome {
    Started {
        agent_id: String,
    },
    /// An error occurred while starting the agent.
    Error(String),
}

/// Opaque, monotonically increasing request identifier.
/// Disambiguates parallel in-flight StartAgent requests.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Default)]
pub struct StartAgentRequestId(u64);

impl StartAgentRequestId {
    #[cfg(test)]
    pub const fn from_raw_for_test(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone)]
pub struct StartAgentRequest {
    pub id: StartAgentRequestId,
    pub name: String,
    pub prompt: String,
    pub execution_mode: StartAgentExecutionMode,
    pub lifecycle_subscription: Option<Vec<LifecycleEventType>>,
    pub parent_conversation_id: AIConversationId,
    pub parent_run_id: Option<String>,
}

struct PendingStartAgent {
    parent_conversation_id: AIConversationId,
    /// Set once the child conversation is synchronously created.
    child_conversation_id: Option<AIConversationId>,
    sender: async_channel::Sender<StartAgentOutcome>,
}

pub struct StartAgentExecutor {
    pending: HashMap<StartAgentRequestId, PendingStartAgent>,
    next_request_id: u64,
}

impl StartAgentExecutor {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, Self::handle_history_event);

        Self {
            pending: HashMap::new(),
            next_request_id: 0,
        }
    }

    fn next_request_id(&mut self) -> StartAgentRequestId {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        StartAgentRequestId(id)
    }

    /// Links a pending request to its freshly-created child
    /// conversation so subsequent history events can find it.
    fn record_child_conversation(
        &mut self,
        request_id: StartAgentRequestId,
        child_conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(pending) = self.pending.get_mut(&request_id) else {
            return;
        };
        pending.child_conversation_id = Some(child_conversation_id);
        self.maybe_complete_pending_for_child_state(request_id, child_conversation_id, ctx);
    }

    fn find_pending_by_child(
        &self,
        child_conversation_id: &AIConversationId,
    ) -> Option<StartAgentRequestId> {
        self.pending.iter().find_map(|(id, pending)| {
            (pending.child_conversation_id.as_ref() == Some(child_conversation_id)).then_some(*id)
        })
    }

    fn complete_pending_as_started(
        &mut self,
        request_id: StartAgentRequestId,
        child_conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(pending) = self.pending.remove(&request_id) else {
            return;
        };
        let agent_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&child_conversation_id)
            .and_then(|conversation| conversation.orchestration_agent_id());
        match agent_id {
            Some(id) => {
                let _ = pending.sender.try_send(StartAgentOutcome::Started {
                    agent_id: id.clone(),
                });
                OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
                    streamer.register_watched_run_id(pending.parent_conversation_id, id, ctx);
                });
            }
            None => {
                report_error!(
                    "No agent identifier found for child conversation",
                    extra: { "child_conversation_id" => ?child_conversation_id }
                );
                let _ = pending.sender.try_send(StartAgentOutcome::Error(
                    "Server did not assign an agent identifier".to_string(),
                ));
            }
        }
    }

    fn complete_pending_as_error(
        &mut self,
        request_id: StartAgentRequestId,
        child_conversation_id: AIConversationId,
        error_msg: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(pending) = self.pending.remove(&request_id) else {
            return;
        };
        let _ = pending.sender.try_send(StartAgentOutcome::Error(error_msg));
        // A child that reaches `complete_pending_as_error` never obtained an
        // agent id, so it failed at the launch stage. Clean up its hidden
        // pane + conversation so the orchestration pill bar does not retain a
        // dead chip — but only for terminal failures, leaving recoverable
        // `Blocked` startup states (e.g. awaiting GitHub auth) intact.
        let should_cleanup = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&child_conversation_id)
            .is_some_and(|conversation| should_cleanup_failed_child_launch(conversation.status()));
        if should_cleanup {
            ctx.emit(StartAgentExecutorEvent::CleanupFailedChildLaunch {
                conversation_id: child_conversation_id,
            });
        }
    }

    fn maybe_complete_pending_for_child_state(
        &mut self,
        request_id: StartAgentRequestId,
        child_conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&child_conversation_id)
        else {
            return;
        };
        if let Some(error_msg) = start_agent_error_message_for_status(
            conversation.status(),
            conversation.status_error_message().as_deref(),
        ) {
            self.complete_pending_as_error(request_id, child_conversation_id, error_msg, ctx);
            return;
        }
        if conversation.orchestration_agent_id().is_some() {
            self.complete_pending_as_started(request_id, child_conversation_id, ctx);
        }
    }

    fn handle_history_event(
        &mut self,
        _: ModelHandle<BlocklistAIHistoryModel>,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id, ..
            } => {
                let Some(request_id) = self.find_pending_by_child(conversation_id) else {
                    return;
                };
                self.complete_pending_as_started(request_id, *conversation_id, ctx);
            }
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            } => {
                let Some(request_id) = self.find_pending_by_child(conversation_id) else {
                    return;
                };
                let history = BlocklistAIHistoryModel::as_ref(ctx);
                let Some(conversation) = history.conversation(conversation_id) else {
                    return;
                };
                let error_msg = start_agent_error_message_for_status(
                    conversation.status(),
                    conversation.status_error_message().as_deref(),
                );
                if let Some(error_msg) = error_msg {
                    self.complete_pending_as_error(request_id, *conversation_id, error_msg, ctx);
                }
            }
            BlocklistAIHistoryEvent::NewConversationRequestComplete {
                request_id,
                conversation_id,
            } => {
                self.record_child_conversation(*request_id, *conversation_id, ctx);
            }
            BlocklistAIHistoryEvent::StartedNewConversation { .. }
            | BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::ReassignedExchange { .. }
            | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::RemoveConversation { .. }
            | BlocklistAIHistoryEvent::DeletedConversation { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationTitle { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
            | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces { .. } => {}
            BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => {}
        }
    }

    /// Dispatch a pre-validated StartAgent request. Returns a receiver
    /// for the resulting [`StartAgentOutcome`].
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch(
        &mut self,
        name: String,
        prompt: String,
        execution_mode: StartAgentExecutionMode,
        lifecycle_subscription: Option<Vec<LifecycleEventType>>,
        parent_conversation_id: AIConversationId,
        parent_run_id: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) -> async_channel::Receiver<StartAgentOutcome> {
        let (sender, receiver) = async_channel::bounded(1);
        let request_id = self.next_request_id();
        self.pending.insert(
            request_id,
            PendingStartAgent {
                parent_conversation_id,
                child_conversation_id: None,
                sender,
            },
        );
        ctx.emit(StartAgentExecutorEvent::CreateAgent(Box::new(
            StartAgentRequest {
                id: request_id,
                name,
                prompt,
                execution_mode,
                lifecycle_subscription,
                parent_conversation_id,
                parent_run_id,
            },
        )));
        receiver
    }
}

/// Whether a child that failed before launch should have its hidden pane and
/// conversation cleaned up. Only terminal launch failures qualify; recoverable
/// `Blocked` startup states (e.g. awaiting GitHub auth) and non-terminal
/// `TransientError` (a recovery is in flight) keep their chip so the user can
/// resolve them or let the retry complete.
fn should_cleanup_failed_child_launch(status: &ConversationStatus) -> bool {
    match status {
        ConversationStatus::Error | ConversationStatus::Cancelled => true,
        ConversationStatus::Blocked { .. }
        | ConversationStatus::InProgress
        | ConversationStatus::TransientError
        | ConversationStatus::Success
        | ConversationStatus::WaitingForEvents => false,
    }
}

fn start_agent_error_message_for_status(
    status: &ConversationStatus,
    error_message: Option<&str>,
) -> Option<String> {
    match status {
        ConversationStatus::Error => Some(
            error_message
                .filter(|message| !message.trim().is_empty())
                .unwrap_or("Child agent failed to initialize")
                .to_string(),
        ),
        ConversationStatus::Cancelled => {
            Some("Child agent was cancelled before initialization".to_string())
        }
        ConversationStatus::Blocked { blocked_action } => {
            let blocked_action = blocked_action.trim();
            Some(if blocked_action.is_empty() {
                "Child agent startup was blocked before initialization".to_string()
            } else {
                blocked_action.to_string()
            })
        }
        // `WaitingForEvents` is treated like `InProgress`/`Success` here:
        // a child that's actively waiting for events has, by definition,
        // already initialized successfully and is not an error case.
        // TransientError is likewise non-terminal: a recovery is in flight,
        // so keep waiting. The agent run is still in flight in all of these
        // cases, so we don't surface an error message for the start path.
        ConversationStatus::InProgress
        | ConversationStatus::TransientError
        | ConversationStatus::Success
        | ConversationStatus::WaitingForEvents => None,
    }
}

impl Entity for StartAgentExecutor {
    type Event = StartAgentExecutorEvent;
}

pub enum StartAgentExecutorEvent {
    CreateAgent(Box<StartAgentRequest>),
    /// A child agent failed at the launch stage (never started a server-side
    /// run). The owning terminal view removes its hidden pane and conversation
    /// so the orchestration pill bar does not retain a dead chip.
    CleanupFailedChildLaunch {
        conversation_id: AIConversationId,
    },
}
