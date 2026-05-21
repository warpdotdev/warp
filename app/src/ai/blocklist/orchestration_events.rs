use super::history_model::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, ConversationStatusUpdate,
};
use super::telemetry::{
    BlocklistOrchestrationTelemetryEvent, TeamAgentCommunicationFailedEvent,
    TeamAgentCommunicationFailureReason, TeamAgentCommunicationKind,
    TeamAgentCommunicationTransport, TeamAgentOrchestrationVersion,
};
use crate::ai::agent::{
    conversation::{AIConversationId, ConversationStatus},
    task::TaskId,
    AIAgentExchangeId, AIAgentInput, AIAgentOutputMessageType, LifecycleEventType,
    ReceivedMessageInput,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_multi_agent_api as api;
use warpui::{Entity, ModelContext, SingletonEntity};

const MAX_RETRY_ATTEMPTS: i32 = 3;
const MAX_PENDING_LIFECYCLE_EVENTS_PER_TARGET: usize = 200;

/// Stage associated with a lifecycle error detail.
/// This keeps persisted/runtime metadata consistent across API payloads and DB rows.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LifecycleEventDetailStage {
    Startup,
    Runtime,
}

#[derive(Debug, Clone)]
struct LifecycleSubscriptionRoute {
    target_agent_id: String,
    subscribed_event_types: Option<Vec<LifecycleEventType>>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct LifecycleEventDetailPayload {
    pub(crate) stage: Option<LifecycleEventDetailStage>,
    pub(crate) reason: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) blocked_action: Option<String>,
}

impl LifecycleEventDetailStage {
    /// Canonical lowercase representation used in persistence/API payloads.
    fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Runtime => "runtime",
        }
    }
}

/// Type-specific queued data, including service-generated fields.
#[derive(Debug, Clone)]
pub enum PendingEventDetail {
    Message {
        #[cfg_attr(not(test), allow(dead_code))]
        sequence: i64,
        message_id: String,
        addresses: Vec<String>,
        subject: String,
        message_body: String,
        #[cfg_attr(not(test), allow(dead_code))]
        occurred_at: String,
    },
    Lifecycle {
        event: api::AgentEvent,
    },
}

/// A queued event consumed by the controller.
#[derive(Debug, Clone)]
pub struct PendingEvent {
    pub event_id: String,
    pub source_agent_id: String,
    pub attempt_count: i32,
    pub detail: PendingEventDetail,
}

/// Result returned from lifecycle send operations.
pub enum SendEventResult {
    LifecycleSent,
    LifecycleDropped,
    Error(String),
}

pub enum SendMessageResult {
    MessageSent { message_id: String },
    Error(String),
}

pub enum OrchestrationEventServiceEvent {
    /// Signals that a conversation may have pending orchestration events
    /// ready to drain.
    EventsReady { conversation_id: AIConversationId },
}

/// Synchronous state manager for orchestration event queuing, delivery
/// tracking, lifecycle dispatch, and readiness detection.
// TODO(QUALITY-733): Remove the legacy v1 orchestration event-service paths once v2 delivery no
// longer reuses this service for local queueing and controller injection.
pub struct OrchestrationEventService {
    pending_events: HashMap<AIConversationId, Vec<PendingEvent>>,
    awaiting_server_echo_events: HashMap<AIConversationId, Vec<PendingEvent>>,
    lifecycle_subscription_routes: HashMap<AIConversationId, Vec<LifecycleSubscriptionRoute>>,
    conversation_statuses: HashMap<AIConversationId, ConversationStatus>,
}

