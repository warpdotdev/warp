//! Implementation of terminal panes.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;

use chrono::Utc;
use parking_lot::Mutex;
#[cfg(not(target_family = "wasm"))]
use session_sharing_protocol::sharer::SessionSourceType;
use smol_str::SmolStr;
use url::Url;
#[cfg(not(target_family = "wasm"))]
use warp_cli::agent::Harness;
use warp_core::execution_mode::AppExecutionMode;
use warp_errors::report_error;
use warp_util::path::EscapeChar;
use warpui::{
    AppContext, EntityId, ModelHandle, SingletonEntity, ViewContext, ViewHandle, WindowId,
};

#[cfg(not(target_family = "wasm"))]
use super::local_harness_launch::{PreparedLocalHarnessLaunch, prepare_local_harness_child_launch};
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneId, PaneStackEvent, PaneView, ShareableLink,
    ShareableLinkError, TerminalPaneId,
};
// Imports below are only consumed by the non-wasm `launch_local_*_child`
// dispatch helpers; gating them keeps the wasm build warning-clean.
use crate::AIExecutionProfilesModel;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{RenderableAIError, StartAgentExecutionMode};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::task::normalize_orchestrator_agent_name;
#[cfg(feature = "local_fs")]
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use crate::ai::blocklist::agent_view::{AgentViewControllerEvent, AgentViewEntryOrigin};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::{BlocklistAIHistoryModel, StartAgentRequest};
#[cfg(not(target_family = "wasm"))]
use crate::ai::blocklist::{
    apply_child_agent_model_override, finish_local_oz_child_conversation,
    prepare_local_oz_child_launch,
};
use crate::ai::conversation_utils;
use crate::ai::llms::LLMPreferences;
use crate::ai::orchestration::{RemoteChildLaunchConfig, prepare_remote_child_launch};
use crate::app_state::{
    AmbientAgentPaneSnapshot, LeafContents, RecordedAgentSession, TerminalPaneSnapshot,
};
use crate::code::buffer_location::LocalOrRemotePath;
use crate::features::FeatureFlag;
#[cfg(feature = "local_fs")]
use crate::pane_group::CodeSource;
use crate::pane_group::Event::OpenConversationHistory;
use crate::pane_group::child_agent::{
    ErrorChildAgentConversationRequest, create_error_child_agent_conversation,
};
use crate::pane_group::{self, Direction, PaneGroup};
use crate::persistence::{BlockCompleted, ModelEvent};
#[cfg(not(target_family = "wasm"))]
use crate::server::server_api::ServerApiProvider;
use crate::session_management::SessionNavigationData;
use crate::terminal::cli_agent_resume::{RecordedFlag, ResumeDeclarations};
use crate::terminal::cli_agent_sessions::{CLIAgentSessionsModel, CLIAgentSessionsModelEvent};
use crate::terminal::general_settings::GeneralSettings;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::shared_session::SharedSessionSource;
use crate::terminal::shared_session::manager::{Manager, ManagerEvent};
use crate::terminal::shared_session::role_change_modal::RoleChangeOpenSource;
use crate::terminal::shared_session::{SharedSessionStatus, join_link};
use crate::terminal::view::Event;
use crate::terminal::{CLIAgent, TerminalManager, TerminalView};
use crate::view_components::ToastFlavor;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::{PaneViewLocator, WorkspaceRegistry};
#[cfg(not(target_family = "wasm"))]
use crate::workspaces::user_workspaces::{ResolvedTeamScope, UserWorkspaces};
#[cfg(not(target_family = "wasm"))]
use crate::{
    pane_group::child_agent::{
        HiddenChildAgentConversation, HiddenChildAgentConversationRequest,
        HiddenChildAgentTaskContext, create_hidden_child_agent_conversation,
    },
    terminal::shared_session::IsSharedSessionCreator,
};

pub type TerminalPaneView = PaneView<TerminalView>;

