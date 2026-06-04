//! Implementation of terminal panes.
#[cfg(not(target_family = "wasm"))]
use std::collections::HashMap;
use std::sync::mpsc::SyncSender;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
#[cfg(not(target_family = "wasm"))]
use session_sharing_protocol::sharer::SessionSourceType;
use url::Url;
use warp_cli::agent::Harness;
use warp_core::execution_mode::AppExecutionMode;
use warp_multi_agent_api as multi_agent_api;
use warpui::{
    AppContext, EntityId, ModelHandle, SingletonEntity, ViewContext, ViewHandle, WindowId,
};

#[cfg(not(target_family = "wasm"))]
use super::local_harness_launch::{prepare_local_harness_child_launch, PreparedLocalHarnessLaunch};
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneId, PaneStackEvent, PaneView, ShareableLink,
    ShareableLinkError, TerminalPaneId,
};
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::StartAgentExecutionMode;
use crate::ai::ambient_agents::task::{normalize_orchestrator_agent_name, HarnessConfig};
use crate::ai::ambient_agents::{AgentConfigSnapshot, AmbientAgentTaskId};
use crate::ai::blocklist::agent_view::{AgentViewControllerEvent, AgentViewEntryOrigin};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
#[cfg(feature = "local_fs")]
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use crate::ai::blocklist::{BlocklistAIHistoryModel, StartAgentRequest};
use crate::ai::conversation_utils;
use crate::ai::llms::LLMPreferences;
use crate::ai::skills::SkillManager;
use crate::app_state::{AmbientAgentPaneSnapshot, LeafContents, TerminalPaneSnapshot};
use crate::code::buffer_location::LocalOrRemotePath;
#[cfg(not(target_family = "wasm"))]
use crate::features::FeatureFlag;
use crate::pane_group::child_agent::{
    create_error_child_agent_conversation, ErrorChildAgentConversationRequest,
};
#[cfg(feature = "local_fs")]
use crate::pane_group::CodeSource;
use crate::pane_group::Event::OpenConversationHistory;
use crate::pane_group::{self, Direction, PaneGroup};
use crate::persistence::{BlockCompleted, ModelEvent};
use crate::server::server_api::ai::{SpawnAgentRequest, UserQueryMode};
#[cfg(not(target_family = "wasm"))]
use crate::server::server_api::ServerApiProvider;
use crate::session_management::SessionNavigationData;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::shared_session::manager::{Manager, ManagerEvent};
use crate::terminal::shared_session::role_change_modal::RoleChangeOpenSource;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::shared_session::SharedSessionSource;
use crate::terminal::shared_session::{join_link, SharedSessionStatus};
use crate::terminal::view::ambient_agent::should_disable_snapshot;
use crate::terminal::view::Event;
use crate::terminal::{TerminalManager, TerminalView};
use crate::view_components::ToastFlavor;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::{PaneViewLocator, WorkspaceRegistry};
use crate::AIExecutionProfilesModel;
// Imports below are only consumed by the non-wasm `launch_local_*_child`
// dispatch helpers; gating them keeps the wasm build warning-clean.
#[cfg(not(target_family = "wasm"))]
use crate::{
    pane_group::child_agent::{
        create_hidden_child_agent_conversation, HiddenChildAgentConversation,
        HiddenChildAgentConversationRequest, HiddenChildAgentTaskContext,
    },
    terminal::shared_session::IsSharedSessionCreator,
};

pub type TerminalPaneView = PaneView<TerminalView>;

/// Data kept for terminal panes.
pub struct TerminalPane {
    model_event_sender: Option<SyncSender<ModelEvent>>,

    /// Used to uniquely identify the pane, even across separate runs of the app.
    uuid: Vec<u8>,

    pane_configuration: ModelHandle<PaneConfiguration>,

    /// Defining `terminal_manager` before `view` means that `terminal_manager`
    /// gets dropped first (guaranteed by the language), which halts the event
    /// loop and avoids possible deadlocks during session cleanup. This is enforced
    /// by the `PaneStack`, since the terminal manager is the associated data for
    /// the backing pane view.
    view: ViewHandle<TerminalPaneView>,
}

fn resolve_runtime_skills(
    skill_references: &[ai::skills::SkillReference],
    ctx: &AppContext,
) -> Result<Vec<String>, Vec<String>> {
    let skill_manager = SkillManager::as_ref(ctx);
    let mut runtime_skills = Vec::with_capacity(skill_references.len());
    let mut unresolved_references = Vec::new();

    for reference in skill_references {
        let Some(skill) = skill_manager.active_skill_by_reference(reference, ctx) else {
            unresolved_references.push(reference.to_string());
            continue;
        };
        runtime_skills.push(serialize_proto_to_base64(&multi_agent_api::Skill::from(
            skill.clone(),
        )));
    }

    if unresolved_references.is_empty() {
        Ok(runtime_skills)
    } else {
        Err(unresolved_references)
    }
}

fn serialize_proto_to_base64<M: prost::Message>(message: &M) -> String {
    BASE64_STANDARD.encode(message.encode_to_vec())
}

/// Overrides the child's preferred agent-mode LLM. `None` is a no-op
/// (inherits the parent's LLM via `propagate_parent_agent_settings`).
#[cfg(not(target_family = "wasm"))]
fn apply_child_model_id_override(
    child_terminal_view_id: EntityId,
    model_id: Option<&str>,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let Some(model_id) = model_id.map(str::trim).filter(|m| !m.is_empty()) else {
        return;
    };
    let llm_id: ai::LLMId = model_id.into();
    LLMPreferences::handle(ctx).update(ctx, |llm_prefs, ctx| {
        llm_prefs.update_preferred_agent_mode_llm(&llm_id, child_terminal_view_id, ctx);
    });
}

/// Returns the host terminal's `SharedSessionSource`, or `None` if it is
/// not currently a shared-session creator. Reads the underlying
/// `TerminalModel` directly via the host's `TerminalView`.
#[cfg(not(target_family = "wasm"))]
pub(in crate::pane_group) fn host_terminal_shared_session_source_type(
    parent_terminal_view: &ViewHandle<TerminalView>,
    ctx: &AppContext,
) -> Option<SharedSessionSource> {
    let model = parent_terminal_view.as_ref(ctx).model.lock();
    if let Some(source) = model.shared_session_source() {
        return Some(source.clone());
    }
    if let SharedSessionStatus::SharePendingPreBootstrap { source } = model.shared_session_status()
    {
        return Some(source.clone());
    }
    None
}

/// Builds the `IsSharedSessionCreator` for a child pane spawned by
/// `run_agents(local)`. Returns `Yes` (stamped with the child's `task_id`)
/// only when `OrchestrationViewerPillBar` is enabled and the host carries
/// an orchestrator `task_id`. The host's variant kind is preserved so
/// cloud-only UI stays gated on `AmbientAgent`.
#[cfg(not(target_family = "wasm"))]
pub(in crate::pane_group) fn inherit_share_for_local_child(
    host_source: Option<&SharedSessionSource>,
    child_task_id: AmbientAgentTaskId,
) -> IsSharedSessionCreator {
    if !FeatureFlag::OrchestrationViewerPillBar.is_enabled() {
        return IsSharedSessionCreator::No;
    }
    let Some(host_source) = host_source else {
        return IsSharedSessionCreator::No;
    };
    if host_source.orchestrator_task_id().is_none() {
        return IsSharedSessionCreator::No;
    }
    let child_task_id_str = child_task_id.to_string();
    let source = match &host_source.source_type {
        SessionSourceType::User => SharedSessionSource::user(Some(child_task_id_str)),
        SessionSourceType::AmbientAgent { .. } => {
            SharedSessionSource::ambient_agent(Some(child_task_id_str))
        }
    };
    IsSharedSessionCreator::Yes { source }
}

impl TerminalPane {
    pub(in crate::pane_group) fn new(
        uuid: Vec<u8>,
        terminal_manager: ModelHandle<Box<dyn TerminalManager>>,
        terminal_view: ViewHandle<TerminalView>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Self {
        let pane_configuration = terminal_view.as_ref(ctx).pane_configuration().to_owned();
        let view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_terminal_pane_ctx(ctx);
            PaneView::new(
                pane_id,
                terminal_view,
                terminal_manager,
                pane_configuration.clone(),
                ctx,
            )
        });

        Self {
            model_event_sender,
            uuid,
            pane_configuration,
            view,
        }
    }

    /// The [`PaneView<TerminalView>`] for this pane.
    #[cfg(any(test, feature = "integration_tests"))]
    pub(in crate::pane_group) fn pane_view(&self) -> ViewHandle<TerminalPaneView> {
        self.view.to_owned()
    }

    /// The [`TerminalView`] backing the [`PaneView`] for this terminal pane.
    pub(crate) fn terminal_view(&self, ctx: &AppContext) -> ViewHandle<TerminalView> {
        self.view.as_ref(ctx).child(ctx)
    }

    /// The UUID that identifies this terminal session across app restarts.
    pub(in crate::pane_group) fn session_uuid(&self) -> Vec<u8> {
        self.uuid.clone()
    }

    /// The terminal manager responsible for this session's event loop.
    pub(in crate::pane_group) fn terminal_manager(
        &self,
        ctx: &AppContext,
    ) -> ModelHandle<Box<dyn TerminalManager>> {
        self.view.as_ref(ctx).child_data(ctx).clone()
    }

    /// Instructs the SQLite thread to delete blocks for this session.
    pub(in crate::pane_group) fn delete_blocks(&self, ctx: &AppContext) {
        if !AppExecutionMode::as_ref(ctx).can_save_session() {
            return;
        }

        if let Some(sender) = &self.model_event_sender {
            let model_event = ModelEvent::DeleteBlocks(self.uuid.clone());
            if let Err(err) = sender.send(model_event) {
                log::error!(
                    "Error sending blocks deleted event for terminal id {} {:?}",
                    self.terminal_view(ctx).id(),
                    err
                );
            }
        }
    }

    pub fn session_navigation_data(
        &self,
        pane_group_id: EntityId,
        window_id: WindowId,
        app: &AppContext,
    ) -> SessionNavigationData {
        let view = self.terminal_view(app).as_ref(app);
        SessionNavigationData::new(
            view.full_prompt(app),
            view.prompt_elements(app),
            view.session_command_context(app),
            PaneViewLocator {
                pane_group_id,
                pane_id: self.id(),
            },
            view.last_focus_ts(),
            view.is_read_only(),
            window_id,
            view.model.lock().shared_session_status().clone(),
        )
    }

    pub fn terminal_pane_id(&self) -> TerminalPaneId {
        self.id()
            .as_terminal_pane_id()
            .expect("Should be able to derive a TerminalPaneId from TerminalPane")
    }
}