impl OrchestrationEventService {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |me, event, ctx| {
            me.handle_history_event(event, ctx);
        });
        Self::new_without_subscriptions()
    }

    fn new_without_subscriptions() -> Self {
        Self {
            pending_events: HashMap::new(),
            awaiting_server_echo_events: HashMap::new(),
            lifecycle_subscription_routes: HashMap::new(),
            conversation_statuses: HashMap::new(),
        }
    }

    pub fn register_lifecycle_subscription(
        &mut self,
        source_conversation_id: AIConversationId,
        target_agent_id: String,
        subscribed_event_types: Option<Vec<LifecycleEventType>>,
    ) {
        let routes = self
            .lifecycle_subscription_routes
            .entry(source_conversation_id)
            .or_default();
        if let Some(existing_route) = routes
            .iter_mut()
            .find(|route| route.target_agent_id == target_agent_id)
        {
            existing_route.subscribed_event_types = subscribed_event_types;
            return;
        }
        routes.push(LifecycleSubscriptionRoute {
            target_agent_id,
            subscribed_event_types,
        });
    }

    #[allow(deprecated)]
    pub fn emit_child_startup_started(
        &mut self,
        child_conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let result = self.dispatch_lifecycle_event(
            child_conversation_id,
            LifecycleEventType::Started,
            LifecycleEventDetailPayload::default(),
            ctx,
        );
        self.log_lifecycle_dispatch_result(
            child_conversation_id,
            LifecycleEventType::Started,
            result,
        );
    }

    pub fn emit_child_startup_errored(
        &mut self,
        child_conversation_id: AIConversationId,
        reason: String,
        error_message: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let result = self.dispatch_lifecycle_event(
            child_conversation_id,
            LifecycleEventType::Errored,
            LifecycleEventDetailPayload {
                stage: Some(LifecycleEventDetailStage::Startup),
                reason: Some(reason),
                error_message: Some(error_message),
                blocked_action: None,
            },
            ctx,
        );
        self.log_lifecycle_dispatch_result(
            child_conversation_id,
            LifecycleEventType::Errored,
            result,
        );
    }

    pub fn emit_child_killed(
        &mut self,
        child_conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> SendEventResult {
        let (source_agent_id, parent_conversation_id) = {
            let history_model = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(child_conversation) = history_model.conversation(&child_conversation_id)
            else {
                return SendEventResult::Error("Child conversation not found".to_string());
            };
            if !child_conversation.status().is_in_progress() {
                return SendEventResult::LifecycleDropped;
            }
            let Some(source_agent_id) = child_conversation.orchestration_agent_id() else {
                return SendEventResult::Error(
                    "Child conversation has no agent identifier".to_string(),
                );
            };
            let parent_conversation_id =
                child_conversation.parent_conversation_id().or_else(|| {
                    child_conversation
                        .parent_agent_id()
                        .and_then(|agent_id| history_model.conversation_id_for_agent_id(agent_id))
                });
            let Some(parent_conversation_id) = parent_conversation_id else {
                return SendEventResult::LifecycleDropped;
            };
            (source_agent_id, parent_conversation_id)
        };

        let occurred_at = chrono::Utc::now();
        let occurred_at_proto = prost_types::Timestamp {
            seconds: occurred_at.timestamp(),
            nanos: occurred_at.timestamp_subsec_nanos() as i32,
        };
        let event_id = Uuid::new_v4().to_string();
        let event = build_lifecycle_event(
            event_id.clone(),
            source_agent_id.clone(),
            LifecycleEventType::Cancelled,
            occurred_at_proto,
            &LifecycleEventDetailPayload::default(),
        );
        self.enqueue_lifecycle_event(
            parent_conversation_id,
            PendingEvent {
                event_id,
                source_agent_id,
                attempt_count: 0,
                detail: PendingEventDetail::Lifecycle { event },
            },
        );
        ctx.emit(OrchestrationEventServiceEvent::EventsReady {
            conversation_id: parent_conversation_id,
        });
        SendEventResult::LifecycleSent
    }

    fn dispatch_lifecycle_event(
        &mut self,
        source_conversation_id: AIConversationId,
        event_type: LifecycleEventType,
        detail_payload: LifecycleEventDetailPayload,
        ctx: &mut ModelContext<Self>,
    ) -> SendEventResult {
        if event_type == LifecycleEventType::Unspecified {
            send_telemetry_from_ctx!(
                BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                    TeamAgentCommunicationFailedEvent {
                        communication_kind: TeamAgentCommunicationKind::LifecycleEvent,
                        transport: TeamAgentCommunicationTransport::Local,
                        orchestration_version: TeamAgentOrchestrationVersion::V1,
                        failure_reason:
                            TeamAgentCommunicationFailureReason::InvalidLifecycleEventType,
                        source_conversation_id,
                        source_run_id: None,
                        target_count: None,
                        lifecycle_event_type: Some(
                            lifecycle_event_type_name(event_type).to_string(),
                        ),
                        error_message: None,
                    }
                ),
                ctx
            );
            return SendEventResult::Error(
                "Cannot send lifecycle event with unspecified type".to_string(),
            );
        }

        let sender_agent_id = {
            let history_model = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(source_conversation) = history_model.conversation(&source_conversation_id)
            else {
                send_telemetry_from_ctx!(
                    BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                        TeamAgentCommunicationFailedEvent {
                            communication_kind: TeamAgentCommunicationKind::LifecycleEvent,
                            transport: TeamAgentCommunicationTransport::Local,
                            orchestration_version: TeamAgentOrchestrationVersion::V1,
                            failure_reason:
                                TeamAgentCommunicationFailureReason::MissingSourceConversation,
                            source_conversation_id,
                            source_run_id: None,
                            target_count: None,
                            lifecycle_event_type: Some(
                                lifecycle_event_type_name(event_type).to_string(),
                            ),
                            error_message: None,
                        }
                    ),
                    ctx
                );
                return SendEventResult::Error("Source conversation not found".to_string());
            };
            let Some(sender_agent_id) = source_conversation
                .server_conversation_token()
                .map(|token| token.as_str().to_string())
            else {
                send_telemetry_from_ctx!(
                    BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                        TeamAgentCommunicationFailedEvent {
                            communication_kind: TeamAgentCommunicationKind::LifecycleEvent,
                            transport: TeamAgentCommunicationTransport::Local,
                            orchestration_version: TeamAgentOrchestrationVersion::V1,
                            failure_reason:
                                TeamAgentCommunicationFailureReason::MissingSourceIdentifier,
                            source_conversation_id,
                            source_run_id: None,
                            target_count: None,
                            lifecycle_event_type: Some(
                                lifecycle_event_type_name(event_type).to_string(),
                            ),
                            error_message: None,
                        }
                    ),
                    ctx
                );
                return SendEventResult::Error(
                    "Source conversation has no server token — cannot send events".to_string(),
                );
            };
            sender_agent_id
        };

        let Some(routes) = self
            .lifecycle_subscription_routes
            .get(&source_conversation_id)
        else {
            return SendEventResult::LifecycleDropped;
        };

        let mut resolved_targets = Vec::new();
        {
            let history_model = BlocklistAIHistoryModel::as_ref(ctx);
            for route in routes {
                if !is_subscribed(route.subscribed_event_types.as_deref(), event_type) {
                    continue;
                }
                let Some(conversation_id) =
                    history_model.conversation_id_for_agent_id(&route.target_agent_id)
                else {
                    send_telemetry_from_ctx!(
                        BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                            TeamAgentCommunicationFailedEvent {
                                communication_kind: TeamAgentCommunicationKind::LifecycleEvent,
                                transport: TeamAgentCommunicationTransport::Local,
                                orchestration_version: TeamAgentOrchestrationVersion::V1,
                                failure_reason: TeamAgentCommunicationFailureReason::UnknownAgent,
                                source_conversation_id,
                                source_run_id: None,
                                target_count: Some(1),
                                lifecycle_event_type: Some(
                                    lifecycle_event_type_name(event_type).to_string(),
                                ),
                                error_message: None,
                            }
                        ),
                        ctx
                    );
                    log::warn!(
                        "OrchestrationEventService: could not resolve lifecycle target {}",
                        route.target_agent_id
                    );
                    continue;
                };
                resolved_targets.push((route.target_agent_id.clone(), conversation_id));
            }
        }

        if resolved_targets.is_empty() {
            return SendEventResult::LifecycleDropped;
        }

        self.send_lifecycle_event(
            &sender_agent_id,
            &resolved_targets,
            event_type,
            &detail_payload,
            ctx,
        )
    }

    pub fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id,
                update,
                ..
            } => {
                let is_restored = matches!(update, ConversationStatusUpdate::Restored);
                self.on_conversation_status_updated(*conversation_id, is_restored, ctx)
            }
            BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id,
                exchange_id,
                ..
            } => self.confirm_delivery_from_exchange(*conversation_id, *exchange_id, ctx),
            BlocklistAIHistoryEvent::StartedNewConversation {
                new_conversation_id,
                ..
            } => self.sync_conversation_status(*new_conversation_id, ctx),
            BlocklistAIHistoryEvent::RestoredConversations {
                conversation_ids, ..
            } => {
                for conversation_id in conversation_ids {
                    self.sync_conversation_status(*conversation_id, ctx);
                    self.restore_v1_lifecycle_subscription(*conversation_id, ctx);
                }
            }
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                self.pending_events.remove(conversation_id);
                self.awaiting_server_echo_events.remove(conversation_id);
                self.lifecycle_subscription_routes.remove(conversation_id);
                self.conversation_statuses.remove(conversation_id);
            }
            _ => {}
        }
    }

    fn log_lifecycle_dispatch_result(
        &self,
        child_conversation_id: AIConversationId,
        event_type: LifecycleEventType,
        result: SendEventResult,
    ) {
        let event_type_name = lifecycle_event_type_name(event_type);
        match result {
            SendEventResult::LifecycleSent => {
                log::debug!(
                    "LIFECYCLE-EVENT-DEBUG: Emitted child lifecycle event: event_type={event_type_name} child_conversation_id={child_conversation_id:?}"
                );
            }
            SendEventResult::LifecycleDropped => {
                log::debug!(
                    "LIFECYCLE-EVENT-DEBUG: Dropped child lifecycle event due to lifecycle subscription filtering: event_type={event_type_name} child_conversation_id={child_conversation_id:?}"
                );
            }
            SendEventResult::Error(error) => {
                log::warn!(
                    "LIFECYCLE-EVENT-WARN: Failed to emit lifecycle event for child agent: event_type={event_type_name} child_conversation_id={child_conversation_id:?} error={error}"
                );
            }
        }
    }

    fn sync_conversation_status(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &ModelContext<Self>,
    ) {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            self.conversation_statuses.remove(&conversation_id);
            return;
        };
        self.conversation_statuses
            .insert(conversation_id, conversation.status().clone());
    }

    fn restore_v1_lifecycle_subscription(
        &mut self,
        source_conversation_id: AIConversationId,
        ctx: &ModelContext<Self>,
    ) {
        // TODO(QUALITY-733): Remove restored v1 lifecycle subscriptions once legacy
        // orchestration lifecycle dispatch is deleted.
        if FeatureFlag::OrchestrationV2.is_enabled() {
            return;
        }

        let target_agent_id = {
            let history_model = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(conversation) = history_model.conversation(&source_conversation_id) else {
                return;
            };
            if !conversation.is_child_agent_conversation() {
                return;
            }

            conversation
                .parent_conversation_id()
                .and_then(|parent_id| history_model.conversation(&parent_id))
                .and_then(|parent| parent.orchestration_agent_id())
                .or_else(|| conversation.parent_agent_id().map(str::to_string))
        };

        if let Some(target_agent_id) = target_agent_id {
            self.register_lifecycle_subscription(source_conversation_id, target_agent_id, None);
        }
    }

    fn on_conversation_status_updated(
        &mut self,
        conversation_id: AIConversationId,
        is_restored: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let (is_child_agent_conversation, current_status, status_error_message) = {
            let Some(conversation) =
                BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
            else {
                self.conversation_statuses.remove(&conversation_id);
                return;
            };
            (
                conversation.is_child_agent_conversation(),
                conversation.status().clone(),
                conversation.status_error_message().map(str::to_string),
            )
        };

        let previous_status = self
            .conversation_statuses
            .insert(conversation_id, current_status.clone());
        let has_pending = self
            .pending_events
            .get(&conversation_id)
            .is_some_and(|events| !events.is_empty());
        if !is_restored && matches!(&current_status, ConversationStatus::Success) && has_pending {
            ctx.emit(OrchestrationEventServiceEvent::EventsReady { conversation_id });
        }

        if is_restored || !is_child_agent_conversation {
            return;
        }

        // When v2 is enabled, lifecycle events are delivered via the server
        // event log (poller reports → polls back → enqueues). Skip the v1
        // local dispatch to avoid duplicate delivery.
        if FeatureFlag::OrchestrationV2.is_enabled() {
            return;
        }
        // TODO(QUALITY-733): Remove legacy v1 lifecycle dispatch once all child-agent lifecycle
        // events are delivered through the v2 event log.

        #[allow(deprecated)]
        match (previous_status.as_ref(), &current_status) {
            (Some(ConversationStatus::Success), ConversationStatus::InProgress) => {
                let result = self.dispatch_lifecycle_event(
                    conversation_id,
                    LifecycleEventType::Restarted,
                    LifecycleEventDetailPayload::default(),
                    ctx,
                );
                self.log_lifecycle_dispatch_result(
                    conversation_id,
                    LifecycleEventType::Restarted,
                    result,
                );
            }
            (Some(ConversationStatus::Blocked { .. }), ConversationStatus::InProgress) => {
                let result = self.dispatch_lifecycle_event(
                    conversation_id,
                    LifecycleEventType::Restarted,
                    LifecycleEventDetailPayload::default(),
                    ctx,
                );
                self.log_lifecycle_dispatch_result(
                    conversation_id,
                    LifecycleEventType::Restarted,
                    result,
                );
            }
            (Some(ConversationStatus::InProgress), ConversationStatus::Success) => {
                let result = self.dispatch_lifecycle_event(
                    conversation_id,
                    LifecycleEventType::Idle,
                    LifecycleEventDetailPayload::default(),
                    ctx,
                );
                self.log_lifecycle_dispatch_result(
                    conversation_id,
                    LifecycleEventType::Idle,
                    result,
                );
            }
            (Some(ConversationStatus::InProgress), ConversationStatus::Error) => {
                let result = self.dispatch_lifecycle_event(
                    conversation_id,
                    LifecycleEventType::Errored,
                    LifecycleEventDetailPayload {
                        stage: Some(LifecycleEventDetailStage::Runtime),
                        reason: Some("conversation_error".to_string()),
                        error_message: status_error_message,
                        blocked_action: None,
                    },
                    ctx,
                );
                self.log_lifecycle_dispatch_result(
                    conversation_id,
                    LifecycleEventType::Errored,
                    result,
                );
            }
            (
                Some(ConversationStatus::InProgress),
                ConversationStatus::Blocked { blocked_action },
            ) => {
                let result = self.dispatch_lifecycle_event(
                    conversation_id,
                    LifecycleEventType::Blocked,
                    LifecycleEventDetailPayload {
                        stage: None,
                        reason: None,
                        error_message: None,
                        blocked_action: Some(blocked_action.clone()),
                    },
                    ctx,
                );
                self.log_lifecycle_dispatch_result(
                    conversation_id,
                    LifecycleEventType::Blocked,
                    result,
                );
            }
            (Some(ConversationStatus::InProgress), ConversationStatus::Cancelled)
            | (Some(ConversationStatus::Blocked { .. }), ConversationStatus::Cancelled) => {
                let result = self.dispatch_lifecycle_event(
                    conversation_id,
                    LifecycleEventType::Cancelled,
                    LifecycleEventDetailPayload::default(),
                    ctx,
                );
                self.log_lifecycle_dispatch_result(
                    conversation_id,
                    LifecycleEventType::Cancelled,
                    result,
                );
            }
            _ => {}
        }
    }

    /// Send an orchestration event from `source_conversation_id` to each agent
    /// in `target_agent_ids`. Resolves addresses, queues, and emits
    /// `EventsReady` for each target conversation.
    pub fn send_message(
        &mut self,
        source_conversation_id: AIConversationId,
        target_agent_ids: &[String],
        subject: String,
        message_body: String,
        ctx: &mut ModelContext<Self>,
    ) -> SendMessageResult {
        let (sender_agent_id, resolved_targets) = {
            let history_model = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(source_conversation) = history_model.conversation(&source_conversation_id)
            else {
                send_telemetry_from_ctx!(
                    BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                        TeamAgentCommunicationFailedEvent {
                            communication_kind: TeamAgentCommunicationKind::Message,
                            transport: TeamAgentCommunicationTransport::Local,
                            orchestration_version: TeamAgentOrchestrationVersion::V1,
                            failure_reason:
                                TeamAgentCommunicationFailureReason::MissingSourceConversation,
                            source_conversation_id,
                            source_run_id: None,
                            target_count: Some(target_agent_ids.len()),
                            lifecycle_event_type: None,
                            error_message: None,
                        }
                    ),
                    ctx
                );
                let error = "Source conversation not found".to_string();
                self.log_send_message_error(
                    source_conversation_id,
                    target_agent_ids,
                    &subject,
                    &error,
                );
                return SendMessageResult::Error(error);
            };
            let Some(sender_agent_id) = source_conversation
                .server_conversation_token()
                .map(|token| token.as_str().to_string())
            else {
                send_telemetry_from_ctx!(
                    BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                        TeamAgentCommunicationFailedEvent {
                            communication_kind: TeamAgentCommunicationKind::Message,
                            transport: TeamAgentCommunicationTransport::Local,
                            orchestration_version: TeamAgentOrchestrationVersion::V1,
                            failure_reason:
                                TeamAgentCommunicationFailureReason::MissingSourceIdentifier,
                            source_conversation_id,
                            source_run_id: None,
                            target_count: Some(target_agent_ids.len()),
                            lifecycle_event_type: None,
                            error_message: None,
                        }
                    ),
                    ctx
                );
                let error =
                    "Source conversation has no server token — cannot send events".to_string();
                self.log_send_message_error(
                    source_conversation_id,
                    target_agent_ids,
                    &subject,
                    &error,
                );
                return SendMessageResult::Error(error);
            };

            let mut resolved_targets = Vec::new();
            for agent_id in target_agent_ids {
                match history_model.conversation_id_for_agent_id(agent_id) {
                    Some(conversation_id) => {
                        resolved_targets.push((agent_id.clone(), conversation_id));
                    }
                    None => {
                        send_telemetry_from_ctx!(
                            BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                                TeamAgentCommunicationFailedEvent {
                                    communication_kind: TeamAgentCommunicationKind::Message,
                                    transport: TeamAgentCommunicationTransport::Local,
                                    orchestration_version: TeamAgentOrchestrationVersion::V1,
                                    failure_reason:
                                        TeamAgentCommunicationFailureReason::UnknownAgent,
                                    source_conversation_id,
                                    source_run_id: None,
                                    target_count: Some(target_agent_ids.len()),
                                    lifecycle_event_type: None,
                                    error_message: None,
                                }
                            ),
                            ctx
                        );
                        let error = format!("Unknown agent address: {agent_id}");
                        self.log_send_message_error(
                            source_conversation_id,
                            target_agent_ids,
                            &subject,
                            &error,
                        );
                        return SendMessageResult::Error(error);
                    }
                }
            }
            (sender_agent_id, resolved_targets)
        };

        if resolved_targets.is_empty() {
            send_telemetry_from_ctx!(
                BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                    TeamAgentCommunicationFailedEvent {
                        communication_kind: TeamAgentCommunicationKind::Message,
                        transport: TeamAgentCommunicationTransport::Local,
                        orchestration_version: TeamAgentOrchestrationVersion::V1,
                        failure_reason: TeamAgentCommunicationFailureReason::NoTargets,
                        source_conversation_id,
                        source_run_id: None,
                        target_count: Some(0),
                        lifecycle_event_type: None,
                        error_message: None,
                    }
                ),
                ctx
            );
            let error = "No target agents provided".to_string();
            self.log_send_message_error(source_conversation_id, target_agent_ids, &subject, &error);
            return SendMessageResult::Error(error);
        }

        self.send_message_event(
            &sender_agent_id,
            &resolved_targets,
            target_agent_ids,
            subject,
            message_body,
            ctx,
        )
    }

    fn send_message_event(
        &mut self,
        sender_agent_id: &str,
        resolved_targets: &[(String, AIConversationId)],
        target_agent_ids: &[String],
        subject: String,
        message_body: String,
        ctx: &mut ModelContext<Self>,
    ) -> SendMessageResult {
        // One logical message fanout maps to many delivery envelopes (message rows).
        // We keep `message_id` stable across targets so dedupe/threading can reason
        // about a single message delivered to multiple recipients.
        let message_id = Uuid::new_v4().to_string();
        let occurred_at = chrono::Utc::now().to_rfc3339();

        for (_, target_conversation_id) in resolved_targets {
            let event_id = Uuid::new_v4().to_string();

            let pending = PendingEvent {
                event_id,
                source_agent_id: sender_agent_id.to_string(),
                attempt_count: 0,
                detail: PendingEventDetail::Message {
                    sequence: 0,
                    message_id: message_id.clone(),
                    addresses: target_agent_ids.to_vec(),
                    subject: subject.clone(),
                    message_body: message_body.clone(),
                    occurred_at: occurred_at.clone(),
                },
            };
            self.pending_events
                .entry(*target_conversation_id)
                .or_default()
                .push(pending);

            // Signal the controller to check this conversation for pending events.
            // The controller will check readiness (ownership, no in-flight) before draining.
            ctx.emit(OrchestrationEventServiceEvent::EventsReady {
                conversation_id: *target_conversation_id,
            });
        }

        SendMessageResult::MessageSent { message_id }
    }

    fn log_send_message_error(
        &self,
        source_conversation_id: AIConversationId,
        target_agent_ids: &[String],
        subject: &str,
        error: &str,
    ) {
        log::warn!(
            "Failed to send child-agent message: source_conversation_id={source_conversation_id:?} target_agent_ids={target_agent_ids:?} subject={subject:?} error={error}"
        );
    }

    /// Broadcast a lifecycle signal to subscribed targets.
    /// Enqueues an in-memory `AgentEvent` for controller delivery.
    fn send_lifecycle_event(
        &mut self,
        sender_agent_id: &str,
        resolved_targets: &[(String, AIConversationId)],
        event_type: LifecycleEventType,
        detail_payload: &LifecycleEventDetailPayload,
        ctx: &mut ModelContext<Self>,
    ) -> SendEventResult {
        if event_type == LifecycleEventType::Unspecified {
            return SendEventResult::Error(
                "Cannot send lifecycle event with unspecified type".to_string(),
            );
        }
        // Use one timestamp for every target in this fanout so all delivered copies of
        // the same logical lifecycle signal carry identical `occurred_at` semantics.
        let occurred_at = chrono::Utc::now();
        let occurred_at_proto = prost_types::Timestamp {
            seconds: occurred_at.timestamp(),
            nanos: occurred_at.timestamp_subsec_nanos() as i32,
        };
        for (_, target_conversation_id) in resolved_targets {
            let event_id = Uuid::new_v4().to_string();
            let agent_event = build_lifecycle_event(
                event_id.clone(),
                sender_agent_id.to_string(),
                event_type,
                occurred_at_proto,
                detail_payload,
            );

            let pending = PendingEvent {
                event_id: event_id.clone(),
                source_agent_id: sender_agent_id.to_string(),
                attempt_count: 0,
                detail: PendingEventDetail::Lifecycle { event: agent_event },
            };
            self.enqueue_lifecycle_event(*target_conversation_id, pending);

            // Signal the controller to check this conversation for pending events.
            ctx.emit(OrchestrationEventServiceEvent::EventsReady {
                conversation_id: *target_conversation_id,
            });
        }
        if resolved_targets.is_empty() {
            SendEventResult::LifecycleDropped
        } else {
            SendEventResult::LifecycleSent
        }
    }

    fn enqueue_lifecycle_event(
        &mut self,
        target_conversation_id: AIConversationId,
        pending: PendingEvent,
    ) {
        // Lifecycle queues are maintained separately per target conversation.
        // Coalescing and cap enforcement happen before queue insertion.
        let dropped_for_cap = {
            let queue = self
                .pending_events
                .entry(target_conversation_id)
                .or_default();
            let _ = coalesce_lifecycle_events(queue, &pending);
            queue.push(pending);
            enforce_lifecycle_queue_cap(queue, MAX_PENDING_LIFECYCLE_EVENTS_PER_TARGET)
        };
        if !dropped_for_cap.is_empty() {
            log::warn!(
                "Dropped {} coalescable lifecycle events due to queue cap for target conversation {target_conversation_id:?}",
                dropped_for_cap.len()
            );
        }
    }

    /// Accepts pre-built events from the v2 streamer and enqueues them
    /// for drain by the controller via the normal v1 path.
    /// Lifecycle events go through coalescing and cap enforcement.
    pub fn enqueue_event_batch(
        &mut self,
        conversation_id: AIConversationId,
        events: Vec<PendingEvent>,
        ctx: &mut ModelContext<Self>,
    ) {
        if events.is_empty() {
            return;
        }
        for event in events {
            if matches!(event.detail, PendingEventDetail::Lifecycle { .. }) {
                self.enqueue_lifecycle_event(conversation_id, event);
            } else {
                self.pending_events
                    .entry(conversation_id)
                    .or_default()
                    .push(event);
            }
        }
        ctx.emit(OrchestrationEventServiceEvent::EventsReady { conversation_id });
    }

    #[cfg(any(test, not(target_family = "wasm")))]
    pub fn has_pending_events(&self, conversation_id: AIConversationId) -> bool {
        self.pending_events
            .get(&conversation_id)
            .is_some_and(|events| !events.is_empty())
    }

    /// Drain and return all pending events for a conversation.
    fn drain_pending_events(&mut self, conversation_id: &AIConversationId) -> Vec<PendingEvent> {
        self.pending_events
            .remove(conversation_id)
            .unwrap_or_default()
    }

    /// Drains pending events for a conversation, resolves the root task ID,
    /// and converts them to AIAgentInput variants ready for injection.
    /// Returns None if there are no events or the conversation cannot be found
    /// (in which case events are requeued automatically).
    pub fn drain_events_for_request(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<(Vec<AIAgentInput>, TaskId)> {
        let inputs = self.drain_and_convert_events(conversation_id);
        if inputs.is_empty() {
            return None;
        }
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            self.requeue_awaiting_events(conversation_id, ctx);
            return None;
        };
        Some((inputs, conversation.get_root_task_id().clone()))
    }

    /// Drains pending events for a conversation and converts them to
    /// AIAgentInput variants ready for injection. Moves the drained events
    /// to awaiting_server_echo_events for delivery confirmation.
    fn drain_and_convert_events(&mut self, conversation_id: AIConversationId) -> Vec<AIAgentInput> {
        let deliverable = self.drain_pending_events(&conversation_id);
        if deliverable.is_empty() {
            return vec![];
        }

        let mut messages = Vec::new();
        let mut lifecycle_events = Vec::new();
        for event in &deliverable {
            match &event.detail {
                PendingEventDetail::Message {
                    sequence: _,
                    message_id,
                    addresses,
                    subject,
                    message_body,
                    occurred_at: _,
                } => messages.push(ReceivedMessageInput {
                    message_id: message_id.clone(),
                    sender_agent_id: event.source_agent_id.clone(),
                    addresses: addresses.clone(),
                    subject: subject.clone(),
                    message_body: message_body.clone(),
                }),
                PendingEventDetail::Lifecycle { event } => lifecycle_events.push(event.clone()),
            }
        }

        // Move to awaiting echo for delivery confirmation.
        self.awaiting_server_echo_events
            .entry(conversation_id)
            .or_default()
            .extend(deliverable);

        let mut inputs = Vec::new();
        if !messages.is_empty() {
            inputs.push(AIAgentInput::MessagesReceivedFromAgents { messages });
        }
        if !lifecycle_events.is_empty() {
            inputs.push(AIAgentInput::EventsFromAgents {
                events: lifecycle_events,
            });
        }
        inputs
    }

    /// Moves all awaiting events back to pending for retry after a failed
    /// send attempt. Increments attempt counts and drops events that have
    /// exhausted their retry limit.
    pub fn requeue_awaiting_events(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let events = self
            .awaiting_server_echo_events
            .remove(&conversation_id)
            .unwrap_or_default();
        if events.is_empty() {
            return;
        }

        let (retryable, exhausted) =
            increment_attempt_and_partition_by_retry_limit(events, MAX_RETRY_ATTEMPTS);

        if !exhausted.is_empty() {
            log::warn!(
                "Dropping {} orchestration events after exhausting retries",
                exhausted.len()
            );
        }

        if !retryable.is_empty() {
            let queue = self.pending_events.entry(conversation_id).or_default();
            let mut combined = retryable;
            combined.append(queue);
            *queue = combined;

            ctx.emit(OrchestrationEventServiceEvent::EventsReady { conversation_id });
        }
    }

    /// Scans the exchange output for orchestration IDs echoed back by the
    /// server, then clears matching entries from awaiting_server_echo_events.
    fn confirm_delivery_from_exchange(
        &mut self,
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        ctx: &ModelContext<Self>,
    ) {
        if !self
            .awaiting_server_echo_events
            .contains_key(&conversation_id)
        {
            return;
        }

        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return;
        };
        let Some(exchange) = conversation.exchange_with_id(exchange_id) else {
            return;
        };

        let mut echoed_message_ids = Vec::new();
        let mut echoed_lifecycle_event_ids = Vec::new();
        if let Some(output) = exchange.output_status.output() {
            for msg in &output.get().messages {
                match &msg.message {
                    AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } => {
                        for received in messages {
                            if !received.message_id.is_empty() {
                                echoed_message_ids.push(received.message_id.clone());
                            }
                        }
                    }
                    AIAgentOutputMessageType::EventsFromAgents { event_ids } => {
                        for id in event_ids {
                            if !id.is_empty() {
                                echoed_lifecycle_event_ids.push(id.clone());
                            }
                        }
                    }
                    AIAgentOutputMessageType::Text(_)
                    | AIAgentOutputMessageType::Reasoning { .. }
                    | AIAgentOutputMessageType::Summarization { .. }
                    | AIAgentOutputMessageType::Subagent(_)
                    | AIAgentOutputMessageType::Action(_)
                    | AIAgentOutputMessageType::TodoOperation(_)
                    | AIAgentOutputMessageType::WebSearch(_)
                    | AIAgentOutputMessageType::WebFetch(_)
                    | AIAgentOutputMessageType::CommentsAddressed { .. }
                    | AIAgentOutputMessageType::DebugOutput { .. }
                    | AIAgentOutputMessageType::ArtifactCreated(_)
                    | AIAgentOutputMessageType::SkillInvoked(_) => {}
                }
            }
        }

        if !echoed_message_ids.is_empty() || !echoed_lifecycle_event_ids.is_empty() {
            self.acknowledge_delivery_from_server_echo(
                conversation_id,
                &echoed_message_ids,
                &echoed_lifecycle_event_ids,
            );
        }
    }

    /// Clears awaiting_server_echo_events entries that match the given IDs.
    fn acknowledge_delivery_from_server_echo(
        &mut self,
        conversation_id: AIConversationId,
        echoed_message_ids: &[String],
        echoed_lifecycle_event_ids: &[String],
    ) {
        if echoed_message_ids.is_empty() && echoed_lifecycle_event_ids.is_empty() {
            return;
        }

        let echoed_message_ids: HashSet<&str> =
            echoed_message_ids.iter().map(String::as_str).collect();
        let echoed_lifecycle_event_ids: HashSet<&str> = echoed_lifecycle_event_ids
            .iter()
            .map(String::as_str)
            .collect();
        let should_remove_entry = {
            let Some(awaiting_events) = self.awaiting_server_echo_events.get_mut(&conversation_id)
            else {
                return;
            };

            awaiting_events.retain(|pending_event| {
                let was_echoed = did_event_round_trip_through_server(
                    pending_event,
                    &echoed_message_ids,
                    &echoed_lifecycle_event_ids,
                );
                !was_echoed
            });

            awaiting_events.is_empty()
        };

        if should_remove_entry {
            self.awaiting_server_echo_events.remove(&conversation_id);
        }
    }
}

