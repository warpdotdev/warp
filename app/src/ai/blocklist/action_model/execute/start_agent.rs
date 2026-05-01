use futures::{future::BoxFuture, FutureExt};
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, LifecycleEventType,
    StartAgentExecutionMode, StartAgentResult,
};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::orchestration_events::OrchestrationEventService;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};

/// The result sent back to the executor after observing the child agent's lifecycle.
enum StartAgentDecision {
    /// The child conversation was created successfully.
    Started { agent_id: String },
    /// An error occurred while starting the agent.
    Error(String),
}

fn invalid_local_child_harness_error(harness_type: &str) -> String {
    let harness_name = harness_type.trim();
    if harness_name.is_empty() {
        "Local child harness type is missing.".to_string()
    } else {
        format!("Unsupported local child harness '{harness_name}'.")
    }
}

/// Groups the data for a single StartAgent invocation as it flows from the
/// executor through the terminal view and pane group into the controller.
#[derive(Clone)]
pub struct StartAgentRequest {
    pub name: String,
    pub prompt: String,
    pub execution_mode: StartAgentExecutionMode,
    pub lifecycle_subscription: Option<Vec<LifecycleEventType>>,
    pub parent_conversation_id: AIConversationId,
    pub parent_run_id: Option<String>,
}

/// Tracks a single in-flight StartAgent action. At most one can be pending at
/// a time because StartAgent actions execute serially (RunningActionPhase::Serial).
struct PendingStartAgent {
    parent_conversation_id: AIConversationId,
    /// Set when `StartedNewConversation` fires for a conversation whose
    /// `parent_conversation_id` matches.
    child_conversation_id: Option<AIConversationId>,
    sender: async_channel::Sender<StartAgentDecision>,
}

pub struct StartAgentExecutor {
    /// The currently pending StartAgent action, if any.
    pending: Option<PendingStartAgent>,
}