/// What a pane needs to remember between agent-session observations. Held behind an `Arc` because
/// the subscription that reads it outlives the detach it has to know about.
#[derive(Default)]
struct AgentCaptureState {
    /// Whether the pane is currently attached to a live pane group.
    ///
    /// Detaching a pane ends its CLI agent session for every reason but a move, so the `Ended`
    /// that follows a detach says nothing about the agent — which is still running, whether the
    /// pane was hidden for a close the user can undo or the app is shutting down.
    is_attached: AtomicBool,

    /// What the pane last asked the writer to store.
    ///
    /// The session event that drives capture fires once per tool call, so an agent task's worth
    /// of observations is hundreds of writes of a row that has not changed. Only a first
    /// observation or a change is worth sending; the rest are dropped here, before they reach a
    /// bounded channel that a blocking main-thread sender shares.
    last_sent: Mutex<Option<RecordedAgentSession>>,
}

/// Data kept for terminal panes.
pub struct TerminalPane {
    model_event_sender: Option<SyncSender<ModelEvent>>,

    /// Used to uniquely identify the pane, even across separate runs of the app.
    uuid: Vec<u8>,

    pane_configuration: ModelHandle<PaneConfiguration>,

    /// State of this pane's agent session capture, shared with the subscription that writes it.
    agent_capture: Arc<AgentCaptureState>,