/// `None` means \"subscribe to all lifecycle types\" (input omitted).
/// `Some([])` means subscribe to no lifecycle events.
fn is_subscribed(
    subscription: Option<&[LifecycleEventType]>,
    event_type: LifecycleEventType,
) -> bool {
    match subscription {
        None => true,
        Some(subscription) => subscription.contains(&event_type),
    }
}

fn did_event_round_trip_through_server(
    pending_event: &PendingEvent,
    echoed_message_ids: &HashSet<&str>,
    echoed_lifecycle_event_ids: &HashSet<&str>,
) -> bool {
    match &pending_event.detail {
        PendingEventDetail::Message { message_id, .. } => {
            echoed_message_ids.contains(message_id.as_str())
        }
        PendingEventDetail::Lifecycle { event } => {
            echoed_lifecycle_event_ids.contains(event.event_id.as_str())
        }
    }
}

#[allow(deprecated)]
pub(super) fn lifecycle_event_type_name(event_type: LifecycleEventType) -> &'static str {
    match event_type {
        LifecycleEventType::Started => "started",
        LifecycleEventType::Idle => "idle",
        LifecycleEventType::Restarted => "restarted",
        LifecycleEventType::InProgress => "in_progress",
        LifecycleEventType::Succeeded => "succeeded",
        LifecycleEventType::Failed => "failed",
        LifecycleEventType::Errored => "errored",
        LifecycleEventType::Cancelled => "cancelled",
        LifecycleEventType::Blocked => "blocked",
        LifecycleEventType::Unspecified => "unspecified",
    }
}