impl PaneContent for TerminalPane {
    fn id(&self) -> PaneId {
        PaneId::from_terminal_pane_view(&self.view)
    }

    fn attach(
        &self,
        group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // TODO(ben): As much as possible, logic from PaneGroup::add_session should go here.
        //  This will simplify PaneGroup, especially when implementing pane management.
        let terminal_pane_id = self.terminal_pane_id();

        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        // Attach the initial terminal view in the stack.
        attach_terminal_view(&self.terminal_view(ctx), terminal_pane_id, ctx);

        // Subscribe to the pane stack to handle views being pushed/popped.
        let pane_stack = self.view.as_ref(ctx).pane_stack().clone();
        ctx.subscribe_to_model(&pane_stack, move |group, _, event, ctx| {
            handle_pane_stack_event(group, event, terminal_pane_id, ctx);
        });

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(terminal_pane_id.into(), event, ctx);
        });

        if SyncedInputState::as_ref(ctx).should_sync_this_pane_group(ctx.view_id(), ctx.window_id())
        {
            if let Some(active_pane_view) = group.active_session_view(ctx) {
                let event = active_pane_view
                    .as_ref(ctx)
                    .create_sync_event_based_on_terminal_state(ctx);

                group.send_sync_event_to_session(terminal_pane_id, &event, ctx);
            }
        }

        let terminal_view_id = self.terminal_view(ctx).id();
        let manager_model = Manager::handle(ctx);
        ctx.subscribe_to_model(&manager_model, move |group, model_handle, event, ctx| {
            if let ManagerEvent::JoinedSession {
                session_id: _,
                view_id,
            } = event
            {
                // only take action if the view id is ours
                if *view_id == terminal_view_id {
                    let url = retrieve_shared_session_link(model_handle.as_ref(ctx), view_id);
                    group.handle_pane_link_updated(terminal_pane_id.into(), url, ctx);
                }
            }
        });

        #[cfg(feature = "local_fs")]
        {
            ctx.subscribe_to_model(
                &BlocklistAIHistoryModel::handle(ctx),
                move |group, _, event, ctx| {
                    let Some(model_event_sender) = group.model_event_sender.clone() else {
                        return;
                    };

                    let is_shared_ambient_agent_session = group
                        .terminal_view_from_pane_id(terminal_pane_id, ctx)
                        .map(|view| {
                            view.as_ref(ctx)
                                .model
                                .lock()
                                .is_shared_ambient_agent_session()
                        })
                        .unwrap_or(false);

                    handle_ai_history_event(
                        event,
                        terminal_view_id,
                        terminal_pane_id,
                        model_event_sender,
                        is_shared_ambient_agent_session,
                        ctx,
                    );
                },
            );
        }

        // Store the pane group entity ID on the agent view controller so the
        // message bar can perform pane-group-scoped visibility checks.
        let pane_group_id = ctx.view_id();
        let terminal_view = self.terminal_view(ctx);
        let agent_view_controller = terminal_view.as_ref(ctx).agent_view_controller().clone();
        agent_view_controller.update(ctx, |controller, _ctx| {
            controller.set_pane_group_id(pane_group_id);
        });
        ctx.subscribe_to_model(&agent_view_controller, move |group, _, event, ctx| {
            if let AgentViewControllerEvent::EnteredAgentView {
                conversation_id,
                display_mode,
                ..
            } = event
            {
                if display_mode.is_fullscreen() {
                    group.restore_missing_child_agent_panes_for_parent(
                        *conversation_id,
                        terminal_pane_id.into(),
                        ctx,
                    );
                }
            }
        });
        let active_session = terminal_view.as_ref(ctx).active_session().clone();
        let active_stack_view = pane_stack.as_ref(ctx).active_view().clone();
        let active_ambient_session_registration = active_stack_view
            .as_ref(ctx)
            .ambient_agent_task_id_for_details_panel(ctx)
            .map(|task_id| (active_stack_view.id(), task_id));
        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_agent_view_controller(
                &agent_view_controller,
                &active_session,
                terminal_view_id,
                ctx,
            );
            if let Some((terminal_view_id, task_id)) = active_ambient_session_registration {
                model.register_ambient_session(terminal_view_id, task_id, ctx);
            }
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        if matches!(detach_type, DetachType::Closed) {
            // Only immediately clear conversations and delete blocks if the session is being
            // permanently closed.
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model
                    .clear_conversations_in_terminal_view(self.terminal_view(ctx).id(), ctx);
            });
            self.delete_blocks(ctx);
        }

        // Unsubscribe from all views in the pane stack.
        let pane_stack = self.view.as_ref(ctx).pane_stack().clone();
        let contents = pane_stack.as_ref(ctx).entries().to_vec();
        let terminal_view_ids = contents
            .iter()
            .map(|(_, view)| view.id())
            .collect::<Vec<_>>();
        for (manager, view) in contents {
            // Notify the view that it's being detached so it can react appropriately
            // (e.g. the shared-session viewer tears down its network only when the detach
            // is not reversible).
            manager.update(ctx, |terminal_manager, ctx| {
                terminal_manager.on_view_detached(detach_type, ctx);
            });
            ctx.unsubscribe_to_view(&view);
        }

        // Notify the active agent views model that the terminal view has been closed
        // (and that any active views are no longer active). On a `HiddenForClose` detach,
        // `attach` will re-register via `register_agent_view_controller` when the tab is
        // restored, so this is safe to run unconditionally.
        let terminal_view_id = self.terminal_view(ctx).id();
        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
            for terminal_view_id in terminal_view_ids {
                model.unregister_agent_view_controller(terminal_view_id, ctx);
                model.unregister_ambient_session(terminal_view_id, ctx);
            }
        });

        // Clean up any active CLI agent session so its notification is removed.
        // Skip this for moves — the session is still running and will re-register in the new tab.
        if !matches!(detach_type, DetachType::Moved) {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.remove_session(terminal_view_id, ctx);
            });
        }

        ctx.unsubscribe_to_model(&pane_stack);

        ctx.unsubscribe_to_view(&self.view);
        ctx.unsubscribe_to_model(
            &self
                .terminal_view(ctx)
                .as_ref(ctx)
                .agent_view_controller()
                .clone(),
        );

        ctx.unsubscribe_to_model(&Manager::handle(ctx));

        #[cfg(feature = "local_fs")]
        {
            ctx.unsubscribe_to_model(&BlocklistAIHistoryModel::handle(ctx));
        }
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let view = self.terminal_view(app).as_ref(app);
        let is_active = view.is_active_session(app);

        // Capture the current input_config from the AI input model
        let current_input_config = view.input_config(app.as_ref());

        if view.model.lock().shared_session_status().is_viewer() {
            // We save and restore ambient agent sessions
            // (restoring the shared session if it's still open and the conversation transcript otherwise).
            if let Some(ambient_model) = view.ambient_agent_view_model() {
                let ambient_model = ambient_model.as_ref(app);
                let task_id = ambient_model.task_id();

                return LeafContents::AmbientAgent(AmbientAgentPaneSnapshot {
                    uuid: self.uuid.clone(),
                    task_id,
                });
            }

            LeafContents::Terminal(TerminalPaneSnapshot {
                uuid: self.uuid.clone(),
                cwd: None,
                is_active,
                is_read_only: false,
                shell_launch_data: None,
                input_config: None,
                llm_model_override: None,
                active_profile_id: None,
                conversation_ids_to_restore: vec![],
                active_conversation_id: None,
            })
        } else if let Some(task_id) = view
            .ambient_agent_view_model()
            .and_then(|ambient_model| ambient_model.as_ref(app).task_id())
        {
            LeafContents::AmbientAgent(AmbientAgentPaneSnapshot {
                uuid: self.uuid.clone(),
                task_id: Some(task_id),
            })
        } else if view.model.lock().is_conversation_transcript_viewer() {
            // Conversation transcript viewers (opened from the conversation list)
            // can be restored via the ambient agent task if one exists.
            let task_id = view.model.lock().ambient_agent_task_id();
            if task_id.is_some() {
                LeafContents::AmbientAgent(AmbientAgentPaneSnapshot {
                    uuid: self.uuid.clone(),
                    task_id,
                })
            } else {
                LeafContents::Terminal(TerminalPaneSnapshot {
                    uuid: self.uuid.clone(),
                    cwd: None,
                    is_active,
                    is_read_only: false,
                    shell_launch_data: None,
                    input_config: None,
                    llm_model_override: None,
                    active_profile_id: None,
                    conversation_ids_to_restore: vec![],
                    active_conversation_id: None,
                })
            }
        } else {
            let llm_model_override =
                LLMPreferences::as_ref(app).get_base_llm_override(self.terminal_view(app).id());

            let active_profile_id = AIExecutionProfilesModel::as_ref(app)
                .active_profile(Some(self.terminal_view(app).id()), app)
                .sync_id();

            // Collect all conversation IDs for this terminal view
            let conversation_ids_to_restore = BlocklistAIHistoryModel::as_ref(app)
                .all_live_conversations_for_terminal_view(self.terminal_view(app).id())
                .map(|conversation| conversation.id())
                .collect();

            // Capture agent view state: if fullscreen, store the active conversation ID
            let active_conversation_id = view
                .agent_view_controller()
                .as_ref(app)
                .agent_view_state()
                .display_mode()
                .filter(|mode| mode.is_fullscreen())
                .and_then(|_| {
                    view.agent_view_controller()
                        .as_ref(app)
                        .agent_view_state()
                        .active_conversation_id()
                });

            LeafContents::Terminal(TerminalPaneSnapshot {
                uuid: self.uuid.clone(),
                cwd: view.pwd_if_local(app),
                is_active,
                is_read_only: view.model.lock().is_read_only(),
                shell_launch_data: view.shell_launch_data_if_local(app),
                input_config: Some(current_input_config),
                llm_model_override,
                active_profile_id,
                conversation_ids_to_restore,
                active_conversation_id,
            })
        }
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.terminal_view(ctx)
            .update(ctx, |view, ctx| view.redetermine_global_focus(ctx));
    }

    fn shareable_link(
        &self,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        let manager = self.terminal_manager(ctx);
        let the_model = manager.as_ref(ctx).model();
        let lock = the_model.lock();

        // Check if this is a conversation transcript viewer
        if lock.is_conversation_transcript_viewer() {
            // Try to get the conversation token from the history model
            let history_model = crate::ai::blocklist::BlocklistAIHistoryModel::handle(ctx);
            let terminal_view_id = self.terminal_view(ctx).id();

            // Find the conversation for this terminal view
            // We're assuming the conversation transcript view only has one conversation.
            // TODO(roland): store conversation id or server conversation token on the model ConversationTranscriptViewerStatus
            if let Some(conversation) = history_model
                .as_ref(ctx)
                .all_live_conversations_for_terminal_view(terminal_view_id)
                .next()
            {
                if let Some(token) = conversation.server_conversation_token() {
                    let url_string = token.conversation_link();
                    if let Ok(url) = url::Url::parse(&url_string) {
                        return Ok(ShareableLink::Pane { url });
                    }
                }
            }

            // If we can't get the conversation link yet (still loading or not available),
            // return Expected error to preserve the current browser URL
            return Err(ShareableLinkError::Expected);
        }

        // Check for shared session status
        let session_status = lock.shared_session_status();
        match session_status {
            SharedSessionStatus::NotShared => Ok(ShareableLink::Base),
            SharedSessionStatus::ActiveViewer { role: _ } => {
                let manager = Manager::as_ref(ctx);
                let terminal_view_id = self.terminal_view(ctx).id();
                if let Some(url) = retrieve_shared_session_link(manager, &terminal_view_id) {
                    Ok(ShareableLink::Pane { url })
                } else {
                    Err(ShareableLinkError::Unexpected(String::from(
                        "Failed to retreive shared session link",
                    )))
                }
            }
            _ => Err(ShareableLinkError::Expected),
        }
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}