impl StartAgentExecutor {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, Self::handle_history_event);

        Self { pending: None }
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::StartedNewConversation {
                new_conversation_id,
                ..
            } => {
                let Some(pending) = self.pending.as_mut() else {
                    return;
                };
                if pending.child_conversation_id.is_some() {
                    return;
                }
                let history = BlocklistAIHistoryModel::as_ref(ctx);
                let Some(conversation) = history.conversation(new_conversation_id) else {
                    return;
                };
                if conversation.parent_conversation_id() == Some(pending.parent_conversation_id) {
                    pending.child_conversation_id = Some(*new_conversation_id);
                }
            }
            BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id, ..
            } => {
                let matches = self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.child_conversation_id.as_ref() == Some(conversation_id));
                if !matches {
                    return;
                }
                let pending = self.pending.take().unwrap();
                let agent_id = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(conversation_id)
                    .and_then(|c| c.orchestration_agent_id());
                match agent_id {
                    Some(id) => {
                        let _ = pending.sender.try_send(StartAgentDecision::Started {
                            agent_id: id.clone(),
                        });
                        if FeatureFlag::OrchestrationV2.is_enabled() {
                            OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
                                streamer.register_watched_run_id(
                                    pending.parent_conversation_id,
                                    id,
                                    ctx,
                                );
                            });
                        } else {
                            OrchestrationEventService::handle(ctx).update(ctx, |svc, ctx| {
                                svc.emit_child_startup_started(*conversation_id, ctx);
                            });
                        }
                    }
                    None => {
                        log::error!(
                            "ConversationServerTokenAssigned fired but no agent identifier for \
                             {conversation_id:?}"
                        );
                        let _ = pending.sender.try_send(StartAgentDecision::Error(
                            "Server did not assign an agent identifier".to_string(),
                        ));
                        if !FeatureFlag::OrchestrationV2.is_enabled() {
                            OrchestrationEventService::handle(ctx).update(ctx, |svc, ctx| {
                                svc.emit_child_startup_errored(
                                    *conversation_id,
                                    "missing_agent_id".to_string(),
                                    "Server did not assign an agent identifier".to_string(),
                                    ctx,
                                );
                            });
                        }
                    }
                }
            }
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            } => {
                let matches = self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.child_conversation_id.as_ref() == Some(conversation_id));
                if !matches {
                    return;
                }
                let history = BlocklistAIHistoryModel::as_ref(ctx);
                let Some(conversation) = history.conversation(conversation_id) else {
                    return;
                };
                let error_msg = start_agent_error_message_for_status(
                    conversation.status(),
                    conversation.status_error_message(),
                );
                if let Some(error_msg) = error_msg {
                    let pending = self.pending.take().unwrap();
                    let _ = pending
                        .sender
                        .try_send(StartAgentDecision::Error(error_msg.clone()));
                    if !FeatureFlag::OrchestrationV2.is_enabled() {
                        OrchestrationEventService::handle(ctx).update(ctx, |svc, ctx| {
                            svc.emit_child_startup_errored(
                                *conversation_id,
                                "conversation_status".to_string(),
                                error_msg,
                                ctx,
                            );
                        });
                    }
                }
            }
            BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::ReassignedExchange { .. }
            | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::RemoveConversation { .. }
            | BlocklistAIHistoryEvent::DeletedConversation { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. } => {}
        }
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        // TODO(QUALITY-342): this should be a setting
        true
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let AIAgentAction {
            action:
                AIAgentActionType::StartAgent {
                    version,
                    name,
                    prompt,
                    execution_mode,
                    lifecycle_subscription,
                },
            ..
        } = input.action
        else {
            return ActionExecution::InvalidAction;
        };

        let prompt = prompt.clone();
        let version = *version;
        let parent_conversation_id = input.conversation_id;
        let (execution_mode, parent_run_id) = match execution_mode.clone() {
            StartAgentExecutionMode::Local { harness_type: None } => {
                // Legacy local Oz child agents do not use
                // StartAgentRequest.parent_run_id. Instead, the child
                // conversation is linked back to its parent on the first
                // request via Request.metadata.parent_agent_id, sourced
                // from the conversation's versioned orchestration_agent_id()
                // (run_id in v2, server conversation token in v1). Remote
                // child agents and local third-party harness children need
                // parent_run_id here because their run is spawned before that
                // first child request exists.
                (StartAgentExecutionMode::Local { harness_type: None }, None)
            }
            StartAgentExecutionMode::Local {
                harness_type: Some(harness_type),
            } => {
                let Some(harness) = Harness::parse_local_child_harness(&harness_type) else {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error: invalid_local_child_harness_error(&harness_type),
                            version,
                        },
                    ));
                };

                if !FeatureFlag::OrchestrationV2.is_enabled() {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error: "Local harness child agents require orchestration v2."
                                .to_string(),
                            version,
                        },
                    ));
                }

                let parent_run_id = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&parent_conversation_id)
                    .and_then(|conversation| conversation.run_id());
                let Some(parent_run_id) = parent_run_id else {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error:
                                "Local harness child agents require the parent run_id to be available."
                                    .to_string(),
                            version,
                        },
                    ));
                };

                (
                    StartAgentExecutionMode::Local {
                        harness_type: Some(harness.to_string()),
                    },
                    Some(parent_run_id),
                )
            }
            StartAgentExecutionMode::Remote {
                environment_id,
                skill_references,
                model_id,
                computer_use_enabled,
                worker_host,
                harness_type,
                title,
            } => {
                if !FeatureFlag::OrchestrationV2.is_enabled() {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error: "Remote child agents require orchestration v2.".to_string(),
                            version,
                        },
                    ));
                }

                let harness_type = Harness::parse_orchestration_harness(&harness_type)
                    .map(|harness| harness.to_string())
                    .unwrap_or(harness_type);
                if Harness::parse_orchestration_harness(&harness_type) == Some(Harness::OpenCode) {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error: "Remote child agents do not support the opencode harness yet."
                                .to_string(),
                            version,
                        },
                    ));
                }

                // An empty environment_id is allowed and means the child will be spawned with an
                // empty environment (no preconfigured repositories, secrets, or integrations).
                // Callers are discouraged from relying on this, but we intentionally do not reject
                // it here so that agent authors can opt into running without an environment.
                if environment_id.trim().is_empty() {
                    log::warn!(
                        "Starting remote child agent with empty environment_id; the child will run \
                         with an empty environment."
                    );
                }

                let parent_run_id = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&parent_conversation_id)
                    .and_then(|conversation| conversation.run_id());
                let Some(parent_run_id) = parent_run_id else {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error: "Remote child agents require the parent run_id to be available."
                                .to_string(),
                            version,
                        },
                    ));
                };

                (
                    StartAgentExecutionMode::Remote {
                        environment_id,
                        skill_references,
                        model_id,
                        computer_use_enabled,
                        worker_host,
                        harness_type,
                        title,
                    },
                    Some(parent_run_id),
                )
            }
        };

        let (sender, receiver) = async_channel::bounded(1);
        self.pending = Some(PendingStartAgent {
            parent_conversation_id,
            child_conversation_id: None,
            sender,
        });

        ctx.emit(StartAgentExecutorEvent::CreateAgent(StartAgentRequest {
            name: name.clone(),
            prompt,
            execution_mode,
            lifecycle_subscription: lifecycle_subscription.clone(),
            parent_conversation_id,
            parent_run_id,
        }));

        ActionExecution::new_async(async move { receiver.recv().await }, move |result, _ctx| {
            match result {
                Ok(StartAgentDecision::Started { agent_id }) => {
                    AIAgentActionResultType::StartAgent(StartAgentResult::Success {
                        agent_id,
                        version,
                    })
                }
                Ok(StartAgentDecision::Error(error)) => {
                    AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                }
                Err(_) => {
                    AIAgentActionResultType::StartAgent(StartAgentResult::Cancelled { version })
                }
            }
        })
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
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
        ConversationStatus::InProgress | ConversationStatus::Success => None,
    }
}

impl Entity for StartAgentExecutor {
    type Event = StartAgentExecutorEvent;
}

pub enum StartAgentExecutorEvent {
    CreateAgent(StartAgentRequest),
}

#[cfg(test)]
#[path = "start_agent_tests.rs"]
mod tests;