pub(super) fn build_lifecycle_event(
    event_id: String,
    sender_agent_id: String,
    event_type: LifecycleEventType,
    occurred_at: prost_types::Timestamp,
    detail_payload: &LifecycleEventDetailPayload,
) -> api::AgentEvent {
    // Build the API envelope that is forwarded to recipients and stored in memory.
    // This keeps `occurred_at` attached
    // to the event itself (not inferred at formatting time).
    let detail = lifecycle_event_detail_from_type(event_type, detail_payload);
    api::AgentEvent {
        event_id,
        occurred_at: Some(occurred_at),
        event: Some(api::agent_event::Event::LifecycleEvent(
            api::agent_event::LifecycleEvent {
                sender_agent_id,
                detail,
            },
        )),
    }
}

#[allow(deprecated)]
fn lifecycle_event_detail_from_type(
    event_type: LifecycleEventType,
    detail_payload: &LifecycleEventDetailPayload,
) -> Option<api::agent_event::lifecycle_event::Detail> {
    match event_type {
        LifecycleEventType::InProgress => {
            Some(api::agent_event::lifecycle_event::Detail::InProgress(()))
        }
        LifecycleEventType::Succeeded => {
            Some(api::agent_event::lifecycle_event::Detail::Succeeded(()))
        }
        LifecycleEventType::Failed => Some(api::agent_event::lifecycle_event::Detail::Failed(
            api::agent_event::lifecycle_event::Failed {
                reason: detail_payload.reason.clone().unwrap_or_default(),
                error_message: detail_payload.error_message.clone().unwrap_or_default(),
            },
        )),
        // Legacy variants delegate to their new equivalents.
        LifecycleEventType::Started => {
            Some(api::agent_event::lifecycle_event::Detail::InProgress(()))
        }
        LifecycleEventType::Idle => Some(api::agent_event::lifecycle_event::Detail::Succeeded(())),
        LifecycleEventType::Restarted => {
            Some(api::agent_event::lifecycle_event::Detail::InProgress(()))
        }
        LifecycleEventType::Cancelled => {
            Some(api::agent_event::lifecycle_event::Detail::Cancelled(()))
        }
        LifecycleEventType::Blocked => Some(api::agent_event::lifecycle_event::Detail::Blocked(
            api::agent_event::lifecycle_event::Blocked {
                blocked_action: detail_payload.blocked_action.clone().unwrap_or_default(),
            },
        )),
        LifecycleEventType::Errored => Some(api::agent_event::lifecycle_event::Detail::Errored(
            api::agent_event::lifecycle_event::Errored {
                stage: detail_payload
                    .stage
                    .map(|stage| stage.as_str().to_string())
                    .unwrap_or_default(),
                reason: detail_payload.reason.clone().unwrap_or_default(),
                error_message: detail_payload.error_message.clone().unwrap_or_default(),
            },
        )),
        LifecycleEventType::Unspecified => None,
    }
}