fn retrieve_shared_session_link(manager: &Manager, terminal_view_id: &EntityId) -> Option<Url> {
    let Some(session_id) = manager.session_id(terminal_view_id) else {
        log::warn!("Failed to get join link args for updating browser url");
        return None;
    };
    if let Ok(url) = Url::parse(&join_link(&session_id)) {
        return Some(url);
    }
    None
}

#[derive(Clone, Copy)]
struct AgentConversationActionState {
    owner_terminal_view_id: EntityId,
    task_id: Option<AmbientAgentTaskId>,
    is_in_progress: bool,
    is_cloud_cancel_candidate: bool,
}

fn agent_conversation_action_state(
    conversation_id: AIConversationId,
    ctx: &AppContext,
) -> Option<AgentConversationActionState> {
    let history_model = BlocklistAIHistoryModel::as_ref(ctx);
    let conversation = history_model.conversation(&conversation_id)?;
    let owner_terminal_view_id =
        history_model.terminal_view_id_for_conversation(&conversation_id)?;
    Some(AgentConversationActionState {
        owner_terminal_view_id,
        task_id: conversation.task_id(),
        is_in_progress: conversation.status().is_in_progress(),
        is_cloud_cancel_candidate: conversation.is_remote_child()
            || conversation.is_viewing_shared_session(),
    })
}

fn terminal_view_for_owner_in_group(
    group: &PaneGroup,
    owner_terminal_view_id: EntityId,
    ctx: &AppContext,
) -> Option<ViewHandle<TerminalView>> {
    let pane_id = group.find_pane_id_for_terminal_view(owner_terminal_view_id, ctx)?;
    group.terminal_view_from_pane_id(pane_id, ctx)
}

fn pane_group_and_terminal_view_for_owner(
    owner_terminal_view_id: EntityId,
    ctx: &AppContext,
) -> Option<(ViewHandle<PaneGroup>, ViewHandle<TerminalView>)> {
    WorkspaceRegistry::as_ref(ctx)
        .all_workspaces(ctx)
        .into_iter()
        .find_map(|(_, workspace)| {
            workspace.as_ref(ctx).tab_views().find_map(|pane_group| {
                terminal_view_for_owner_in_group(
                    pane_group.as_ref(ctx),
                    owner_terminal_view_id,
                    ctx,
                )
                .map(|terminal_view| (pane_group.clone(), terminal_view))
            })
        })
}

fn stop_local_agent_conversation(
    group: &PaneGroup,
    owner_terminal_view_id: EntityId,
    conversation_id: AIConversationId,
    ctx: &mut ViewContext<PaneGroup>,
) -> bool {
    let terminal_view = terminal_view_for_owner_in_group(group, owner_terminal_view_id, ctx)
        .or_else(|| {
            pane_group_and_terminal_view_for_owner(owner_terminal_view_id, ctx)
                .map(|(_, terminal_view)| terminal_view)
        });
    let Some(terminal_view) = terminal_view else {
        log::warn!(
            "StopAgentConversation: no terminal view found for conversation {conversation_id:?}"
        );
        return false;
    };

    terminal_view.update(ctx, |terminal_view, ctx| {
        terminal_view.stop_local_agent_conversation(conversation_id, ctx);
    });
    true
}

fn cancel_cloud_agent_task(
    task_id: Option<AmbientAgentTaskId>,
    conversation_id: AIConversationId,
    show_toast: bool,
    ctx: &mut ViewContext<PaneGroup>,
) -> bool {
    let Some(task_id) = task_id else {
        log::warn!(
            "cancel_cloud_agent_task: cloud conversation {conversation_id:?} has no task id"
        );
        return false;
    };
    if show_toast {
        crate::ai::ambient_agents::cancel_task_with_toast(task_id, ctx);
    } else {
        crate::ai::ambient_agents::cancel_task_silently(task_id, ctx);
    }
    true
}

fn stop_agent_conversation(
    group: &PaneGroup,
    conversation_id: AIConversationId,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let Some(state) = agent_conversation_action_state(conversation_id, ctx) else {
        log::warn!("StopAgentConversation: conversation {conversation_id:?} not found");
        return;
    };
    if !state.is_in_progress {
        return;
    }
    if state.is_cloud_cancel_candidate {
        cancel_cloud_agent_task(state.task_id, conversation_id, true, ctx);
    } else if !stop_local_agent_conversation(
        group,
        state.owner_terminal_view_id,
        conversation_id,
        ctx,
    ) {
        // If the owner view is gone, still make Stop visible in history.
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            history_model.update_conversation_status(
                state.owner_terminal_view_id,
                conversation_id,
                ConversationStatus::Cancelled,
                ctx,
            );
        });
    }
}

fn pane_group_hosting_split_off_child(
    conversation_id: AIConversationId,
    ctx: &AppContext,
) -> Option<ViewHandle<PaneGroup>> {
    WorkspaceRegistry::as_ref(ctx)
        .all_workspaces(ctx)
        .into_iter()
        .find_map(|(_, workspace)| {
            workspace.as_ref(ctx).tab_views().find_map(|pane_group| {
                let group = pane_group.as_ref(ctx);
                group
                    .child_agent_origin()
                    .is_some_and(|origin| origin.conversation_id == conversation_id)
                    .then(|| pane_group.clone())
            })
        })
}

fn discard_child_agent_pane_for_conversation(
    group: &mut PaneGroup,
    owner_terminal_view_id: Option<EntityId>,
    conversation_id: AIConversationId,
    ctx: &mut ViewContext<PaneGroup>,
) -> bool {
    if group.discard_child_agent_pane_for_conversation(conversation_id, ctx) {
        return true;
    }
    if let Some(split_off_pane_group) = pane_group_hosting_split_off_child(conversation_id, ctx) {
        if split_off_pane_group.id() != ctx.view_id()
            && split_off_pane_group.update(ctx, |pane_group, ctx| {
                pane_group.discard_child_agent_pane_for_conversation(conversation_id, ctx)
            })
        {
            return true;
        }
    }

    let Some(owner_terminal_view_id) = owner_terminal_view_id else {
        return false;
    };
    let Some((owner_pane_group, _)) =
        pane_group_and_terminal_view_for_owner(owner_terminal_view_id, ctx)
    else {
        return false;
    };
    if owner_pane_group.id() == ctx.view_id() {
        return false;
    }

    owner_pane_group.update(ctx, |pane_group, ctx| {
        pane_group.discard_child_agent_pane_for_conversation(conversation_id, ctx)
    })
}

