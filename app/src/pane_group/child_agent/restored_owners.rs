use std::collections::HashMap;
use std::ffi::OsString;

use anyhow::Context as _;
use warp_cli::agent::Harness;
use warpui::{EntityId, ModelHandle, SingletonEntity, ViewContext, ViewHandle};

use crate::ai::agent::conversation::{
    AIConversation, AIConversationId, LocalChildRestorationStatus,
};
use crate::ai::ambient_agents::discovery::is_native_local_run;
use crate::ai::ambient_agents::task::AmbientAgentTask;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::orchestration_event_streamer::{
    LocalChildHistoryInstallation, OrchestrationEventStreamer, agent_task_harness,
};
use crate::pane_group::PaneGroup;
use crate::terminal::TerminalView;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};

impl PaneGroup {
    pub(in crate::pane_group) fn install_local_child_history(
        &mut self,
        streamer: &ModelHandle<OrchestrationEventStreamer>,
        installation: LocalChildHistoryInstallation,
        prepared: &AIConversation,
        ctx: &mut ViewContext<Self>,
    ) {
        // Installation and acknowledgement are synchronous, so queued duplicates see an invalid token.
        if installation.target.pane_group_id != ctx.view_id()
            || !streamer.read(ctx, |streamer, ctx| {
                streamer.is_current_local_child_history_installation(installation, ctx)
            })
        {
            return;
        }
        let result = (|| {
            let child_id = installation.conversation_id;
            anyhow::ensure!(prepared.id() == child_id, "conversation identity changed");
            let pane = self
                .child_agent_panes
                .get(&child_id)
                .copied()
                .context("canonical child pane missing")?;
            anyhow::ensure!(
                self.has_pane_id(pane) && !self.is_pane_hidden_for_close(pane),
                "canonical child pane closed"
            );
            anyhow::ensure!(
                self.pane_id_for_owned_conversation(child_id, ctx) == Some(pane),
                "canonical child pane replaced"
            );
            let terminal = self
                .terminal_view_from_pane_id(pane, ctx)
                .context("owner terminal missing")?;
            anyhow::ensure!(
                terminal.id() == installation.target.terminal_id
                    && terminal.as_ref(ctx).active_conversation_id(ctx) == Some(child_id),
                "owner terminal binding changed"
            );
            let current = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_id)
                .context("conversation missing")?;
            anyhow::ensure!(
                current.driver().owns_turn_lifecycle()
                    && current.local_child_restoration_status()
                        == Some(LocalChildRestorationStatus::Restoring),
                "execution conversation replaced"
            );
            let mut conversation = prepared.clone();
            conversation.preserve_local_subtree_startup(current);
            conversation.set_local_child_restoration_status(LocalChildRestorationStatus::Restoring);
            terminal.update(ctx, |terminal, ctx| {
                terminal.restore_conversation_after_view_creation(
                    RestoredAIConversation::new(conversation),
                    true,
                    RestoreConversationEntryBehavior::PreserveAgentViewState,
                    ctx,
                );
            });
            anyhow::ensure!(
                self.pane_id_for_owned_conversation(child_id, ctx) == Some(pane)
                    && terminal.as_ref(ctx).active_conversation_id(ctx) == Some(child_id),
                "owner terminal changed during installation"
            );
            Ok(())
        })();
        streamer.update(ctx, |streamer, ctx| {
            streamer.acknowledge_local_child_history_installation(installation, result, ctx);
        });
    }
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn release_passive_child_seeding(&mut self, ctx: &mut ViewContext<Self>) {
        self.passive_child_seeding_released = true;
        self.process_pending_parent_child_seeds(ctx);
    }
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn prepare_restored_local_child(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_terminal_id: EntityId,
        task: &AmbientAgentTask,
        env_vars: HashMap<OsString, OsString>,
        ctx: &mut ViewContext<Self>,
    ) -> (
        AIConversationId,
        anyhow::Result<Option<ViewHandle<TerminalView>>>,
    ) {
        let parent_terminal_id = self
            .pane_id_for_owned_conversation(parent_conversation_id, ctx)
            .and_then(|pane| self.terminal_view_from_pane_id(pane, ctx))
            .map_or(parent_terminal_id, |terminal| terminal.id());
        let existing = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation_id_for_agent_id(&task.task_id.to_string());
        let child_id = existing.unwrap_or_else(AIConversationId::new);
        let result = (|| {
            if BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_id)
                .is_some_and(|child| {
                    child.local_child_restoration_status()
                        == Some(LocalChildRestorationStatus::Error)
                })
            {
                return Ok(None);
            }
            if let Some(child) = BlocklistAIHistoryModel::as_ref(ctx).conversation(&child_id)
                && !child.is_remote_child()
                && !child.is_viewing_shared_session()
                && let Some(pane) = self.pane_id_for_owned_conversation(child_id, ctx)
                && self
                    .terminal_view_from_pane_id(pane, ctx)
                    .is_some_and(|terminal| terminal.id() != parent_terminal_id)
            {
                return Ok(None);
            }
            if self.child_agent_panes.contains_key(&child_id) {
                self.discard_child_agent_pane_for_conversation(child_id, ctx);
            }
            let mut child = AIConversation::new_local_child_for_restoration(child_id);
            child.set_task_id(task.task_id);
            child.set_parent_conversation_id(parent_conversation_id);
            if let Some(parent_run) = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&parent_conversation_id)
                .and_then(AIConversation::run_id)
            {
                child.set_parent_agent_id(parent_run);
            }
            child.set_agent_name(task.display_name().to_owned());
            child.set_fallback_display_title(task.title.clone());
            child.set_orchestration_harness(Harness::Oz);
            child.set_last_event_sequence(task.last_event_sequence.unwrap_or(0));
            child.toggle_autoexecute_override();
            if let Some(token) = task
                .conversation_id()
                .filter(|token| !token.trim().is_empty())
            {
                child.set_server_conversation_token(token.to_owned());
            }
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.restore_conversations(parent_terminal_id, vec![child.clone()], ctx);
            });
            let parent_pane = self
                .find_pane_id_for_terminal_view(parent_terminal_id, ctx)
                .context("owner setup: parent pane missing")?;
            anyhow::ensure!(
                child.server_conversation_token().is_some(),
                "owner setup: missing conversation token"
            );
            self.create_hidden_child_agent_pane_with_env(child, parent_pane, env_vars, ctx);
            let pane = self
                .child_agent_panes
                .get(&child_id)
                .context("owner setup: hidden pane unavailable")?;
            self.terminal_view_from_pane_id(*pane, ctx)
                .map(Some)
                .context("owner setup: terminal/controller unavailable")
        })();
        (child_id, result)
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn seed_passive_children(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_terminal_id: EntityId,
        children: &[AmbientAgentTask],
        ctx: &mut ViewContext<Self>,
    ) {
        let parent_terminal_id = self
            .pane_id_for_owned_conversation(parent_conversation_id, ctx)
            .and_then(|pane| self.terminal_view_from_pane_id(pane, ctx))
            .map_or(parent_terminal_id, |terminal| terminal.id());
        for task in children {
            if is_native_local_run(task) {
                continue;
            }
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let child_id = history.ensure_remote_child_conversation(
                    parent_terminal_id,
                    parent_conversation_id,
                    task.task_id.to_string(),
                    task.task_id,
                    task.display_name().to_owned(),
                    task.title.clone(),
                    agent_task_harness(task),
                    ctx,
                );
                history.apply_remote_child_task_snapshot(parent_terminal_id, child_id, task, ctx);
            });
        }
    }
}