#[allow(deprecated)]
pub(super) fn lifecycle_event_type_from_proto(
    lifecycle_event: &api::agent_event::LifecycleEvent,
) -> api::LifecycleEventType {
    match lifecycle_event.detail.as_ref() {
        Some(api::agent_event::lifecycle_event::Detail::InProgress(_)) => {
            api::LifecycleEventType::InProgress
        }
        Some(api::agent_event::lifecycle_event::Detail::Succeeded(_)) => {
            api::LifecycleEventType::Succeeded
        }
        Some(api::agent_event::lifecycle_event::Detail::Failed(_)) => {
            api::LifecycleEventType::Failed
        }
        // Legacy detail variants map to new types.
        Some(api::agent_event::lifecycle_event::Detail::Started(_)) => {
            api::LifecycleEventType::InProgress
        }
        Some(api::agent_event::lifecycle_event::Detail::Idle(_)) => {
            api::LifecycleEventType::Succeeded
        }
        Some(api::agent_event::lifecycle_event::Detail::Restarted(_)) => {
            api::LifecycleEventType::InProgress
        }
        Some(api::agent_event::lifecycle_event::Detail::Cancelled(_)) => {
            api::LifecycleEventType::Cancelled
        }
        Some(api::agent_event::lifecycle_event::Detail::Blocked(_)) => {
            api::LifecycleEventType::Blocked
        }
        Some(api::agent_event::lifecycle_event::Detail::Errored(_)) => {
            api::LifecycleEventType::Errored
        }
        None => api::LifecycleEventType::Unspecified,
    }
}