fn kill_agent_conversation(
    group: &mut PaneGroup,
    source_terminal_view_id: Option<EntityId>,
    conversation_id: AIConversationId,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let state = agent_conversation_action_state(conversation_id, ctx);
    // Tombstone every Kill so late events cannot restore a removed child.
    OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
        streamer.mark_conversation_killed(conversation_id, ctx);
    });

    if let Some(state) = state {
        if state.is_in_progress {
            if state.is_cloud_cancel_candidate {
                cancel_cloud_agent_task(state.task_id, conversation_id, false, ctx);
            } else {
                stop_local_agent_conversation(
                    group,
                    state.owner_terminal_view_id,
                    conversation_id,
                    ctx,
                );
            }
        }
    }

    let owner_terminal_view_id = state
        .map(|state| state.owner_terminal_view_id)
        .or(source_terminal_view_id);
    if !discard_child_agent_pane_for_conversation(
        group,
        owner_terminal_view_id,
        conversation_id,
        ctx,
    ) {
        log::warn!("KillAgentConversation: no child pane found for {conversation_id:?}");
    }

    if owner_terminal_view_id.is_none() {
        log::warn!(
            "KillAgentConversation: no terminal view found for conversation {conversation_id:?}"
        );
    }
    // Delete (not remove): drop the conversation from sqlite + cloud so a
    // killed child does not resurrect on restart.
    conversation_utils::delete_conversation(conversation_id, owner_terminal_view_id, ctx);
}

/// Attaches a terminal view to the pane group by subscribing to its events
/// and setting the file tree code model.
fn attach_terminal_view(
    terminal_view: &ViewHandle<TerminalView>,
    terminal_pane_id: TerminalPaneId,
    ctx: &mut ViewContext<PaneGroup>,
) {
    ctx.subscribe_to_view(
        terminal_view,
        move |group: &mut PaneGroup, _, event, ctx| {
            handle_terminal_view_event(group, terminal_pane_id, event, ctx);
        },
    );
}

/// Handles events from the pane stack when views are added or removed.
fn handle_pane_stack_event(
    group: &mut PaneGroup,
    event: &PaneStackEvent<TerminalView>,
    terminal_pane_id: TerminalPaneId,
    ctx: &mut ViewContext<PaneGroup>,
) {
    match event {
        PaneStackEvent::ViewAdded(terminal_view) => {
            attach_terminal_view(terminal_view, terminal_pane_id, ctx);
        }
        PaneStackEvent::ViewRemoved(terminal_view) => {
            ctx.unsubscribe_to_view(terminal_view);
        }
    }

    // Ensure we use the new top-level view's title and active session status.
    // TODO(ben): This shouldn't be necessary once titles are set declaratively.
    if let Some(active_terminal) = group.terminal_view_from_pane_id(terminal_pane_id, ctx) {
        active_terminal.update(ctx, |view, ctx| view.on_pane_state_change(ctx));
    }
}

