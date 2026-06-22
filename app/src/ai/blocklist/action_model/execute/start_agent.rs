use std::collections::HashMap;

use futures::future::BoxFuture;
use futures::FutureExt;
use shell_words::split as split_shell_words;
use warp_cli::agent::Harness;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, LifecycleEventType,
    StartAgentExecutionMode, StartAgentResult,
};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::local_harness_setup::local_harness_product_disabled_message;

/// Per-request outcome of a StartAgent dispatch.
#[derive(Debug, Clone)]
pub enum StartAgentOutcome {
    Started {
        agent_id: String,
    },
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

/// Handles local child launch requests produced by older agents, where the
/// prompt encoded the target CLI command and `execution_mode.harness_type` was
/// still unset. Normalizing here keeps those requests routed through the Codex
/// local harness path instead of launching them as Oz child prompts.
fn parse_legacy_local_child_harness_command(command: &str) -> Option<(String, String)> {
    let args = split_shell_words(command.trim()).ok()?;
    match args.as_slice() {
        [binary, flag, child_prompt]
            if binary == "codex"
                && flag == "--dangerously-bypass-approvals-and-sandbox"
                && !child_prompt.trim().is_empty() =>
        {
            Some(("codex".to_string(), child_prompt.clone()))
        }
        _ => None,
    }
}

fn normalize_legacy_local_child_harness_command(
    prompt: String,
    execution_mode: StartAgentExecutionMode,
) -> (String, StartAgentExecutionMode) {
    match execution_mode {
        StartAgentExecutionMode::Local {
            harness_type: None,
            model_id,
        } => {
            if let Some((harness_type, child_prompt)) =
                parse_legacy_local_child_harness_command(&prompt)
            {
                (
                    child_prompt,
                    StartAgentExecutionMode::Local {
                        harness_type: Some(harness_type),
                        model_id,
                    },
                )
            } else {
                (
                    prompt,
                    StartAgentExecutionMode::Local {
                        harness_type: None,
                        model_id,
                    },
                )
            }
        }
        execution_mode => (prompt, execution_mode),
    }
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
                log::error!(
                    "No agent identifier found for child conversation {child_conversation_id:?}"
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
            conversation.status_error_message(),
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
                    conversation.status_error_message(),
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
            | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::RemoveConversation { .. }
            | BlocklistAIHistoryEvent::DeletedConversation { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationTitle { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
            | BlocklistAIHistoryEvent::ConversationOwnershipTransferred { .. } => {}
            BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => {}
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
        let (prompt, execution_mode) =
            normalize_legacy_local_child_harness_command(prompt, execution_mode.clone());
        let (execution_mode, parent_run_id) = match execution_mode {
            StartAgentExecutionMode::Local {
                harness_type: None,
                model_id,
            } => {
                // Oz local children resolve their parent's run id from the
                // parent conversation. This mirrors the third-party-harness
                // and remote-child branches below; the child task row is
                // created eagerly at dispatch (see
                // `launch_local_no_harness_child`) using this value as the
                // `parent_run_id` on `CreateAgentTask`. Bail out if the
                // parent has no `run_id` yet — the eager-create path has no
                // late-binding fallback (the pre-change lazy path would have
                // linked via `Request.metadata.parent_agent_id` later), so
                // proceeding would mint an orphan child with no server-side
                // parent linkage.
                let parent_run_id = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&parent_conversation_id)
                    .and_then(|conversation| conversation.run_id());
                let Some(parent_run_id) = parent_run_id else {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error:
                                "Local Oz child agents require the parent run_id to be available."
                                    .to_string(),
                            version,
                        },
                    ));
                };
                (
                    StartAgentExecutionMode::Local {
                        harness_type: None,
                        model_id,
                    },
                    Some(parent_run_id),
                )
            }
            StartAgentExecutionMode::Local {
                harness_type: Some(harness_type),
                model_id,
            } => {
                let Some(harness) = Harness::parse_local_child_harness(&harness_type) else {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error: invalid_local_child_harness_error(&harness_type),
                            version,
                        },
                    ));
                };
                if let Some(message) = local_harness_product_disabled_message(harness) {
                    return ActionExecution::Sync(AIAgentActionResultType::StartAgent(
                        StartAgentResult::Error {
                            error: message.to_string(),
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
                        model_id,
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
                auth_secret_name,
            } => {
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
                        auth_secret_name,
                    },
                    Some(parent_run_id),
                )
            }
        };

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
                name: name.clone(),
                prompt,
                execution_mode,
                lifecycle_subscription: lifecycle_subscription.clone(),
                parent_conversation_id,
                parent_run_id,
            },
        )));

        ActionExecution::new_async(async move { receiver.recv().await }, move |result, _ctx| {
            match result {
                Ok(StartAgentOutcome::Started { agent_id }) => {
                    AIAgentActionResultType::StartAgent(StartAgentResult::Success {
                        agent_id,
                        version,
                    })
                }
                Ok(StartAgentOutcome::Error(error)) => {
                    AIAgentActionResultType::StartAgent(StartAgentResult::Error { error, version })
                }
                Err(_) => {
                    AIAgentActionResultType::StartAgent(StartAgentResult::Cancelled { version })
                }
            }
        })
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
        let (prompt, execution_mode) =
            normalize_legacy_local_child_harness_command(prompt, execution_mode);
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

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
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

#[cfg(test)]
#[path = "start_agent_tests.rs"]
mod tests;