/// True when a pending event is a lifecycle succeeded/in_progress event and
/// therefore eligible to be superseded by a newer lifecycle transition from
/// the same sender.
fn is_coalescable_lifecycle_pending_event(event: &PendingEvent) -> bool {
    let PendingEventDetail::Lifecycle { event: agent_event } = &event.detail else {
        return false;
    };
    let Some(api::agent_event::Event::LifecycleEvent(lifecycle_event)) = &agent_event.event else {
        return false;
    };
    matches!(
        lifecycle_event_type_from_proto(lifecycle_event),
        api::LifecycleEventType::Succeeded | api::LifecycleEventType::InProgress
    )
}

fn coalesce_lifecycle_events(
    queue: &mut Vec<PendingEvent>,
    new_event: &PendingEvent,
) -> Vec<String> {
    // Remove older supersedable lifecycle events for the same sender as `new_event`.
    // Returns the removed event IDs for callers that need observability/debug assertions.
    // Only coalesce supersedable lifecycle states (succeeded/in_progress). Critical states
    // like errored/cancelled/blocked are retained so recipients don't lose important
    // transitions.
    let PendingEventDetail::Lifecycle {
        event: new_agent_event,
    } = &new_event.detail
    else {
        return vec![];
    };
    let Some(api::agent_event::Event::LifecycleEvent(new_lifecycle_event)) = &new_agent_event.event
    else {
        return vec![];
    };
    let new_type = lifecycle_event_type_from_proto(new_lifecycle_event);
    if !matches!(
        new_type,
        api::LifecycleEventType::Succeeded | api::LifecycleEventType::InProgress
    ) {
        return vec![];
    }

    let mut removed_event_ids = Vec::new();
    queue.retain(|existing| {
        let PendingEventDetail::Lifecycle {
            event: existing_agent_event,
        } = &existing.detail
        else {
            return true;
        };
        let Some(api::agent_event::Event::LifecycleEvent(existing_lifecycle_event)) =
            &existing_agent_event.event
        else {
            return true;
        };
        let existing_type = lifecycle_event_type_from_proto(existing_lifecycle_event);
        let should_remove = existing_lifecycle_event.sender_agent_id
            == new_lifecycle_event.sender_agent_id
            && matches!(
                existing_type,
                api::LifecycleEventType::Succeeded | api::LifecycleEventType::InProgress
            );
        if should_remove {
            removed_event_ids.push(existing.event_id.clone());
        }
        !should_remove
    });
    removed_event_ids
}