fn handle_terminal_view_event(
    group: &mut PaneGroup,
    terminal_pane_id: TerminalPaneId,
    event: &Event,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let pane_id = terminal_pane_id.into();

    if group.pane_contents.contains_key(&pane_id) {
        match event {
            Event::Escape => ctx.emit(pane_group::Event::Escape),
            Event::ExecuteCommand(event) => {
                ctx.emit(pane_group::Event::ExecuteCommand(event.clone()));
            }
            Event::Exited => {
                // If the shell process exited before it successfully bootstrapped,
                // keep the pane open.  There might be useful information visible
                // in the output, and if this was the first shell spawned when the
                // user started the app, it will prevent it from suddenly quitting.
                if group
                    .terminal_view_from_pane_id(terminal_pane_id, ctx)
                    .is_some_and(|terminal_view| {
                        !terminal_view.as_ref(ctx).is_login_shell_bootstrapped()
                    })
                {
                    return;
                }

                group.close_pane(pane_id, ctx);
            }
            Event::CloseRequested => {
                group.close_pane_with_confirmation(pane_id, ctx);
            }
            Event::Pane(pane_event) => group.handle_pane_event(pane_id, pane_event, ctx),
            Event::BlockListCleared => {
                // Capture CMD-K to clear blocks here so we could remove
                // all the associated blocks stored in the history.
                if let Some(terminal_pane) = group.terminal_session_by_id(pane_id) {
                    terminal_pane.delete_blocks(ctx);
                }
            }
            Event::ShareModalOpened(block_id) => {
                group.terminal_with_open_share_block_modal = Some(terminal_pane_id);
                group.share_block_modal.update(ctx, |share_modal, ctx| {
                    if let Some(session) = group.terminal_view_from_pane_id(pane_id, ctx) {
                        let model = session.read(ctx, |view, _| view.model.clone());
                        share_modal.open_with_model_update(model, *block_id, ctx);
                        ctx.notify();
                    }
                });
                ctx.notify();
            }
            Event::SendNotification(notification) => {
                ctx.emit(pane_group::Event::SendNotification {
                    notification: notification.clone(),
                    pane_id,
                })
            }
            Event::PluggableNotification { title, body } => {
                let message = if let Some(t) = title {
                    format!("{t}: {body}")
                } else {
                    body.clone()
                };
                ctx.emit(pane_group::Event::ShowToast {
                    message,
                    flavor: ToastFlavor::Default,
                    pane_id: Some(pane_id),
                })
            }
            Event::AppStateChanged => {
                ctx.emit(pane_group::Event::AppStateChanged);
            }
            Event::BlockCompleted { block, is_local } => {
                match group.terminal_session_by_id(pane_id) {
                    Some(pane) => {
                        if *GeneralSettings::as_ref(ctx).restore_session
                            && AppExecutionMode::as_ref(ctx).can_save_session()
                        {
                            if let Some(sender) = &group.model_event_sender {
                                let block_completed_event = ModelEvent::SaveBlock(BlockCompleted {
                                    pane_id: pane.session_uuid(),
                                    block: block.clone(),
                                    is_local: *is_local,
                                });

                                let sender_clone = sender.clone();
                                let _ = ctx.spawn(async move {
                                // Sending over a sync sender can block the current thread, so we do this async.
                                sender_clone.send(block_completed_event)
                            }, move |_, res, _| {
                                if let Err(err) = res {
                                    log::error!("Error sending block completed event for terminal id {terminal_pane_id:?} {err:?}");
                                }
                            });
                            }
                        }
                        ctx.emit(pane_group::Event::ActiveSessionChanged);
                    }
                    None => {
                        log::error!("Could not find uuid for terminal id: {terminal_pane_id:?}");
                    }
                };
            }
            Event::SessionBootstrapped => {
                ctx.emit(pane_group::Event::ActiveSessionChanged);
            }
            Event::OpenSettings(section) => {
                ctx.emit(pane_group::Event::OpenSettings(*section));
            }
            Event::OpenAutoReloadModal { purchased_credits } => {
                ctx.emit(pane_group::Event::OpenAutoReloadModal {
                    purchased_credits: *purchased_credits,
                });
            }
            #[cfg(not(target_family = "wasm"))]
            Event::OpenPluginInstructionsPane(agent, kind) => {
                ctx.emit(pane_group::Event::OpenPluginInstructionsPane(*agent, *kind));
            }
            Event::AskAIAssistant(ask_type) => {
                ctx.emit(pane_group::Event::AskAIAssistant(ask_type.to_owned()))
            }
            Event::SyncInput(sync_event) => {
                if SyncedInputState::as_ref(ctx)
                    .should_sync_this_pane_group(ctx.view_id(), ctx.window_id())
                {
                    ctx.emit(pane_group::Event::SyncInput(sync_event.clone()));
                }
            }
            Event::ShowCommandSearch(options) => {
                ctx.emit(pane_group::Event::ShowCommandSearch(options.clone()));
            }
            Event::TerminalViewStateChanged => {
                ctx.emit(pane_group::Event::TerminalViewStateChanged);
            }
            Event::OnboardingTutorialCompleted => {
                ctx.emit(pane_group::Event::OnboardingTutorialCompleted);
            }
            Event::OpenWorkflowModalWithCommand(command) => {
                ctx.emit(pane_group::Event::OpenWorkflowModalWithCommand(
                    command.clone(),
                ));
            }
            Event::OpenWorkflowModalWithCloudWorkflow(workflow_id) => {
                ctx.emit(pane_group::Event::OpenCloudWorkflowForEdit(*workflow_id));
            }
            Event::OpenWorkflowModalWithTemporary(workflow) => {
                ctx.emit(pane_group::Event::OpenWorkflowModalWithTemporary(
                    workflow.clone(),
                ));
            }
            Event::OpenPromptEditor => {
                ctx.emit(pane_group::Event::OpenPromptEditor);
            }
            Event::OpenAgentToolbarEditor => {
                ctx.emit(pane_group::Event::OpenAgentToolbarEditor);
            }
            Event::OpenCLIAgentToolbarEditor => {
                ctx.emit(pane_group::Event::OpenCLIAgentToolbarEditor);
            }
            Event::OpenFileInWarp { path, session } => {
                ctx.emit(pane_group::Event::OpenFileInWarp {
                    path: LocalOrRemotePath::Local(path.clone()),
                    session: session.clone(),
                });
            }
            #[cfg(feature = "local_fs")]
            Event::PreviewCodeInWarp { source } => {
                ctx.emit(pane_group::Event::PreviewCodeInWarp {
                    source: source.clone(),
                });
            }
            #[cfg(feature = "local_fs")]
            Event::OpenCodeInWarp { source, layout } => {
                ctx.emit(pane_group::Event::OpenCodeInWarp {
                    source: source.clone(),
                    layout: *layout,
                    line_col: if let CodeSource::Link { range_start, .. } = source {
                        *range_start
                    } else {
                        None
                    },
                });
            }
            Event::OpenCodeDiff { view } => {
                ctx.emit(pane_group::Event::OpenCodeDiff { view: view.clone() });
            }
            Event::OpenCodeReviewPane(arg) => {
                ctx.emit(pane_group::Event::OpenCodeReviewPane(arg.clone()));
            }
            Event::OpenCodeReviewPaneAndScrollToComment {
                open_code_review,
                comment,
                diff_mode,
            } => {
                ctx.emit(pane_group::Event::OpenCodeReviewPaneAndScrollToComment {
                    open_code_review: open_code_review.clone(),
                    comment: comment.clone(),
                    diff_mode: diff_mode.clone(),
                });
            }
            Event::ImportAllCodeReviewComments {
                open_code_review,
                comments,
                diff_mode,
            } => {
                ctx.emit(pane_group::Event::ImportAllCodeReviewComments {
                    open_code_review: open_code_review.clone(),
                    comments: comments.clone(),
                    diff_mode: diff_mode.clone(),
                });
            }
            Event::ToggleCodeReviewPane(arg) => {
                ctx.emit(pane_group::Event::ToggleCodeReviewPane(arg.clone()));
            }
            Event::OpenShareSessionModal { open_source } => {
                group.open_share_session_modal(terminal_pane_id, *open_source, ctx)
            }
            // When the host's manual share stops, also stop the share on
            // any local children whose share was auto-created via
            // `inherit_share_for_local_child`. Skipped on wasm because the
            // transitive-share tracker is only populated on non-wasm
            // dispatch paths.
            #[cfg(not(target_family = "wasm"))]
            Event::StopSharingCurrentSession { .. } => {
                group.stop_transitively_shared_child_shares(pane_id, ctx);
            }
            Event::OpenShareSessionDeniedModal => {
                group.open_share_session_denied_modal(terminal_pane_id, ctx);
            }
            Event::FocusSession => {
                group.focus_pane(terminal_pane_id.into(), true, ctx);
                ctx.emit(pane_group::Event::FocusPaneGroup);
            }
            Event::OpenSharedSessionRoleChangeModal { source } => match source {
                RoleChangeOpenSource::ViewerRequest { role } => {
                    group.open_shared_session_viewer_request_modal(terminal_pane_id, *role, ctx)
                }
                RoleChangeOpenSource::SharerResponse {
                    participant_id,
                    role_request_id,
                    role,
                } => group.open_shared_session_sharer_response_modal(
                    terminal_pane_id,
                    participant_id.clone(),
                    role_request_id.clone(),
                    *role,
                    ctx,
                ),
                RoleChangeOpenSource::SharerGrant { participant_id } => group
                    .open_shared_session_sharer_grant_modal(
                        terminal_pane_id,
                        participant_id.clone(),
                        ctx,
                    ),
            },
            Event::CloseSharedSessionRoleChangeModal(source) => {
                group.close_shared_session_role_change_modal(*source, ctx);
            }
            Event::RoleRequestInFlight { role_request_id } => {
                group.set_shared_session_role_change_modal_request_id(role_request_id.clone(), ctx);
            }
            Event::RoleRequestCancelled(role_request_id) => {
                group.remove_shared_session_role_request(role_request_id.clone(), ctx);
            }
            Event::OpenWarpDriveObjectInPane(uid) => {
                ctx.emit(pane_group::Event::OpenWarpDriveObjectInPane(uid.clone()));
            }
            Event::OpenSuggestedAgentModeWorkflowModal { workflow_and_id } => {
                ctx.emit(pane_group::Event::OpenSuggestedAgentModeWorkflowModal {
                    workflow_and_id: workflow_and_id.clone(),
                });
            }
            Event::OpenSuggestedRuleDialog { rule_and_id } => {
                ctx.emit(pane_group::Event::OpenSuggestedRuleModal {
                    rule_and_id: rule_and_id.clone(),
                });
            }
            Event::OpenAIFactCollection { sync_id } => {
                ctx.emit(pane_group::Event::OpenAIFactCollection { sync_id: *sync_id });
            }
            Event::SummarizationCancelDialogToggled { is_open } => {
                group.terminal_with_open_summarization_dialog = is_open.then_some(terminal_pane_id);
                ctx.notify();
            }
            Event::EnvironmentSetupModeSelectorToggled { is_open } => {
                group.pane_with_open_environment_setup_mode_selector = is_open.then_some(pane_id);
                ctx.notify();
            }
            Event::AuthSecretDeleteConfirmationDialogToggled { is_open } => {
                group.pane_with_open_auth_secret_delete_confirmation_dialog =
                    is_open.then_some(pane_id);
                ctx.notify();
            }
            Event::AnonymousUserSignup => ctx.emit(pane_group::Event::AnonymousUserSignup),
            #[cfg(feature = "local_fs")]
            Event::OpenFileWithTarget {
                path,
                target,
                line_col,
            } => {
                ctx.emit(pane_group::Event::OpenFileWithTarget {
                    path: path.clone(),
                    target: target.clone(),
                    line_col: *line_col,
                });
            }
            Event::CopyFileToRemote { command, upload_id } => {
                let new_pane_id = group.insert_terminal_pane(
                    Direction::Right,
                    pane_id,
                    None, /*chosen_shell*/
                    ctx,
                );

                group.hide_pane_for_job(new_pane_id.into(), ctx);

                let new_terminal_view = group
                    .active_session_view(ctx)
                    .expect("should have new terminal view");
                new_terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view.set_pending_command(command, ctx);
                    terminal_view.set_is_ssh_uploader(true);
                });

                ctx.emit(pane_group::Event::FileUploadCommand {
                    upload_id: *upload_id,
                    command: command.to_owned(),
                    remote_pane_id: terminal_pane_id,
                    local_pane_id: new_pane_id,
                });

                group.focus_pane(pane_id, true, ctx);
            }
            Event::FileUploadPasswordPending => {
                ctx.emit(pane_group::Event::FileUploadPasswordPending {
                    local_pane_id: terminal_pane_id,
                });
            }
            Event::OpenConversationHistory => {
                ctx.emit(OpenConversationHistory);
            }
            Event::FileUploadFinished(exit_code) => {
                ctx.emit(pane_group::Event::FileUploadFinished {
                    local_pane_id: terminal_pane_id,
                    exit_code: *exit_code,
                });

                // Each upload spawns its own new terminal pane. Once an upload
                // has finished, we know that its terminal session will no
                // longer be responsible for any UI-based uploads.
                if let Some(uploader_terminal_view) =
                    group.terminal_view_from_pane_id(terminal_pane_id, ctx)
                {
                    uploader_terminal_view.update(ctx, |terminal_view, _ctx| {
                        terminal_view.set_is_ssh_uploader(false);
                    });
                }
            }
            Event::OpenFileUploadSession(upload_id) => {
                ctx.emit(pane_group::Event::OpenFileUploadSession {
                    remote_pane_id: terminal_pane_id,
                    upload_id: *upload_id,
                })
            }
            Event::TerminateFileUploadSession(upload_id) => {
                ctx.emit(pane_group::Event::TerminateFileUploadSession {
                    remote_pane_id: terminal_pane_id,
                    upload_id: *upload_id,
                })
            }
            Event::SignupAnonymousUser { entrypoint } => {
                ctx.emit(pane_group::Event::SignupAnonymousUser {
                    entrypoint: *entrypoint,
                });
            }
            Event::OpenThemeChooser => {
                ctx.emit(pane_group::Event::OpenThemeChooser);
            }
            Event::OpenMCPSettingsPage { page } => {
                ctx.emit(pane_group::Event::OpenMCPSettingsPage { page: *page });
            }
            Event::OpenFilesPalette { source } => {
                ctx.emit(pane_group::Event::OpenFilesPalette { source: *source })
            }
            Event::OpenAddRulePane => {
                ctx.emit(crate::pane_group::Event::OpenAddRulePane);
            }
            Event::OpenRulesPane => {
                ctx.emit(crate::pane_group::Event::OpenAIFactCollection { sync_id: None });
            }
            Event::OpenAddPromptPane { initial_content } => {
                ctx.emit(crate::pane_group::Event::OpenAddPromptPane {
                    initial_content: initial_content.clone(),
                });
            }
            Event::OpenEnvironmentManagementPane => {
                ctx.emit(crate::pane_group::Event::OpenEnvironmentManagementPane);
            }
            #[cfg(feature = "local_fs")]
            Event::FileRenamed { old_path, new_path } => {
                ctx.emit(pane_group::Event::FileRenamed {
                    old_path: old_path.clone(),
                    new_path: new_path.clone(),
                });
            }
            #[cfg(feature = "local_fs")]
            Event::FileDeleted { path } => {
                ctx.emit(pane_group::Event::FileDeleted { path: path.clone() });
            }
            Event::ToggleLeftPanel {
                target_view,
                force_open,
            } => {
                ctx.emit(pane_group::Event::ToggleLeftPanel {
                    target_view: *target_view,
                    force_open: *force_open,
                });
            }
            Event::ToggleAIDocumentPane {
                document_id,
                document_version,
            } => {
                if let Some(conversation_id) =
                    crate::ai::document::ai_document_model::AIDocumentModel::as_ref(ctx)
                        .get_conversation_id_for_document_id(document_id)
                {
                    group.toggle_ai_document_pane(
                        conversation_id,
                        *document_id,
                        *document_version,
                        ctx,
                    );
                }
            }
            Event::HideAIDocumentPanes => {
                group.close_all_ai_document_panes(ctx);
            }
            Event::OpenAIDocumentPane {
                document_id,
                document_version,
                is_auto_open,
            } => {
                let should_open = if *is_auto_open {
                    // Auto-open: only open if there's already a visible plan pane
                    // (to replace it with the newest plan) or if there's enough space.
                    let has_visible_ai_doc_pane = group
                        .ai_document_panes()
                        .any(|pane_id| !group.is_pane_hidden_for_close(pane_id));

                    has_visible_ai_doc_pane
                        || group
                            .terminal_view_from_pane_id(terminal_pane_id, ctx)
                            .is_some_and(|tv| tv.as_ref(ctx).can_auto_open_panel())
                } else {
                    // User-triggered: always open.
                    true
                };

                if should_open {
                    if let Some(conversation_id) =
                        crate::ai::document::ai_document_model::AIDocumentModel::as_ref(ctx)
                            .get_conversation_id_for_document_id(document_id)
                    {
                        group.open_ai_document_pane(
                            conversation_id,
                            *document_id,
                            *document_version,
                            ctx,
                        );
                    }
                }
            }
            Event::OpenAgentProfileEditor { profile_id } => {
                ctx.emit(pane_group::Event::OpenAgentProfileEditor {
                    profile_id: *profile_id,
                });
            }
            Event::InsertCodeReviewComments {
                repo_path,
                comments,
                diff_mode,
                open_code_review,
            } => {
                ctx.emit(pane_group::Event::InsertCodeReviewComments {
                    repo_path: repo_path.clone(),
                    comments: comments.to_owned(),
                    diff_mode: diff_mode.to_owned(),
                    open_code_review: open_code_review.clone(),
                });
            }
            Event::ShowCloudAgentCapacityModal { variant } => {
                ctx.emit(pane_group::Event::ShowCloudAgentCapacityModal { variant: *variant });
            }
            Event::FreeTierLimitCheckTriggered => {
                ctx.emit(pane_group::Event::FreeTierLimitCheckTriggered);
            }
            Event::RevealChildAgent { conversation_id } => {
                // Routed through the swap mechanism to land all reveal cases in one path.
                if group.ensure_hidden_child_agent_pane_for_conversation(*conversation_id, ctx) {
                    group.swap_active_pane_to_conversation(pane_id, *conversation_id, ctx);
                } else {
                    log::warn!(
                        "RevealChildAgent: failed to materialize child conversation {conversation_id:?}"
                    );
                }
            }
            Event::SwapPaneToConversation { conversation_id } => {
                // Swap visibility instead of cloning so in-flight state in the
                // target pane is preserved.
                if group.ensure_hidden_child_agent_pane_for_conversation(*conversation_id, ctx) {
                    group.swap_active_pane_to_conversation(pane_id, *conversation_id, ctx);
                } else {
                    log::warn!(
                        "SwapPaneToConversation: failed to materialize conversation {conversation_id:?}"
                    );
                }
            }
            Event::EnsureSharedSessionViewerChildPane {
                conversation_id,
                session_id,
            } => {
                // Emitted by `OrchestrationViewerModel` when a child of the
                // orchestrator currently being viewed first surfaces a
                // joinable `session_id`. Materializes a dedicated hidden
                // shared-session viewer pane for the child so subsequent pill
                // clicks land on a populated agent view rather than an empty
                // cloud-mode shell.
                group.ensure_shared_session_viewer_child_pane(*conversation_id, *session_id, ctx);
            }
            Event::OpenChildAgentInNewTab { conversation_id } => {
                // Pane group can't add tabs; forward to the workspace.
                if group.ensure_hidden_child_agent_pane_for_conversation(*conversation_id, ctx) {
                    ctx.emit(pane_group::Event::OpenChildAgentInNewTab {
                        conversation_id: *conversation_id,
                    });
                } else {
                    log::warn!(
                        "OpenChildAgentInNewTab: failed to materialize child conversation {conversation_id:?}"
                    );
                }
            }
            Event::OpenChildAgentInNewPane { conversation_id } => {
                // Reuse the existing hidden child pane to preserve in-flight
                // state and the live transcript instead of creating a new view.
                if group.ensure_hidden_child_agent_pane_for_conversation(*conversation_id, ctx) {
                    if group
                        .unhide_child_agent_pane_for_split_off(*conversation_id, ctx)
                        .is_none()
                    {
                        log::warn!(
                            "OpenChildAgentInNewPane: no hidden child pane registered for conversation {conversation_id:?}"
                        );
                    }
                } else {
                    log::warn!(
                        "OpenChildAgentInNewPane: failed to materialize child conversation {conversation_id:?}"
                    );
                }
            }
            Event::StopAgentConversation { conversation_id } => {
                stop_agent_conversation(group, *conversation_id, ctx);
            }
            Event::KillAgentConversation { conversation_id } => {
                let source_terminal_view_id = group
                    .terminal_view_from_pane_id(terminal_pane_id, ctx)
                    .map(|terminal_view| terminal_view.id());
                kill_agent_conversation(group, source_terminal_view_id, *conversation_id, ctx);
            }
            Event::StartAgentConversation(request) => {
                dispatch_start_agent_conversation(
                    group,
                    pane_id,
                    terminal_pane_id,
                    request.clone(),
                    ctx,
                );
            }
            _ => {}
        }
    } else {
        log::warn!("Session {terminal_pane_id:?} not found");
    }
}

