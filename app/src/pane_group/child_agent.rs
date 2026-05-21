use std::{collections::HashMap, ffi::OsString, path::PathBuf};

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::attachment_utils::attachments_download_dir;
use warp_cli::agent::Harness;
use warpui::{EntityId, SingletonEntity, ViewContext, ViewHandle};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::{BlocklistAIHistoryModel, StartAgentRequestId};
use crate::ai::llms::LLMPreferences;
use crate::pane_group::{PaneGroup, PaneId};
use crate::terminal::shared_session::IsSharedSessionCreator;
use crate::terminal::TerminalView;
use crate::AIExecutionProfilesModel;

pub(crate) struct HiddenChildAgentConversation {
    pub terminal_view: ViewHandle<TerminalView>,
    pub terminal_view_id: EntityId,
    pub conversation_id: AIConversationId,
}
#[derive(Clone, Debug)]
pub(crate) struct HiddenChildAgentTaskContext {
    pub task_id: AmbientAgentTaskId,
    pub working_dir: Option<PathBuf>,
}

pub(crate) struct HiddenChildAgentConversationRequest {
    pub parent_pane_id: PaneId,
    pub name: String,
    pub parent_conversation_id: AIConversationId,
    pub orchestration_harness: Option<Harness>,
    pub env_vars: HashMap<OsString, OsString>,
    pub task_context: Option<HiddenChildAgentTaskContext>,
    /// When `Yes`, the child pane's terminal is asked to share its session
    /// using the embedded `SessionSourceType` once the shell bootstraps.
    /// The dispatch helpers in `terminal_pane.rs` compute this from the host
    /// terminal's own shared-session state (gated on
    /// `FeatureFlag::OrchestrationViewerPillBar`).
    pub is_shared_session_creator: IsSharedSessionCreator,
}

pub(crate) struct ErrorChildAgentConversationRequest {
    pub parent_pane_id: PaneId,
    pub name: String,
    pub parent_conversation_id: AIConversationId,
    pub request_id: Option<StartAgentRequestId>,
    pub orchestration_harness: Option<Harness>,
    pub error_message: String,
}

pub(crate) fn apply_hidden_child_agent_task_context(
    terminal_view: &ViewHandle<TerminalView>,
    task_context: &HiddenChildAgentTaskContext,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let task_id = task_context.task_id;
    let working_dir = task_context.working_dir.clone();

    terminal_view.update(ctx, move |terminal_view, ctx| {
        terminal_view
            .ai_controller()
            .update(ctx, |controller, ctx| {
                controller.set_ambient_agent_task_id(Some(task_id), ctx);
                if let Some(working_dir) = working_dir.as_deref() {
                    controller.set_attachments_download_dir(attachments_download_dir(working_dir));
                }
            });
    });
}

fn propagate_parent_agent_settings(
    group: &PaneGroup,
    parent_pane_id: PaneId,
    child_terminal_view_id: EntityId,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let Some(parent_terminal_view) = group.terminal_view_from_pane_id(parent_pane_id, ctx) else {
        log::warn!(
            "Could not find parent terminal view for pane {parent_pane_id:?}; child will use default AI profile"
        );
        return;
    };

    let parent_view_id = parent_terminal_view.id();
    let parent_profile_id = *AIExecutionProfilesModel::as_ref(ctx)
        .active_profile(Some(parent_view_id), ctx)
        .id();
    AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
        profiles.set_active_profile(child_terminal_view_id, parent_profile_id, ctx);
    });

    let parent_base_model_id = LLMPreferences::as_ref(ctx)
        .get_active_base_model(ctx, Some(parent_view_id))
        .id
        .clone();
    LLMPreferences::handle(ctx).update(ctx, |llm_prefs, ctx| {
        llm_prefs.update_preferred_agent_mode_llm(
            &parent_base_model_id,
            child_terminal_view_id,
            ctx,
        );
    });
}

fn start_new_child_conversation(
    terminal_view_id: EntityId,
    name: String,
    parent_conversation_id: AIConversationId,
    orchestration_harness: Option<Harness>,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
        history_model.start_new_child_conversation(
            terminal_view_id,
            name,
            parent_conversation_id,
            orchestration_harness,
            ctx,
        )
    })
}

