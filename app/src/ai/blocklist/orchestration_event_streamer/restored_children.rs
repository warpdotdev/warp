use std::time::Duration;

use anyhow::Context as _;
use warp_errors::report_error;
use warp_graphql::ai::AgentTaskState;
use warpui::AppContext;
use warpui::r#async::Timer;

use super::{
    AgentEventDriverConfig, BlocklistAIHistoryModel, DormantClaudeWakeConsumer, Harness,
    LocalChildHistoryInstallation, LocalChildRestorationTarget, ModelContext,
    OrchestrationEventStreamer, OrchestrationEventStreamerEvent, ServerApiAgentEventSource,
    SingletonEntity, WakeConnectionState, run_agent_event_driver,
};
use crate::ai::agent::RenderableAIError;
use crate::ai::agent::api::convert_conversation::{
    RestorationMode, convert_conversation_data_to_ai_conversation,
};
use crate::ai::agent::conversation::{
    AIConversation, AIConversationId, ConversationStatus, LocalChildRestorationStatus,
};
use crate::server::server_api::ai::TaskStatusUpdate;

const LOCAL_CHILD_HISTORY_INSTALLATION_TIMEOUT: Duration = Duration::from_secs(5);

impl OrchestrationEventStreamer {
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn register_restored_local_child(
        &mut self,
        conversation_id: AIConversationId,
        target: LocalChildRestorationTarget,
        ctx: &mut ModelContext<Self>,
    ) {
        let stream = self.streams.entry(conversation_id).or_default();
        let previous = stream.local_child_restoration_target.replace(target);
        if let Some(previous) = previous.filter(|previous| *previous != target) {
            stream.consumers.remove(&previous.terminal_id);
            stream.local_child_restoration_generation = None;
            if self.local_child_restoration_status(conversation_id, ctx)
                == Some(LocalChildRestorationStatus::Restoring)
            {
                self.fail_local_child_restoration(
                    conversation_id,
                    "install transcript: owner replaced".into(),
                    ctx,
                );
            }
        }
        self.register_consumer(conversation_id, target.terminal_id, ctx);
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn release_local_subtree_startup(
        &mut self,
        conversation_ids: &[AIConversationId],
        ctx: &mut ModelContext<Self>,
    ) {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            for conversation_id in conversation_ids {
                let surface = history.terminal_surface_id_for_conversation(conversation_id);
                let Some(child) = history.conversation_mut(conversation_id) else {
                    continue;
                };
                let was_ready = child.is_local_child_execution_ready();
                child.release_local_subtree_startup();
                if !was_ready
                    && child.is_local_child_execution_ready()
                    && let Some(surface) = surface
                {
                    child.update_status_with_error(
                        child.status().clone(),
                        child.status_error().cloned(),
                        surface,
                        ctx,
                    );
                }
            }
        });
        for conversation_id in conversation_ids {
            self.reevaluate_eligibility(*conversation_id, ctx);
        }
    }
    pub(crate) fn local_child_restoration_status(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<LocalChildRestorationStatus> {
        BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(AIConversation::local_child_restoration_status)
    }

    pub(super) fn reevaluate_local_child_restoration(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        match self.local_child_restoration_status(conversation_id, ctx) {
            None | Some(LocalChildRestorationStatus::Restored) => false,
            Some(LocalChildRestorationStatus::Pending) => {
                self.teardown_sse(conversation_id, ctx);
                let stream = self.streams.entry(conversation_id).or_default();
                if stream.consumers.is_empty() {
                    self.teardown_dormant_claude_wake_listener(conversation_id);
                } else if stream.local_child_restoration_target.is_some()
                    && stream.wake_connection.is_none()
                {
                    self.start_local_child_wake_listener(conversation_id, ctx);
                }
                true
            }
            Some(LocalChildRestorationStatus::Restoring | LocalChildRestorationStatus::Error) => {
                self.teardown_sse(conversation_id, ctx);
                self.teardown_dormant_claude_wake_listener(conversation_id);
                true
            }
        }
    }

    fn start_local_child_wake_listener(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return;
        };
        let Some(run_id) = conversation.run_id() else {
            return;
        };
        let cursor = conversation.last_event_sequence().unwrap_or(0);
        let stream = self.streams.entry(conversation_id).or_default();
        stream.event_cursor = cursor;
        stream.harness = Some(Harness::Oz);
        let generation = self.next_wake_generation;
        self.next_wake_generation += 1;
        let source = ServerApiAgentEventSource::new(self.server_api.clone());
        let task = ctx.spawn(
            async move {
                let config =
                    AgentEventDriverConfig::retry_forever_run_ids(vec![run_id.clone()], cursor);
                let mut consumer = DormantClaudeWakeConsumer::new(run_id);
                run_agent_event_driver(source, config, &mut consumer).await?;
                consumer
                    .wake_message
                    .context("child wake listener ended without a message")?;
                Ok::<_, anyhow::Error>(())
            },
            move |me, result, ctx| {
                if me
                    .streams
                    .get(&conversation_id)
                    .and_then(|stream| stream.wake_connection.as_ref())
                    .is_none_or(|connection| connection.generation != generation)
                {
                    return;
                }
                if let Some(stream) = me.streams.get_mut(&conversation_id) {
                    stream.wake_connection = None;
                }
                match result {
                    Ok(()) => {
                        me.restore_local_child(conversation_id, ctx);
                    }
                    Err(error) => me.fail_local_child_restoration(
                        conversation_id,
                        format!("wake listener: {error:#}"),
                        ctx,
                    ),
                }
            },
        );
        self.streams
            .entry(conversation_id)
            .or_default()
            .wake_connection = Some(WakeConnectionState { generation, task });
    }

    pub(crate) fn restore_local_child(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        match self.local_child_restoration_status(conversation_id, ctx) {
            None | Some(LocalChildRestorationStatus::Restored) => return false,
            Some(LocalChildRestorationStatus::Restoring | LocalChildRestorationStatus::Error) => {
                return true;
            }
            Some(LocalChildRestorationStatus::Pending) => {}
        }
        let Some(stub) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .cloned()
        else {
            return true;
        };
        let Some(token) = stub.server_conversation_token().cloned() else {
            self.fail_local_child_restoration(
                conversation_id,
                "missing conversation token".into(),
                ctx,
            );
            return true;
        };
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                conversation
                    .set_local_child_restoration_status(LocalChildRestorationStatus::Restoring);
            }
        });
        self.reevaluate_eligibility(conversation_id, ctx);
        let client = self.ai_client.clone();
        let generation = uuid::Uuid::new_v4();
        self.streams
            .entry(conversation_id)
            .or_default()
            .local_child_restoration_generation = Some(generation);
        ctx.spawn(
            async move {
                let (data, metadata) = client
                    .get_ai_conversation(token.clone())
                    .await
                    .context("fetch/decode transcript")?;
                anyhow::ensure!(
                    metadata.server_conversation_token == token,
                    "transcript token mismatch"
                );
                anyhow::ensure!(
                    metadata.harness == crate::ai::agent::conversation::AIAgentHarness::Oz,
                    "transcript harness is not Oz"
                );
                let mut hydrated = convert_conversation_data_to_ai_conversation(
                    conversation_id,
                    &data,
                    metadata,
                    RestorationMode::Continue,
                )
                .context("convert transcript")?;
                hydrated.set_task_id(stub.task_id().context("missing child run ID")?);
                hydrated.set_parent_agent_id(
                    stub.parent_agent_id()
                        .context("missing parent run ID")?
                        .to_string(),
                );
                hydrated.set_parent_conversation_id(
                    stub.parent_conversation_id()
                        .context("missing parent conversation")?,
                );
                hydrated.set_agent_name(stub.agent_name().unwrap_or("Agent").to_string());
                if let Some(title) = stub.title() {
                    hydrated.set_fallback_display_title(title);
                }
                if hydrated.autoexecute_override() != stub.autoexecute_override() {
                    hydrated.toggle_autoexecute_override();
                }
                hydrated.set_orchestration_harness(Harness::Oz);
                hydrated.set_last_event_sequence(stub.last_event_sequence().unwrap_or(0));
                hydrated.set_local_child_restoration_status(LocalChildRestorationStatus::Restoring);
                Ok::<_, anyhow::Error>(hydrated)
            },
            move |me, result, ctx| {
                if me.local_child_restoration_status(conversation_id, ctx)
                    != Some(LocalChildRestorationStatus::Restoring)
                    || me
                        .streams
                        .get(&conversation_id)
                        .and_then(|stream| stream.local_child_restoration_generation)
                        != Some(generation)
                {
                    return;
                }
                let result = result.and_then(|conversation| {
                    let target = me
                        .streams
                        .get(&conversation_id)
                        .and_then(|stream| stream.local_child_restoration_target)
                        .context("install transcript: missing owner")?;
                    let installation = LocalChildHistoryInstallation {
                        conversation_id,
                        generation,
                        target,
                    };
                    ctx.emit(OrchestrationEventStreamerEvent::LocalChildHistoryPrepared {
                        installation,
                        conversation: Box::new(conversation),
                    });
                    ctx.spawn(
                        async { Timer::after(LOCAL_CHILD_HISTORY_INSTALLATION_TIMEOUT).await },
                        move |me, _, ctx| {
                            if me.is_current_local_child_history_installation(installation, ctx) {
                                me.fail_local_child_restoration(
                                    conversation_id,
                                    "install transcript: installation was not acknowledged".into(),
                                    ctx,
                                );
                            }
                        },
                    );
                    Ok(())
                });
                if let Err(error) = result {
                    me.fail_local_child_restoration(conversation_id, format!("{error:#}"), ctx);
                }
            },
        );
        true
    }

    pub(crate) fn is_current_local_child_history_installation(
        &self,
        installation: LocalChildHistoryInstallation,
        ctx: &AppContext,
    ) -> bool {
        self.local_child_restoration_status(installation.conversation_id, ctx)
            == Some(LocalChildRestorationStatus::Restoring)
            && self
                .streams
                .get(&installation.conversation_id)
                .is_some_and(|stream| {
                    stream.local_child_restoration_generation == Some(installation.generation)
                        && stream.local_child_restoration_target == Some(installation.target)
                })
    }
    pub(crate) fn acknowledge_local_child_history_installation(
        &mut self,
        installation: LocalChildHistoryInstallation,
        result: anyhow::Result<()>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.is_current_local_child_history_installation(installation, ctx) {
            return;
        }
        let conversation_id = installation.conversation_id;
        let result = result.and_then(|()| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            anyhow::ensure!(
                history.terminal_surface_id_for_conversation(&conversation_id)
                    == Some(installation.target.terminal_id),
                "install transcript: terminal binding changed"
            );
            anyhow::ensure!(
                history
                    .conversation(&conversation_id)
                    .is_some_and(|conversation| { conversation.driver().owns_turn_lifecycle() }),
                "install transcript: execution conversation missing"
            );
            Ok(())
        });
        match result {
            Ok(()) => {
                self.streams
                    .get_mut(&conversation_id)
                    .unwrap()
                    .local_child_restoration_generation = None;
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history
                        .conversation_mut(&conversation_id)
                        .unwrap()
                        .set_local_child_restoration_status(LocalChildRestorationStatus::Restored);
                    history.update_conversation_status(
                        installation.target.terminal_id,
                        conversation_id,
                        ConversationStatus::Success,
                        ctx,
                    );
                });
                self.reevaluate_eligibility(conversation_id, ctx);
            }
            Err(error) => self.fail_local_child_restoration(
                conversation_id,
                format!("install transcript: {error:#}"),
                ctx,
            ),
        }
    }

    pub(crate) fn fail_local_child_restoration(
        &mut self,
        conversation_id: AIConversationId,
        stage: String,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(stream) = self.streams.get_mut(&conversation_id) {
            stream.local_child_restoration_generation = None;
        }
        let Some(task_id) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(AIConversation::task_id)
        else {
            report_error!("Cannot report restored child failure: missing run ID",
                extra: { "conversation_id" => %conversation_id });
            return;
        };
        let message = format!("Local child {task_id} restoration failed: {stage}");
        let surface = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_surface_id_for_conversation(&conversation_id)
            .or_else(|| {
                // Detachment must not hide the diagnostic; this ID only attributes the error event.
                self.streams
                    .get(&conversation_id)
                    .and_then(|stream| stream.local_child_restoration_target)
                    .map(|target| target.terminal_id)
            });
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                conversation.set_local_child_restoration_status(LocalChildRestorationStatus::Error);
                if let Some(surface) = surface {
                    conversation.update_status_with_error(
                        ConversationStatus::Error,
                        Some(RenderableAIError::other(message.clone(), false)),
                        surface,
                        ctx,
                    );
                }
            }
        });
        self.reevaluate_eligibility(conversation_id, ctx);
        let client = self.ai_client.clone();
        ctx.spawn(
            async move {
                client
                    .update_agent_task(
                        task_id,
                        Some(AgentTaskState::Error),
                        None,
                        None,
                        Some(TaskStatusUpdate::message(message)),
                        None,
                        None,
                    )
                    .await
            },
            move |_, result, _| {
                if let Err(error) = result {
                    report_error!(error.context("Failed to report restored child Error"),
                        extra: { "child_run_id" => %task_id });
                }
            },
        );
    }
}