/// Dispatches a StartAgent request to the appropriate per-mode helper.
/// Each helper echoes the child conversation id back via
/// [`BlocklistAIHistoryModel::record_new_conversation_request_complete`].
#[cfg_attr(target_family = "wasm", allow(unused_variables))]
fn dispatch_start_agent_conversation(
    group: &mut PaneGroup,
    parent_pane_id: PaneId,
    terminal_pane_id: TerminalPaneId,
    request: StartAgentRequest,
    ctx: &mut ViewContext<PaneGroup>,
) {
    match request.execution_mode.clone() {
        #[cfg(not(target_family = "wasm"))]
        StartAgentExecutionMode::Local {
            harness_type: None,
            model_id,
        } => {
            launch_local_no_harness_child(group, parent_pane_id, request, model_id, ctx);
        }
        #[cfg(not(target_family = "wasm"))]
        StartAgentExecutionMode::Local {
            harness_type: Some(harness_type),
            model_id,
        } => {
            launch_local_harness_child(
                group,
                parent_pane_id,
                terminal_pane_id,
                request,
                harness_type,
                model_id,
                ctx,
            );
        }
        #[cfg(target_family = "wasm")]
        StartAgentExecutionMode::Local { .. } => {
            let _ = create_error_child_agent_conversation(
                group,
                ErrorChildAgentConversationRequest {
                    parent_pane_id,
                    name: request.name,
                    parent_conversation_id: request.parent_conversation_id,
                    request_id: Some(request.id),
                    orchestration_harness: None,
                    error_message: "Local child agents are not supported in WASM builds."
                        .to_string(),
                },
                ctx,
            );
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
            launch_remote_child(
                group,
                parent_pane_id,
                request,
                RemoteLaunchFields {
                    environment_id,
                    skill_references,
                    model_id,
                    computer_use_enabled,
                    worker_host,
                    harness_type,
                    title,
                    auth_secret_name,
                },
                ctx,
            );
        }
    }
}