pub(crate) fn create_hidden_child_agent_conversation(
    group: &mut PaneGroup,
    request: HiddenChildAgentConversationRequest,
    ctx: &mut ViewContext<PaneGroup>,
) -> Option<HiddenChildAgentConversation> {
    let HiddenChildAgentConversationRequest {
        parent_pane_id,
        name,
        parent_conversation_id,
        orchestration_harness,
        env_vars,
        task_context,
        is_shared_session_creator,
    } = request;
    let new_pane_id = group.insert_terminal_pane_hidden_for_child_agent(
        parent_pane_id,
        env_vars,
        is_shared_session_creator,
        ctx,
    );
    let Some(new_terminal_view) = group.terminal_view_from_pane_id(new_pane_id, ctx) else {
        log::error!("Failed to get terminal view for new StartAgent pane");
        group.discard_pane(new_pane_id.into(), ctx);
        return None;
    };

    let terminal_view_id = new_terminal_view.id();
    propagate_parent_agent_settings(group, parent_pane_id, terminal_view_id, ctx);
    if let Some(task_context) = task_context.as_ref() {
        apply_hidden_child_agent_task_context(&new_terminal_view, task_context, ctx);
    }

    let conversation_id = start_new_child_conversation(
        terminal_view_id,
        name,
        parent_conversation_id,
        orchestration_harness,
        ctx,
    );

    group
        .child_agent_panes
        .insert(conversation_id, new_pane_id.into());

    Some(HiddenChildAgentConversation {
        terminal_view: new_terminal_view,
        terminal_view_id,
        conversation_id,
    })
}

fn create_error_child_agent_conversation_context(
    group: &mut PaneGroup,
    parent_pane_id: PaneId,
    name: String,
    parent_conversation_id: AIConversationId,
    orchestration_harness: Option<Harness>,
    ctx: &mut ViewContext<PaneGroup>,
) -> Option<(Option<ViewHandle<TerminalView>>, EntityId, AIConversationId)> {
    if let Some(HiddenChildAgentConversation {
        terminal_view,
        terminal_view_id,
        conversation_id,
        ..
    }) = create_hidden_child_agent_conversation(
        group,
        HiddenChildAgentConversationRequest {
            parent_pane_id,
            name: name.clone(),
            parent_conversation_id,
            orchestration_harness,
            env_vars: HashMap::new(),
            task_context: None,
            is_shared_session_creator: IsSharedSessionCreator::No,
        },
        ctx,
    ) {
        return Some((Some(terminal_view), terminal_view_id, conversation_id));
    }

    let parent_terminal_view = group.terminal_view_from_pane_id(parent_pane_id, ctx)?;
    let parent_terminal_view_id = parent_terminal_view.id();
    let conversation_id = start_new_child_conversation(
        parent_terminal_view_id,
        name,
        parent_conversation_id,
        orchestration_harness,
        ctx,
    );
    Some((None, parent_terminal_view_id, conversation_id))
}

pub(crate) fn create_error_child_agent_conversation(
    group: &mut PaneGroup,
    request: ErrorChildAgentConversationRequest,
    ctx: &mut ViewContext<PaneGroup>,
) -> Option<AIConversationId> {
    let ErrorChildAgentConversationRequest {
        parent_pane_id,
        name,
        parent_conversation_id,
        request_id,
        orchestration_harness,
        error_message,
    } = request;
    let Some((terminal_view, terminal_view_id, conversation_id)) =
        create_error_child_agent_conversation_context(
            group,
            parent_pane_id,
            name,
            parent_conversation_id,
            orchestration_harness,
            ctx,
        )
    else {
        log::error!(
            "Failed to surface local child harness error for parent conversation {parent_conversation_id:?}: {error_message}"
        );
        return None;
    };

    if let Some(request_id) = request_id {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            history_model.record_new_conversation_request_complete(
                request_id,
                conversation_id,
                ctx,
            );
        });
    }
    if let Some(terminal_view) = terminal_view {
        terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.enter_agent_view(
                None,
                Some(conversation_id),
                AgentViewEntryOrigin::ChildAgent,
                ctx,
            );
        });
    }

    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
        history_model.update_conversation_status_with_error_message(
            terminal_view_id,
            conversation_id,
            ConversationStatus::Error,
            Some(error_message),
            ctx,
        );
    });
    Some(conversation_id)
}