/// Enforce an upper bound on pending lifecycle events while preferentially dropping
/// supersedable lifecycle states first, preserving critical transitions.
fn enforce_lifecycle_queue_cap(
    queue: &mut Vec<PendingEvent>,
    max_pending_lifecycle_events: usize,
) -> Vec<String> {
    let mut dropped_event_ids = Vec::new();
    while count_pending_lifecycle_events(queue) > max_pending_lifecycle_events {
        if let Some(index) = queue
            .iter()
            .position(is_coalescable_lifecycle_pending_event)
        {
            dropped_event_ids.push(queue.remove(index).event_id);
        } else {
            // No coalescable items remain; keep critical events.
            break;
        }
    }
    dropped_event_ids
}

/// Count lifecycle entries in a mixed pending queue (message + lifecycle).
fn count_pending_lifecycle_events(queue: &[PendingEvent]) -> usize {
    queue
        .iter()
        .filter(|event| matches!(event.detail, PendingEventDetail::Lifecycle { .. }))
        .count()
}

/// Increment attempt counts and split events into retryable vs exhausted buckets.
/// Exhaustion is based on `max_retry_attempts` after incrementing this attempt.
fn increment_attempt_and_partition_by_retry_limit(
    mut attempted_events: Vec<PendingEvent>,
    max_retry_attempts: i32,
) -> (Vec<PendingEvent>, Vec<PendingEvent>) {
    for event in &mut attempted_events {
        event.attempt_count += 1;
    }
    attempted_events
        .into_iter()
        .partition(|event| event.attempt_count < max_retry_attempts)
}

impl Default for OrchestrationEventService {
    fn default() -> Self {
        Self::new_without_subscriptions()
    }
}

impl Entity for OrchestrationEventService {
    type Event = OrchestrationEventServiceEvent;
}

impl SingletonEntity for OrchestrationEventService {}

#[cfg(test)]
#[path = "orchestration_events_tests.rs"]
mod tests;