/// Sets up a hidden child pane for a Local-no-harness (Oz) agent and
/// dispatches the prompt. Asynchronously creates the server-side `ai_tasks`
/// row via `AIClient::create_agent_task` at dispatch time, mirroring the
/// third-party-harness path (see [`launch_local_harness_child`]). The
/// resulting `task_id` is stamped onto the child's `AIConversation` (so the
/// per-`Network` share-reporter in `local_tty/terminal_manager.rs` can link
/// the shared session id to the child task once the shell bootstraps) and
/// onto the child's `BlocklistAIController` via the
/// `HiddenChildAgentTaskContext` (so the agent UI reflects it). On failure
/// the child surfaces as an error conversation instead.
///
/// Gated to non-wasm because `ServerApiProvider` is `cfg(not(wasm))`-only.
/// `dispatch_start_agent_conversation`'s wasm wildcard arm routes the Oz
/// path through `create_error_child_agent_conversation` instead.
#[cfg(not(target_family = "wasm"))]
fn launch_local_no_harness_child(
    group: &mut PaneGroup,
    parent_pane_id: PaneId,
    request: StartAgentRequest,
    model_id: Option<String>,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();
    let request_id = request.id;
    let agent_name = normalize_orchestrator_agent_name(&request.name);
    let request_name = agent_name.clone().unwrap_or_default();
    let parent_conversation_id = request.parent_conversation_id;
    let parent_run_id = request.parent_run_id.clone();
    let prompt = request.prompt.clone();

    // Snapshot the host terminal's shared-session source before the spawn
    // so we can cascade it onto the child's source type once the spawn
    // returns.
    let host_source = group
        .terminal_view_from_pane_id(parent_pane_id, ctx)
        .and_then(|view| host_terminal_shared_session_source_type(&view, ctx));

    let prompt_for_create = prompt.clone();
    let agent_name_for_create = agent_name.clone();
    let _ = ctx.spawn(
        async move {
            ai_client
                .create_agent_task(
                    prompt_for_create,
                    None,
                    parent_run_id,
                    Some(AgentConfigSnapshot {
                        name: agent_name_for_create,
                        ..Default::default()
                    }),
                )
                .await
        },
        move |group, result, ctx| match result {
            Ok(child_task_id) => {
                let is_shared_session_creator =
                    inherit_share_for_local_child(host_source.as_ref(), child_task_id);

                if let Some(HiddenChildAgentConversation {
                    terminal_view: new_terminal_view,
                    terminal_view_id,
                    conversation_id,
                    ..
                }) = create_hidden_child_agent_conversation(
                    group,
                    HiddenChildAgentConversationRequest {
                        parent_pane_id,
                        name: request_name.clone(),
                        parent_conversation_id,
                        orchestration_harness: Some(Harness::Oz),
                        env_vars: HashMap::new(),
                        task_context: Some(HiddenChildAgentTaskContext {
                            task_id: child_task_id,
                            working_dir: None,
                        }),
                        is_shared_session_creator,
                    },
                    ctx,
                ) {
                    apply_child_model_id_override(terminal_view_id, model_id.as_deref(), ctx);

                    // Stamp the task id on the child conversation directly
                    // so the share-reporter in
                    // `local_tty/terminal_manager.rs` can resolve it from
                    // the selected conversation when the share handshake
                    // succeeds. Mirrors the pattern used by
                    // `OrchestrationViewerModel::apply_children_fetch`.
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                        if let Some(conversation) = model.conversation_mut(&conversation_id) {
                            conversation.set_task_id(child_task_id);
                        }
                        model.record_new_conversation_request_complete(
                            request_id,
                            conversation_id,
                            ctx,
                        );
                    });

                    new_terminal_view.update(ctx, |terminal_view, ctx| {
                        terminal_view
                            .ai_controller()
                            .update(ctx, |controller, ctx| {
                                controller.send_agent_query_in_conversation(
                                    prompt.clone(),
                                    conversation_id,
                                    ctx,
                                );
                            });

                        terminal_view.enter_agent_view(
                            None,
                            Some(conversation_id),
                            AgentViewEntryOrigin::ChildAgent,
                            ctx,
                        );
                    });
                } else {
                    let _ = create_error_child_agent_conversation(
                        group,
                        ErrorChildAgentConversationRequest {
                            parent_pane_id,
                            name: request_name,
                            parent_conversation_id,
                            request_id: Some(request_id),
                            orchestration_harness: Some(Harness::Oz),
                            error_message:
                                "Failed to create a hidden pane for the local child agent."
                                    .to_string(),
                        },
                        ctx,
                    );
                }
            }
            Err(error) => {
                let _ = create_error_child_agent_conversation(
                    group,
                    ErrorChildAgentConversationRequest {
                        parent_pane_id,
                        name: request_name,
                        parent_conversation_id,
                        request_id: Some(request_id),
                        orchestration_harness: Some(Harness::Oz),
                        error_message: format!("Failed to create local child task: {error}"),
                    },
                    ctx,
                );
            }
        },
    );
}

/// Asynchronously prepares a local harness launch, then creates the
/// hidden child pane and executes the launch command.
#[cfg(not(target_family = "wasm"))]
#[allow(clippy::too_many_arguments)]
fn launch_local_harness_child(
    group: &mut PaneGroup,
    parent_pane_id: PaneId,
    terminal_pane_id: TerminalPaneId,
    request: StartAgentRequest,
    harness_type: String,
    model_id: Option<String>,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let startup_directory = group.startup_path_for_new_session(Some(terminal_pane_id), ctx);
    let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();
    let request_id = request.id;
    let agent_name = normalize_orchestrator_agent_name(&request.name);
    let request_name = agent_name.clone().unwrap_or_default();
    let parent_conversation_id = request.parent_conversation_id;
    let parent_run_id = request.parent_run_id.clone();
    let prompt = request.prompt.clone();
    let orchestration_harness =
        Harness::parse_orchestration_harness(&harness_type).unwrap_or(Harness::Unknown);
    let shell_type = group
        .terminal_view_from_pane_id(parent_pane_id, ctx)
        .and_then(|terminal_view| terminal_view.as_ref(ctx).active_session_shell_type(ctx));

    // Snapshot the host's shared-session source before the spawn so we can
    // cascade it onto the prepared child task.
    let host_source = group
        .terminal_view_from_pane_id(parent_pane_id, ctx)
        .and_then(|view| host_terminal_shared_session_source_type(&view, ctx));

    let model_id_for_harness_env = model_id.clone();
    let agent_name_for_task = agent_name.clone();
    let _ = ctx.spawn(
        async move {
            prepare_local_harness_child_launch(
                prompt,
                harness_type,
                model_id_for_harness_env,
                parent_run_id,
                agent_name_for_task,
                shell_type,
                startup_directory,
                ai_client,
            )
            .await
        },
        move |group, result, ctx| match result {
            Ok(launch) => {
                let PreparedLocalHarnessLaunch {
                    command,
                    env_vars,
                    run_id,
                    task_id,
                } = launch;
                let is_shared_session_creator =
                    inherit_share_for_local_child(host_source.as_ref(), task_id);
                if let Some(HiddenChildAgentConversation {
                    terminal_view: new_terminal_view,
                    terminal_view_id,
                    conversation_id,
                    ..
                }) = create_hidden_child_agent_conversation(
                    group,
                    HiddenChildAgentConversationRequest {
                        parent_pane_id,
                        name: request_name.clone(),
                        parent_conversation_id,
                        orchestration_harness: Some(orchestration_harness),
                        env_vars,
                        task_context: None,
                        is_shared_session_creator,
                    },
                    ctx,
                ) {
                    apply_child_model_id_override(terminal_view_id, model_id.as_deref(), ctx);

                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                        model.record_new_conversation_request_complete(
                            request_id,
                            conversation_id,
                            ctx,
                        );
                    });

                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                        history_model.assign_run_id_for_conversation(
                            conversation_id,
                            run_id,
                            Some(task_id),
                            terminal_view_id,
                            ctx,
                        );
                    });

                    new_terminal_view.update(ctx, |terminal_view, ctx| {
                        terminal_view.execute_command_or_set_pending(&command, ctx);
                        terminal_view.enter_agent_view(
                            None,
                            Some(conversation_id),
                            AgentViewEntryOrigin::ChildAgent,
                            ctx,
                        );
                    });
                } else {
                    let _ = create_error_child_agent_conversation(
                        group,
                        ErrorChildAgentConversationRequest {
                            parent_pane_id,
                            name: request_name,
                            parent_conversation_id,
                            request_id: Some(request_id),
                            orchestration_harness: Some(orchestration_harness),
                            error_message:
                                "Failed to create a hidden pane for the local child harness."
                                    .to_string(),
                        },
                        ctx,
                    );
                }
            }
            Err(error_message) => {
                let _ = create_error_child_agent_conversation(
                    group,
                    ErrorChildAgentConversationRequest {
                        parent_pane_id,
                        name: request_name,
                        parent_conversation_id,
                        request_id: Some(request_id),
                        orchestration_harness: Some(orchestration_harness),
                        error_message,
                    },
                    ctx,
                );
            }
        },
    );
}

/// Fields destructured from `StartAgentExecutionMode::Remote` so they can be
/// passed through to [`launch_remote_child`] as a single argument cluster.
struct RemoteLaunchFields {
    environment_id: String,
    skill_references: Vec<ai::skills::SkillReference>,
    model_id: String,
    computer_use_enabled: bool,
    worker_host: String,
    harness_type: String,
    title: String,
    /// Managed-secret name forwarded from the orchestration UI for non-Oz
    /// harness credentials. Resolved to `AgentConfigSnapshot.harness_auth_secrets`
    /// when applicable.
    auth_secret_name: Option<String>,
}