    /// Defining `terminal_manager` before `view` means that `terminal_manager`
    /// gets dropped first (guaranteed by the language), which halts the event
    /// loop and avoids possible deadlocks during session cleanup. This is enforced
    /// by the `PaneStack`, since the terminal manager is the associated data for
    /// the backing pane view.
    view: ViewHandle<TerminalPaneView>,
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
/// when the host carries an orchestrator `task_id`. The host's variant kind
/// is preserved so cloud-only UI stays gated on `AmbientAgent`.
#[cfg(not(target_family = "wasm"))]
pub(in crate::pane_group) fn inherit_share_for_local_child(
    host_source: Option<&SharedSessionSource>,
    child_task_id: AmbientAgentTaskId,
) -> IsSharedSessionCreator {
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
            agent_capture: Arc::new(AgentCaptureState::default()),
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
                report_error!(
                    anyhow::Error::new(err).context("Error sending blocks deleted event"),
                    extra: { "terminal_id" => ?self.terminal_view(ctx).id() }
                );
            }
        }
    }

    /// Instructs the SQLite thread to drop whatever agent state was recorded for this session.
    ///
    /// Sent from the permanent-close branch of [`Self::detach`] only, and behind the same guard
    /// [`Self::delete_blocks`] uses. A pane hidden for close comes back if the user undoes the
    /// close, and what it recorded is exactly what resumes its agent then (R20) — only a pane
    /// that will never return leaves a row that nothing can claim.
    pub(in crate::pane_group) fn delete_recorded_agent_session(&self, ctx: &AppContext) {
        if !AppExecutionMode::as_ref(ctx).can_save_session() {
            return;
        }

        if let Some(sender) = &self.model_event_sender {
            let model_event = ModelEvent::SetAgentSession {
                pane_id: self.uuid.clone(),
                session: None,
            };
            if let Err(err) = sender.send(model_event) {
                report_error!(
                    anyhow::Error::new(err).context("Error sending agent session deleted event"),
                    extra: { "terminal_id" => ?self.terminal_view(ctx).id() }
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
        self.agent_capture.is_attached.store(true, Ordering::SeqCst);

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
            && let Some(active_pane_view) = group.active_session_view(ctx)
        {
            let event = active_pane_view
                .as_ref(ctx)
                .create_sync_event_based_on_terminal_state(ctx);

            group.send_sync_event_to_session(terminal_pane_id, &event, ctx);
        }

        let terminal_view_id = self.terminal_view(ctx).id();

        // Recording the pane's agent session is scoped to this pane by filtering on the terminal
        // view captured here, the same way the agent driver scopes its own session subscription:
        // the sessions model is a singleton keyed by terminal view, and a group-wide identity map
        // would have to be kept in step with every pane that moves between groups.
        let agent_capture = self.agent_capture.clone();
        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            move |group, _, event, ctx| {
                if event.terminal_view_id() != terminal_view_id {
                    return;
                }
                capture_agent_session(group, event, terminal_pane_id, &agent_capture, ctx);
            },
        );

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
                && display_mode.is_fullscreen()
            {
                group.restore_missing_child_agent_panes_for_parent(
                    *conversation_id,
                    terminal_pane_id.into(),
                    true,
                    ctx,
                );
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
        // Marked before anything below can end the CLI agent session, so the capture subscription
        // reads a detach for what it is rather than as an agent that finished.
        self.agent_capture
            .is_attached
            .store(false, Ordering::SeqCst);

        if matches!(detach_type, DetachType::Closed) {
            // Only immediately clear conversations and delete blocks if the session is being
            // permanently closed.
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model
                    .clear_conversations_for_terminal_surface(self.terminal_view(ctx).id(), ctx);
            });
            self.delete_blocks(ctx);
            // This detach is the one place that knows the pane will not return, so it is also
            // where the row keyed to its uuid stops being state and starts being garbage.
            self.delete_recorded_agent_session(ctx);
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
                .all_live_conversations_for_terminal_surface(self.terminal_view(app).id())
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
                .all_live_conversations_for_terminal_surface(terminal_view_id)
                .next()
                && let Some(token) = conversation.server_conversation_token()
            {
                let url_string = token.conversation_link();
                if let Ok(url) = url::Url::parse(&url_string) {
                    return Ok(ShareableLink::Pane { url });
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

/// Records what the CLI agent sessions model now reports about this pane, or that it reports
/// nothing, so a restart can offer to resume the agent the pane was running.
///
/// The write is sent from here rather than from a spawned task on purpose: two independently
/// scheduled sends have no order between them, and an older identifier landing after a newer one
/// would offer to resume a conversation the user has already left behind. The payload is a
/// handful of short strings and the writer coalesces per pane, so one ordered send is cheaper
/// than the block writes that already go through this channel.
fn capture_agent_session(
    group: &PaneGroup,
    event: &CLIAgentSessionsModelEvent,
    terminal_pane_id: TerminalPaneId,
    agent_capture: &AgentCaptureState,
    ctx: &mut ViewContext<PaneGroup>,
) {
    // A detached pane records nothing further and clears nothing: its agent is still running,
    // and what it last recorded is exactly what the next launch needs (R20). `remove_session`
    // fires on every detach but a move — including the hide-for-close an undo reverses — and app
    // teardown detaches every pane before draining the writer.
    if !agent_capture.is_attached.load(Ordering::SeqCst) {
        return;
    }

    // The same gate block saving uses: a user who turned session restore off, or a Warp that is
    // not an interactive app, has nothing recorded about their panes.
    if !*GeneralSettings::as_ref(ctx).restore_session
        || !AppExecutionMode::as_ref(ctx).can_save_session()
    {
        return;
    }

    let Some(sender) = group.model_event_sender.clone() else {
        return;
    };
    let Some(pane_id) = group
        .terminal_session_by_id(terminal_pane_id)
        .map(TerminalPane::session_uuid)
    else {
        return;
    };

    let session = match event {
        CLIAgentSessionsModelEvent::SessionUpdated { .. }
        | CLIAgentSessionsModelEvent::StatusChanged { .. } => {
            // Nothing to record until the agent has reported an identifier: a recording without
            // one claims no session and resumes nothing.
            let Some((terminal_view, agent, session_id)) =
                reported_agent_identity(group, terminal_pane_id, event.terminal_view_id(), ctx)
            else {
                return;
            };
            // These events fire once per tool call while the pair naming the conversation holds
            // for the whole task, so the burst is settled on a map lookup rather than on the
            // grid walk, alias resolution and working-directory stat that reading the rest of
            // the state costs.
            let unchanged_identity = agent_capture
                .last_sent
                .lock()
                .as_ref()
                .is_some_and(|sent| sent.agent == agent && sent.session_id == session_id);
            if unchanged_identity {
                return;
            }
            Some(observed_agent_session(
                &terminal_view,
                agent,
                session_id,
                ctx,
            ))
        }
        CLIAgentSessionsModelEvent::Ended { .. } => {
            // An agent replaced rather than removed — a second agent started in the same pane —
            // ends the old session with the new one already registered, and the pane is still
            // running an agent. Only a pane the model reports nothing for has nothing to resume.
            if CLIAgentSessionsModel::as_ref(ctx)
                .session(event.terminal_view_id())
                .is_some()
            {
                return;
            }
            None
        }
        _ => return,
    };

    let mut last_sent = agent_capture.last_sent.lock();
    if records_same_agent_session(last_sent.as_ref(), session.as_ref()) {
        return;
    }
    *last_sent = session.clone();
    drop(last_sent);

    if let Err(err) = sender.send(ModelEvent::SetAgentSession { pane_id, session }) {
        report_error!(
            anyhow::Error::new(err).context("Error sending agent session event"),
            extra: { "terminal_pane_id" => ?terminal_pane_id }
        );
    }
}

/// Whether two observations say the same thing about a pane's agent.
///
/// `observed_at` is deliberately left out: it moves with every tool call, and rewriting a row only
/// to advance it is the write volume this comparison exists to remove. The cost is that the time
/// recorded is when the state was first seen rather than last seen, which only ever loosens the
/// last tie-break between two panes claiming one identifier.
fn records_same_agent_session(
    last_sent: Option<&RecordedAgentSession>,
    observed: Option<&RecordedAgentSession>,
) -> bool {
    match (last_sent, observed) {
        (None, None) => true,
        (Some(last_sent), Some(observed)) => {
            // Destructured so that a field added to the recorded state has to be considered here
            // before a change to it can go unwritten.
            let RecordedAgentSession {
                agent,
                session_id,
                flags,
                directory,
                observed_at: _,
            } = last_sent;
            agent == &observed.agent
                && session_id == &observed.session_id
                && flags == &observed.flags
                && directory == &observed.directory
        }
        _ => false,
    }
}

/// The agent and identifier `terminal_pane_id` is running, with the view they were reported for,
/// or `None` while its agent has reported no session identifier.
///
/// Kept apart from [`observed_agent_session`] so that the pair naming the conversation — all a
/// repeat observation has to be compared on — can be read without paying for the rest.
fn reported_agent_identity(
    group: &PaneGroup,
    terminal_pane_id: TerminalPaneId,
    terminal_view_id: EntityId,
    ctx: &AppContext,
) -> Option<(ViewHandle<TerminalView>, CLIAgent, String)> {
    let terminal_view = group.terminal_view_from_pane_id(terminal_pane_id, ctx)?;
    // A pane can push another terminal view over the one the agent is running in. The pushed
    // view's command line and working directory are not the agent's, so there is nothing to
    // record from it.
    if terminal_view.id() != terminal_view_id {
        return None;
    }
    let (agent, session_id) = CLIAgentSessionsModel::as_ref(ctx)
        .reported_agent_session(terminal_view_id)
        .map(|(agent, session_id)| (agent, session_id.to_owned()))?;
    Some((terminal_view, agent, session_id))
}

/// The agent state to record for a pane whose agent reported `agent` and `session_id`.
fn observed_agent_session(
    terminal_view: &ViewHandle<TerminalView>,
    agent: CLIAgent,
    session_id: String,
    ctx: &AppContext,
) -> RecordedAgentSession {
    let view = terminal_view.as_ref(ctx);
    // The model lock is held only long enough to copy the command text out. Resolving an alias
    // reads the shell session model, and reaching for a second model with this one held is what
    // the locking rule in `AGENTS.md` forbids.
    let command = {
        let model = view.model.lock();
        model
            .block_list()
            .active_block()
            .command_with_secrets_obfuscated(false)
    };

    let shell_session = view
        .active_block_session_id()
        .and_then(|session_id| view.sessions_model().as_ref(ctx).get(session_id));
    let flags = recorded_resume_flags(
        agent,
        &command,
        shell_session
            .as_ref()
            .map(|session| session.shell_family().escape_char()),
        shell_session.as_ref().map(|session| session.aliases()),
    );

    RecordedAgentSession {
        agent,
        session_id,
        flags,
        // Only a local session has a directory to report here. A pane running its agent
        // elsewhere records none rather than a remote path, which could resolve locally and make
        // a session that was never local look like one that can be relaunched in place.
        directory: view
            .pwd_if_local(ctx)
            .map(PathBuf::from)
            .unwrap_or_default(),
        observed_at: Utc::now().naive_utc(),
    }
}

/// The resume-relevant flags `command` gave `agent`, with the first word resolved through the
/// shell session's aliases so that a flag carried by an alias is recorded as one the user ran.
///
/// `command` is the obfuscated form of the invocation, so a secret passed to the agent is
/// recorded as its placeholder. The placeholder fails the declared value shape when the resume
/// invocation is built, which drops the flag instead of replaying a wrong value.
///
/// A command that does not resolve to `agent` contributes no flags at all: it is some other
/// program running in the pane, and its arguments were never the agent's.
fn recorded_resume_flags(
    agent: CLIAgent,
    command: &str,
    escape_char: Option<EscapeChar>,
    aliases: Option<&HashMap<SmolStr, String>>,
) -> Vec<RecordedFlag> {
    let resolved = CLIAgent::resolve_command_aliases(command, escape_char, aliases);
    if !agent.matches_command(&resolved, escape_char) {
        return Vec::new();
    }

    // Splitting on whitespace splits a quoted value too, but every shape the allowlist declares
    // is a bare token, so a value that needed quoting was never one a resume could carry.
    let args = resolved.split_whitespace().skip(1).collect::<Vec<_>>();
    ResumeDeclarations::embedded().extract_resume_flags(agent, &args)
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
        history_model.terminal_surface_id_for_conversation(&conversation_id)?;
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
    if let Some(split_off_pane_group) = pane_group_hosting_split_off_child(conversation_id, ctx)
        && split_off_pane_group.id() != ctx.view_id()
        && split_off_pane_group.update(ctx, |pane_group, ctx| {
            pane_group.discard_child_agent_pane_for_conversation(conversation_id, ctx)
        })
    {
        return true;
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

    if let Some(state) = state
        && state.is_in_progress
    {
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
                let Some(session) = group.terminal_view_from_pane_id(pane_id, ctx) else {
                    return;
                };
                let model = session.read(ctx, |view, _| view.model.clone());

                group.terminal_with_open_share_block_modal = Some(terminal_pane_id);
                group.share_block_modal.update(ctx, |share_modal, ctx| {
                    share_modal.open_with_model_update(model, *block_id, ctx);
                    ctx.notify();
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
                            && let Some(sender) = &group.model_event_sender
                        {
                            let block_completed_event = ModelEvent::SaveBlock(BlockCompleted {
                                pane_id: pane.session_uuid(),
                                block: block.clone(),
                                is_local: *is_local,
                            });

                            let sender_clone = sender.clone();
                            let _ = ctx.spawn(
                                async move {
                                    // Sending over a sync sender can block the current thread, so we do this async.
                                    sender_clone.send(block_completed_event)
                                },
                                move |_, res, _| {
                                    if let Err(err) = res {
                                        report_error!(
                                            anyhow::Error::new(err)
                                                .context("Error sending block completed event"),
                                            extra: { "terminal_pane_id" => ?terminal_pane_id }
                                        );
                                    }
                                },
                            );
                        }
                        ctx.emit(pane_group::Event::ActiveSessionChanged);
                    }
                    None => {
                        report_error!(
                            "Could not find uuid for terminal id",
                            extra: { "terminal_pane_id" => ?terminal_pane_id }
                        );
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
            Event::EnsureUnifiedViewerChildPane {
                conversation_id,
                task,
            } => {
                if FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
                    group.materialize_viewer_child_pane_from_task(
                        *conversation_id,
                        task.as_ref().clone(),
                        ctx,
                    );
                }
            }
            Event::OrchestrationChildSharedSessionJoinFailed {
                conversation_id,
                session_id,
            } => {
                if FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
                    group.recover_viewer_child_join_failure(
                        pane_id,
                        *conversation_id,
                        *session_id,
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

                if should_open
                    && let Some(conversation_id) =
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
            Event::OpenAgentProfileEditor { profile_id } => {
                ctx.emit(pane_group::Event::OpenAgentProfileEditor {
                    profile_id: profile_id.clone(),
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
                // cloud-mode shell. Only reached while
                // `OrchestrationUnifiedStack` is disabled; the unified stack
                // emits `EnsureUnifiedViewerChildPane` instead.
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
            runner_id,
            agent_identity_uid,
        } => {
            let working_dir = group
                .terminal_view_from_pane_id(parent_pane_id, ctx)
                .and_then(|view| view.as_ref(ctx).pwd_if_local(ctx))
                .map(std::path::PathBuf::from)
                .unwrap_or_default();
            launch_remote_child(
                group,
                parent_pane_id,
                request,
                RemoteChildLaunchConfig {
                    environment_id,
                    skill_references,
                    working_dir,
                    model_id,
                    computer_use_enabled,
                    worker_host,
                    harness_type,
                    title,
                    auth_secret_name,
                    runner_id,
                    agent_identity_uid,
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
    let request_id = request.id;
    let parent_conversation_id = request.parent_conversation_id;
    let prompt = request.prompt.clone();

    // Snapshot the host terminal's shared-session source before the spawn
    // so we can cascade it onto the child's source type once the spawn
    // returns.
    let host_source = group
        .terminal_view_from_pane_id(parent_pane_id, ctx)
        .and_then(|view| host_terminal_shared_session_source_type(&view, ctx));

    let launch = prepare_local_oz_child_launch(
        &request.name,
        &request.prompt,
        request.parent_run_id.as_deref(),
        ctx,
    );
    let _ = ctx.spawn(launch, move |group, result, ctx| match result {
        Ok(prepared) => {
            let child_task_id = prepared.task_id;
            let is_shared_session_creator =
                inherit_share_for_local_child(host_source.as_ref(), child_task_id);

            match create_hidden_child_agent_conversation(
                group,
                HiddenChildAgentConversationRequest {
                    parent_pane_id,
                    name: prepared.conversation_name.clone(),
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
                Some(HiddenChildAgentConversation {
                    terminal_view: new_terminal_view,
                    terminal_view_id,
                    conversation_id,
                    ..
                }) => {
                    let scope = ResolvedTeamScope::from_scope(
                        &UserWorkspaces::as_ref(ctx).team_context_for_view(ctx),
                    );
                    apply_child_agent_model_override(
                        &scope,
                        terminal_view_id,
                        model_id.as_deref(),
                        ctx,
                    );
                    finish_local_oz_child_conversation(
                        conversation_id,
                        terminal_view_id,
                        child_task_id,
                        request_id,
                        ctx,
                    );

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
                }
                _ => {
                    let _ = create_error_child_agent_conversation(
                        group,
                        ErrorChildAgentConversationRequest {
                            parent_pane_id,
                            name: prepared.conversation_name,
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
        }
        Err(error) => {
            let _ = create_error_child_agent_conversation(
                group,
                ErrorChildAgentConversationRequest {
                    parent_pane_id,
                    name: normalize_orchestrator_agent_name(&request.name).unwrap_or_default(),
                    parent_conversation_id,
                    request_id: Some(request_id),
                    orchestration_harness: Some(Harness::Oz),
                    error_message: format!("Failed to create local child task: {error}"),
                },
                ctx,
            );
        }
    });
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

                match create_hidden_child_agent_conversation(
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
                    Some(HiddenChildAgentConversation {
                        terminal_view: new_terminal_view,
                        terminal_view_id,
                        conversation_id,
                        ..
                    }) => {
                        let scope = ResolvedTeamScope::from_scope(
                            &UserWorkspaces::as_ref(ctx).team_context_for_view(ctx),
                        );
                        apply_child_agent_model_override(
                            &scope,
                            terminal_view_id,
                            model_id.as_deref(),
                            ctx,
                        );

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
                    }
                    _ => {
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
    config: RemoteChildLaunchConfig,
    ctx: &mut ViewContext<PaneGroup>,
) -> Option<AIConversationId> {
    let request_id = request.id;
    if request.parent_run_id.is_none() {
        report_error!(
            "Remote StartAgent request missing parent_run_id",
            extra: { "parent_conversation_id" => ?request.parent_conversation_id }
        );
        return None;
    }

    let agent_name = normalize_orchestrator_agent_name(&request.name);
    let request_name = agent_name.clone().unwrap_or_default();
    let orchestration_harness = config.orchestration_harness();

    let new_pane_id = group.insert_ambient_agent_pane_hidden_for_child_agent(parent_pane_id, ctx);

    let Some(new_terminal_view) = group.terminal_view_from_pane_id(new_pane_id, ctx) else {
        report_error!("Failed to get terminal view for new remote StartAgent pane");
        group.discard_pane(new_pane_id.into(), ctx);
        return None;
    };

    let terminal_view_id = new_terminal_view.id();
    // Under the unified stack, remote children are not persisted to the local DB
    // (they are re-seeded from the server on restore). Under the flag-off path,
    // children must be persisted so they survive restarts.
    let is_remote = crate::features::FeatureFlag::OrchestrationUnifiedStack.is_enabled();
    let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
        let id = history_model.start_new_child_conversation(
            terminal_view_id,
            request_name.clone(),
            request.parent_conversation_id,
            Some(orchestration_harness),
            is_remote,
            ctx,
        );
        // `start_new_child_conversation` already marked this remote above
        // (before its first persist); this call is now a no-op, kept so the
        // parent's LocalAgentTaskSyncModel skipping status reporting for
        // remote children stays obviously correct at a glance.
        history_model.mark_conversation_as_remote_child(id, ctx);
        id
    });

    BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
        model.record_new_conversation_request_complete(request_id, conversation_id, ctx);
    });

    let prepared = match prepare_remote_child_launch(&request, config, ctx) {
        Ok(prepared) => prepared,
        Err(error) => {
            let error_message = error.user_message();
            report_error!(
                anyhow::Error::new(error).context("Failed to prepare remote child launch"),
                extra: { "conversation_id" => ?conversation_id }
            );
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model.update_conversation_status_with_error(
                    terminal_view_id,
                    conversation_id,
                    ConversationStatus::Error,
                    Some(RenderableAIError::other(error_message, false)),
                    ctx,
                );
            });
            return None;
        }
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
                model.spawn_agent_with_request(prepared.spawn_request, ctx);
            });
        } else {
            report_error!("Remote StartAgent child pane missing ambient agent view model");
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
    use crate::ai::blocklist::maybe_build_ai_query_upsert_event;

    if event
        .terminal_surface_id()
        .is_some_and(|id| id != terminal_view_id)
    {
        return;
    }

    match event {
        BlocklistAIHistoryEvent::AppendedExchange { .. }
        | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. } => {
            // Check if session restoration is enabled.
            if !*GeneralSettings::as_ref(ctx).restore_session
                || !AppExecutionMode::as_ref(ctx).can_save_session()
            {
                return;
            }
            let Some(upsert_ai_query_event) = maybe_build_ai_query_upsert_event(
                event,
                terminal_view_id,
                is_shared_ambient_agent_session,
                ctx,
            ) else {
                return;
            };
            let _ = ctx.spawn(
                // Sending over a sync sender can block the current thread, so we
                // do this async.
                async move { model_event_sender.send(upsert_ai_query_event) },
                move |_, res, _| {
                    if let Err(err) = res {
                        report_error!(
                            anyhow::Error::new(err).context("Error sending upsert AI query event"),
                            extra: { "terminal_pane_id" => ?terminal_pane_id }
                        );
                    }
                },
            );
        }
        BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. }
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
                        report_error!(
                            anyhow::Error::new(err)
                                .context("Error sending delete events for conversation")
                        );
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
        | BlocklistAIHistoryEvent::UpdatedConversationTitle { .. }
        | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
        | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
        | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
        | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces { .. }
        | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
        | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
        | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
        | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => (),
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "terminal_pane_tests.rs"]
mod tests;