/// Sets up a hidden ambient-agent pane for a Remote child agent: creates the
/// child conversation, marks it as remote, resolves runtime skills (silently
/// bailing with a status update on resolution failure), constructs the
/// `SpawnAgentRequest`, enters the agent view, and kicks off the spawn via
/// the ambient agent view model. Returns the freshly-created
/// `AIConversationId` on success.
///
/// The executor handle is used to echo the child conversation id back to
/// the executor's pending table via
/// [`StartAgentExecutor::record_child_conversation`] so the
/// `ConversationServerTokenAssigned` event that fires when
/// `model.spawn_agent_with_request` resolves can be matched back to this
/// request.
fn launch_remote_child(
    group: &mut PaneGroup,
    parent_pane_id: PaneId,
    request: StartAgentRequest,
    fields: RemoteLaunchFields,
    ctx: &mut ViewContext<PaneGroup>,
) -> Option<AIConversationId> {
    let RemoteLaunchFields {
        environment_id,
        skill_references,
        model_id,
        computer_use_enabled,
        worker_host,
        harness_type,
        title,
        auth_secret_name,
    } = fields;

    let request_id = request.id;
    let orchestration_harness = if harness_type.trim().is_empty() {
        Harness::Oz
    } else {
        Harness::parse_orchestration_harness(&harness_type).unwrap_or(Harness::Unknown)
    };
    let Some(parent_run_id) = request.parent_run_id.clone() else {
        log::error!(
            "Remote StartAgent request missing parent_run_id for {:?}",
            request.parent_conversation_id
        );
        return None;
    };

    let agent_name = normalize_orchestrator_agent_name(&request.name);
    let request_name = agent_name.clone().unwrap_or_default();

    let new_pane_id = group.insert_ambient_agent_pane_hidden_for_child_agent(parent_pane_id, ctx);

    let Some(new_terminal_view) = group.terminal_view_from_pane_id(new_pane_id, ctx) else {
        log::error!("Failed to get terminal view for new remote StartAgent pane");
        group.discard_pane(new_pane_id.into(), ctx);
        return None;
    };

    let terminal_view_id = new_terminal_view.id();
    let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
        let id = history_model.start_new_child_conversation(
            terminal_view_id,
            request_name.clone(),
            request.parent_conversation_id,
            Some(orchestration_harness),
            ctx,
        );
        // Mark as remote so the parent's LocalAgentTaskSyncModel skips status
        // reporting — the remote worker handles it.
        history_model.mark_conversation_as_remote_child(id, ctx);
        id
    });

    BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
        model.record_new_conversation_request_complete(request_id, conversation_id, ctx);
    });

    let runtime_skills = match resolve_runtime_skills(&skill_references, ctx) {
        Ok(runtime_skills) => runtime_skills,
        Err(unresolved_references) => {
            let error_message = format!(
                "Failed to resolve child agent skills: {}",
                unresolved_references.join(", ")
            );
            log::error!(
                "Failed to resolve StartAgentV2 skill references for remote child {:?}: {}",
                conversation_id,
                unresolved_references.join(", ")
            );
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model.update_conversation_status_with_error_message(
                    terminal_view_id,
                    conversation_id,
                    ConversationStatus::Error,
                    Some(error_message),
                    ctx,
                );
            });
            return None;
        }
    };

    // Treat an empty environment_id as "no environment specified" so the
    // spawn request leaves the config.environment_id field unset. The
    // server's StartAgent producer defaults to the parent's environment
    // when available, so an empty value here means the caller explicitly
    // opted into running with an empty environment.
    let environment_id = Some(environment_id).filter(|s| !s.trim().is_empty());
    // Unrecognized harness types collapse to None so the server picks
    // its default, matching the behavior of an empty `harness_type`.
    // We deliberately do NOT round-trip `Harness::Unknown` to the server;
    // that variant is for representing server-originated unknowns to the
    // user, not for writes.
    let harness_override = if harness_type.is_empty() {
        None
    } else {
        match <Harness as clap::ValueEnum>::from_str(&harness_type, true) {
            Ok(harness) => Some(HarnessConfig::from_harness_type(harness)),
            Err(_) => {
                log::warn!(
                    "Unknown harness type from StartAgentV2 proto: {harness_type:?}; omitting harness override so the server picks its default"
                );
                None
            }
        }
    };
    let computer_use_enabled =
        (orchestration_harness == Harness::Oz).then_some(computer_use_enabled);
    // Map the run-wide auth secret name into the harness-specific
    // config variant. For unsupported harnesses (Oz, OpenCode, Gemini,
    // Unknown), the secret is silently ignored — those harnesses either
    // use Warp's built-in auth (Oz) or don't currently support managed
    // secrets via this flow.
    let harness_auth_secrets = auth_secret_name
        .as_ref()
        .filter(|name| !name.trim().is_empty())
        .and_then(|name| match orchestration_harness {
            Harness::Claude => Some(crate::ai::ambient_agents::task::HarnessAuthSecretsConfig {
                claude_auth_secret_name: Some(name.clone()),
                codex_auth_secret_name: None,
            }),
            Harness::Codex => Some(crate::ai::ambient_agents::task::HarnessAuthSecretsConfig {
                claude_auth_secret_name: None,
                codex_auth_secret_name: Some(name.clone()),
            }),
            Harness::Oz | Harness::OpenCode | Harness::Gemini | Harness::Unknown => None,
        });
    let spawn_request = SpawnAgentRequest {
        prompt: Some(request.prompt),
        mode: UserQueryMode::Normal,
        config: Some(AgentConfigSnapshot {
            name: agent_name,
            environment_id,
            model_id: (!model_id.is_empty()).then_some(model_id),
            worker_host: (!worker_host.is_empty()).then_some(worker_host),
            computer_use_enabled,
            harness: harness_override,
            harness_auth_secrets,
            ..Default::default()
        }),
        title: (!title.is_empty()).then_some(title),
        team: None,
        skill: None,
        attachments: vec![],
        interactive: Some(true),
        parent_run_id: Some(parent_run_id),
        runtime_skills,
        referenced_attachments: vec![],
        conversation_id: None,
        initial_snapshot_token: None,
        agent_identity_uid: None,
        snapshot_disabled: should_disable_snapshot(ctx).then_some(true),
        orchestration_handoff: None,
    };

    new_terminal_view.update(ctx, |terminal_view, ctx| {
        terminal_view.enter_agent_view(
            None,
            Some(conversation_id),
            AgentViewEntryOrigin::CloudAgent,
            ctx,
        );
        if let Some(ambient_agent_view_model) = terminal_view.ambient_agent_view_model() {
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.set_conversation_id(Some(conversation_id));
                model.spawn_agent_with_request(spawn_request, ctx);
            });
        } else {
            log::error!("Remote StartAgent child pane missing ambient agent view model");
        }
    });

    group
        .child_agent_panes
        .insert(conversation_id, new_pane_id.into());

    Some(conversation_id)
}

#[cfg(feature = "local_fs")]
fn handle_ai_history_event(
    event: &BlocklistAIHistoryEvent,
    terminal_view_id: EntityId,
    terminal_pane_id: TerminalPaneId,
    model_event_sender: SyncSender<ModelEvent>,
    is_shared_ambient_agent_session: bool,
    ctx: &mut ViewContext<PaneGroup>,
) {
    use std::sync::Arc;

    use crate::ai::blocklist::{
        AIQueryHistoryOutputStatus, PersistedAIInput, PersistedAIInputType,
    };

    if event
        .terminal_view_id()
        .is_some_and(|id| id != terminal_view_id)
    {
        return;
    }

    match event {
        BlocklistAIHistoryEvent::AppendedExchange {
            exchange_id,
            conversation_id,
            is_hidden,
            ..
        }
        | BlocklistAIHistoryEvent::UpdatedStreamingExchange {
            exchange_id,
            conversation_id,
            is_hidden,
            ..
        } => {
            // Check if session restoration is enabled.
            if !*GeneralSettings::as_ref(ctx).restore_session
                || !AppExecutionMode::as_ref(ctx).can_save_session()
            {
                return;
            }

            let Some(conversation) =
                BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id)
            else {
                log::warn!("Received event with invalid conversation ID: {conversation_id:?}");
                return;
            };

            let Some(exchange) = conversation.exchange_with_id(*exchange_id) else {
                log::warn!("Received event with invalid exchange ID: {exchange_id:?}");
                return;
            };

            // Hidden blocks and passive-only conversations should not be restored, so we skip
            // them.
            if *is_hidden || conversation.is_entirely_passive() {
                return;
            }

            // Do not persist AI queries from shared ambient agent sessions that we've viewed,
            // as these were sent as part of an ambient agent run and shouldn't polute the up arrow history.
            if is_shared_ambient_agent_session {
                return;
            }

            let persisted_query = PersistedAIInput {
                start_ts: exchange.start_time,
                inputs: exchange
                    .input
                    .iter()
                    .filter_map(|input| PersistedAIInputType::try_from(input).ok())
                    .collect(),
                exchange_id: exchange.id,
                conversation_id: *conversation_id,
                output_status: AIQueryHistoryOutputStatus::from(&exchange.output_status),
                working_directory: exchange.working_directory.clone(),
                // TODO(CORE-3546): shell: exchange.shell.clone(),
                model_id: exchange.model_id.clone(),
                coding_model_id: exchange.coding_model_id.clone(),
            };
            let upsert_ai_query_event = ModelEvent::UpsertAIQuery {
                query: Arc::new(persisted_query),
            };
            let _ = ctx.spawn(
                // Sending over a sync sender can block the current thread, so we
                // do this async.
                async move { model_event_sender.send(upsert_ai_query_event) },
                move |_, res, _| {
                    if let Err(err) = res {
                        log::error!(
                            "Error sending upsert AI query event for terminal id {terminal_pane_id:?} {err:?}"
                        );
                    }
                },
            );
        }
        BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
        | BlocklistAIHistoryEvent::ClearedActiveConversation { .. } => {
            ctx.emit(pane_group::Event::InvalidatedActiveConversation);
        }
        BlocklistAIHistoryEvent::RemoveConversation {
            conversation_id, ..
        } => {
            let conversation_id = conversation_id.to_string();
            // On remove, delete all related AI query and multi-agent conversation data for this conversation.
            let _ = ctx.spawn(
                async move {
                    model_event_sender.send(ModelEvent::DeleteAIConversation {
                        conversation_id: conversation_id.clone(),
                    })?;
                    model_event_sender.send(ModelEvent::DeleteMultiAgentConversations {
                        conversation_ids: vec![conversation_id],
                    })
                },
                |_, res, _| {
                    if let Err(err) = res {
                        log::error!("Error sending delete events for conversation: {err:?}");
                    }
                },
            );
        }
        // DeletedConversation SQL cleanup is handled directly in delete_conversation().
        BlocklistAIHistoryEvent::DeletedConversation { .. }
        | BlocklistAIHistoryEvent::StartedNewConversation { .. }
        | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
        | BlocklistAIHistoryEvent::ReassignedExchange { .. }
        | BlocklistAIHistoryEvent::SetActiveConversation { .. }
        | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
        | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
        | BlocklistAIHistoryEvent::SplitConversation { .. }
        | BlocklistAIHistoryEvent::RestoredConversations { .. }
        | BlocklistAIHistoryEvent::CreatedSubtask { .. }
        | BlocklistAIHistoryEvent::UpgradedTask { .. }
        | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
        | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
        | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
        | BlocklistAIHistoryEvent::ConversationOwnershipTransferred { .. }
        | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
        | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
        | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
        | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => (),
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "terminal_pane_tests.rs"]
mod tests;
