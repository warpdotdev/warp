use async_broadcast::InactiveReceiver;
use parking_lot::FairMutex;
use pathfinder_geometry::vector::Vector2F;
use session_sharing_protocol::common::{
    ActivePrompt, AddGuestsResponse, CLIAgentSessionState, CommandExecutionFailureReason,
    LinkAccessLevelUpdateResponse, RemoveGuestResponse, SelectedAgentModel, SessionId,
    TeamAccessLevelUpdateResponse, UniversalDeveloperInputContextUpdate,
    UpdatePendingUserRoleResponse,
};
use session_sharing_protocol::sharer::SessionSourceType;
use session_sharing_protocol::viewer::SessionEndedReason;
use settings::Setting as _;
use std::any::Any;

use std::sync::Arc;

use warpui::{
    AppContext, ModelContext, ModelHandle, SingletonEntity, ViewHandle, WeakViewHandle, WindowId,
};

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::ai::blocklist::{
    BlocklistAIContextEvent, BlocklistAIContextModel, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel,
};
use crate::ai::llms::{LLMPreferences, LLMPreferencesEvent};
use crate::context_chips::prompt_snapshot::PromptSnapshot;
use crate::context_chips::prompt_type::PromptType;
use crate::features::FeatureFlag;
use crate::pane_group::pane::DetachType;

use crate::network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind};

use crate::settings::{DebugSettings, InputModeSettings, WarpPromptSeparator};
use crate::terminal::event_listener::ChannelEventListener;

use crate::terminal::input::CommandExecutionSource;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::PTY_READS_BROADCAST_CHANNEL_SIZE;

use crate::terminal::session_settings::SessionSettings;

use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::shared_session::manager::Manager;
use crate::terminal::shared_session::permissions_manager::SessionPermissionsManager;
use crate::terminal::shared_session::shared_handlers::{
    apply_auto_approve_agent_actions_update, apply_cli_agent_state_update, apply_input_mode_update,
    apply_selected_agent_model_update, apply_selected_conversation_update,
    build_selected_conversation_update, ActiveRemoteUpdate, RemoteUpdateGuard,
};
use crate::terminal::shared_session::SharedSessionStatus;
use crate::terminal::terminal_manager::{compute_block_size, terminal_colors_list};

use super::event_loop::SharedSessionInitialLoadMode;
use super::network::{
    agent_prompt_failure_reason_string, command_execution_failure_reason_string,
    control_action_failure_reason_string, session_ended_reason_string,
    viewer_removed_reason_string, write_to_pty_failure_reason_string, Network, NetworkEvent,
};
use super::orchestration_viewer_model::OrchestrationViewerModel;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::terminal::view::ambient_agent::is_cloud_agent_pre_first_exchange;
use crate::terminal::view::ExecuteCommandEvent;
use crate::terminal::{Event as TerminalViewEvent, TerminalModel, TerminalView};
use crate::view_components::ToastFlavor;
use crate::{pane_group::TerminalViewResources, terminal::model::session::Sessions};

enum NetworkState {
    /// No viewer network is attached yet; deferred cloud-mode viewers start here until the
    /// follow-up shared session is created.
    Idle,
    Active(ModelHandle<Network>),
    /// Transient state while connecting a viewer network.
    Connecting,
}

struct NetworkResources {
    prompt_type: ModelHandle<PromptType>,
    channel_event_proxy: ChannelEventListener,
}

pub struct TerminalManager {
    model: Arc<FairMutex<TerminalModel>>,
    view: ViewHandle<TerminalView>,

    // We store this here just to keep it from being dropped.
    _model_events: ModelHandle<ModelEventDispatcher>,

    /// An inactive receiver for PTY reads received from the sharer over the network.
    /// We hold onto this so that the broadcast channel isn't closed prematurely.
    _inactive_pty_reads_rx: InactiveReceiver<Arc<Vec<u8>>>,

    /// The network state for the shared session viewer.
    network_state: NetworkState,
    network_resources: NetworkResources,
    current_network: Arc<FairMutex<Option<ModelHandle<Network>>>>,
    viewer_remote_update_guard: RemoteUpdateGuard,
    outbound_handlers_registered: bool,
    /// Owns child discovery + status polling for an orchestrated ambient
    /// agent run. Lazily created in `JoinedSuccessfully` on the first
    /// ambient session join. `Arc<FairMutex<Option<...>>>` matches
    /// `current_network` so the network-event closure can write into it
    /// without `&mut self`. Behind `FeatureFlag::OrchestrationViewerPillBar`.
    orchestration_viewer_model: Arc<FairMutex<Option<ModelHandle<OrchestrationViewerModel>>>>,
    /// `true` for the root viewer pane of an orchestrator, `false` for
    /// per-child viewer panes. Skipping polling on children avoids
    /// duplicated REST traffic and grandchild double-registration via the
    /// transitive `ancestor_run_id` filter.
    enable_orchestration_polling: bool,
}

impl TerminalManager {
    fn send_selected_conversation_update_for_viewer_to_current_network(
        guard: &RemoteUpdateGuard,
        model: &Arc<FairMutex<TerminalModel>>,
        current_network: &Arc<FairMutex<Option<ModelHandle<Network>>>>,
        agent_view_controller: &ModelHandle<AgentViewController>,
        ai_context_model: &ModelHandle<BlocklistAIContextModel>,
        ctx: &mut AppContext,
    ) {
        let Some(update) =
            build_selected_conversation_update(agent_view_controller, ai_context_model, ctx)
        else {
            return;
        };

        Self::send_input_context_update_to_current_network(
            guard,
            model,
            current_network,
            update,
            ctx,
        );
    }

    fn current_network(
        current_network: &Arc<FairMutex<Option<ModelHandle<Network>>>>,
    ) -> Option<ModelHandle<Network>> {
        current_network.lock().clone()
    }

    fn update_current_network(
        current_network: &Arc<FairMutex<Option<ModelHandle<Network>>>>,
        ctx: &mut AppContext,
        update: impl FnOnce(&mut Network, &mut ModelContext<Network>),
    ) {
        let Some(network) = Self::current_network(current_network) else {
            return;
        };
        network.update(ctx, update);
    }

    fn send_input_context_update_to_current_network(
        guard: &RemoteUpdateGuard,
        model: &Arc<FairMutex<TerminalModel>>,
        current_network: &Arc<FairMutex<Option<ModelHandle<Network>>>>,
        update: UniversalDeveloperInputContextUpdate,
        ctx: &mut AppContext,
    ) {
        if !guard.should_broadcast() {
            return;
        }
        if !model.lock().shared_session_status().is_executor() {
            return;
        }

        Self::update_current_network(current_network, ctx, |network, _| {
            network.send_universal_developer_input_context_update(update);
        });
    }

    /// Internal constructor that creates all the models for viewing a shared session. This does not rely on the shared session existing yet.
    fn new_internal(
        resources: TerminalViewResources,
        initial_size: Vector2F,
        window_id: WindowId,
        is_cloud_mode: bool,
        enable_orchestration_polling: bool,
        ctx: &mut AppContext,
    ) -> Self {
        // Create all the necessary channels we need for communication.
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, events_rx) = async_channel::unbounded();
        let (executor_command_tx, _executor_command_rx) = async_channel::unbounded();

        // Although the viewer doesn't have a local PTY, it receives PTY bytes from the sharer
        // over the network. Those bytes are still broadcast through the ChannelEventListener,
        // so we keep an inactive listener alive for PTY recordings and other consumers.
        let (pty_reads_tx, pty_reads_rx) =
            async_broadcast::broadcast(PTY_READS_BROADCAST_CHANNEL_SIZE);
        let inactive_pty_reads_rx = pty_reads_rx.deactivate();

        let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);

        let show_memory_stats = DebugSettings::as_ref(ctx).should_show_memory_stats();

        // TODO: we have to figure out what prompt the viewer will see.
        // For now, just respect the viewer's settings.
        let honor_ps1 = *SessionSettings::as_ref(ctx).honor_ps1;
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let is_inverted = input_mode.is_inverted_blocklist();

        // TODO: use the sharer's size.
        let sizes = compute_block_size(initial_size, ctx);

        let model = if is_cloud_mode {
            TerminalModel::new_for_cloud_mode_shared_session_viewer(
                sizes,
                terminal_colors_list(ctx),
                channel_event_proxy.clone(),
                ctx.background_executor().clone(),
                show_memory_stats,
                honor_ps1,
                is_inverted,
                // When viewing a shared session, we don't want to apply our own
                // secret redaction rules but rather rely on the sharer obfuscating
                // the contents before reaching us.
                ObfuscateSecrets::No,
            )
        } else {
            TerminalModel::new_for_shared_session_viewer(
                sizes,
                terminal_colors_list(ctx),
                channel_event_proxy.clone(),
                ctx.background_executor().clone(),
                show_memory_stats,
                honor_ps1,
                is_inverted,
                // When viewing a shared session, we don't want to apply our own
                // secret redaction rules but rather rely on the sharer obfuscating
                // the contents before reaching us.
                ObfuscateSecrets::No,
            )
        };

        let colors = model.colors();
        let model = Arc::new(FairMutex::new(model));

        let sessions: ModelHandle<Sessions> =
            ctx.add_model(|ctx| Sessions::new(executor_command_tx, ctx));
        let cloned_model = model.clone();
        let model_events =
            ctx.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));
        // The prompt is initially empty until we receive the update from the server.
        let prompt_type =
            ctx.add_model(|_| PromptType::new_static(vec![], false, WarpPromptSeparator::None));

        let view = ctx.add_typed_action_view(window_id, |ctx| {
            let size_info = cloned_model.lock().block_list().size().to_owned();
            TerminalView::new(
                resources,
                wakeups_rx,
                model_events.clone(),
                cloned_model,
                sessions.clone(),
                size_info,
                colors,
                None, // model_event_sender - not used for viewer
                prompt_type.clone(),
                None, // initial_input_config - not used for viewer
                None, // no conversation restoration for shared session viewer
                Some(inactive_pty_reads_rx.clone()),
                is_cloud_mode,
                ctx,
            )
        });

        let terminal_view_id = view.id();
        let agent_view_controller = view.as_ref(ctx).agent_view_controller().clone();
        let active_session = view.as_ref(ctx).active_session().clone();
        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_agent_view_controller(
                &agent_view_controller,
                &active_session,
                terminal_view_id,
                ctx,
            );
        });

        Self {
            model,
            _model_events: model_events,
            view,
            _inactive_pty_reads_rx: inactive_pty_reads_rx,
            network_state: NetworkState::Idle,
            network_resources: NetworkResources {
                prompt_type,
                channel_event_proxy,
            },
            current_network: Arc::new(FairMutex::new(None)),
            viewer_remote_update_guard: RemoteUpdateGuard::new(),
            outbound_handlers_registered: false,
            orchestration_viewer_model: Arc::new(FairMutex::new(None)),
            enable_orchestration_polling,
        }
    }

    /// Create a new terminal manager for viewing a shared session. See
    /// [`Self::enable_orchestration_polling`] for the meaning of the flag.
    pub fn new(
        session_id: SessionId,
        resources: TerminalViewResources,
        initial_size: Vector2F,
        window_id: WindowId,
        enable_orchestration_polling: bool,
        ctx: &mut AppContext,
    ) -> Self {
        let mut terminal_manager = Self::new_internal(
            resources,
            initial_size,
            window_id,
            false,
            enable_orchestration_polling,
            ctx,
        );

        terminal_manager.connect_session(
            session_id,
            SharedSessionInitialLoadMode::ReplaceFromSessionScrollback,
            ctx,
        );

        terminal_manager
    }

    /// Create a new terminal manager for eventually viewing a cloud mode
    /// shared session that is not yet available. See
    /// [`Self::enable_orchestration_polling`] for the meaning of the flag.
    pub fn new_deferred(
        resources: TerminalViewResources,
        initial_size: Vector2F,
        window_id: WindowId,
        enable_orchestration_polling: bool,
        ctx: &mut AppContext,
    ) -> Self {
        Self::new_internal(
            resources,
            initial_size,
            window_id,
            true,
            enable_orchestration_polling,
            ctx,
        )
    }

    /// Connects a deferred terminal manager to a shared session.
    /// This can only be called on a TerminalManager created with `new_deferred`.
    /// Returns `true` if the connection was initiated, `false` if already connected.
    ///
    /// `append_followup_scrollback` controls whether the initial join uses
    /// `AppendFollowupScrollback` mode instead of `ReplaceFromSessionScrollback`.
    /// Local-to-cloud handoff panes set this to `true` so the pre-populated
    /// forked conversation is not replaced by the cloud session's replay
    /// scrollback.
    pub fn connect_to_session(
        &mut self,
        session_id: SessionId,
        append_followup_scrollback: bool,
        ctx: &mut AppContext,
    ) -> bool {
        let load_mode = if append_followup_scrollback {
            SharedSessionInitialLoadMode::AppendFollowupScrollback
        } else {
            SharedSessionInitialLoadMode::ReplaceFromSessionScrollback
        };
        match self.network_state {
            NetworkState::Idle => {
                self.connect_session(session_id, load_mode, ctx);
                true
            }
            NetworkState::Connecting => {
                log::warn!("connect_to_session called while already connecting to shared session");
                false
            }
            NetworkState::Active(_) => false,
        }
    }

    pub fn attach_execution_session(
        &mut self,
        session_id: SessionId,
        ctx: &mut AppContext,
    ) -> bool {
        match std::mem::replace(&mut self.network_state, NetworkState::Connecting) {
            NetworkState::Active(network) => {
                network.update(ctx, |network, _| {
                    network.close_without_reconnection();
                });
                self.model
                    .lock()
                    .clear_write_to_pty_events_for_shared_session_tx();
                *self.current_network.lock() = None;
                self.network_state = NetworkState::Idle;
            }
            NetworkState::Idle => {
                self.network_state = NetworkState::Idle;
            }
            NetworkState::Connecting => {
                self.network_state = NetworkState::Connecting;
                log::warn!(
                    "attach_execution_session called while already connecting to shared session"
                );
                return false;
            }
        }
        self.connect_session(
            session_id,
            SharedSessionInitialLoadMode::AppendFollowupScrollback,
            ctx,
        );
        self.start_cloud_mode_setup_command_tracking();
        true
    }

    pub fn start_cloud_mode_setup_command_tracking(&mut self) {
        if FeatureFlag::CloudModeSetupV2.is_enabled() {
            self.model
                .lock()
                .block_list_mut()
                .set_is_executing_oz_environment_startup_commands(true);
        }
    }

    /// Connects this terminal manager to a shared session.
    /// This method sets up the network model and all associated event handlers.
    fn connect_session(
        &mut self,
        session_id: SessionId,
        initial_load_mode: SharedSessionInitialLoadMode,
        ctx: &mut AppContext,
    ) {
        match std::mem::replace(&mut self.network_state, NetworkState::Connecting) {
            NetworkState::Idle => {}
            other => {
                self.network_state = other;
                log::warn!("connect_session called on already-connected TerminalManager");
                return;
            }
        }

        // Set up the channel for forwarding write-to-pty events over the network to the sharer.
        // Whenever the user writes to a long-running command (e.g. ctrl-c or typing), those bytes
        // are sent from the terminal view through this channel to the network.
        let (write_to_pty_events_tx, write_to_pty_events_rx) = async_channel::unbounded();
        self.model
            .lock()
            .set_write_to_pty_events_for_shared_session_tx(write_to_pty_events_tx);
        self.model
            .lock()
            .set_shared_session_status(SharedSessionStatus::ViewPending);

        let network = ctx.add_model(|ctx| {
            Network::new(
                session_id,
                self.network_resources.channel_event_proxy.clone(),
                self.view.downgrade(),
                self.model.clone(),
                write_to_pty_events_rx,
                initial_load_mode,
                ctx,
            )
        });
        *self.current_network.lock() = Some(network.clone());

        Self::handle_network_events(
            &network,
            &self.view,
            self.model.clone(),
            self.current_network.clone(),
            self.network_resources.prompt_type.clone(),
            self.viewer_remote_update_guard.clone(),
            self.orchestration_viewer_model.clone(),
            self.enable_orchestration_polling,
            ctx,
        );
        if !self.outbound_handlers_registered {
            Self::handle_view_events(
                self.current_network.clone(),
                &self.view,
                self.model.clone(),
                self.viewer_remote_update_guard.clone(),
                ctx,
            );
            Self::handle_network_status_events(&self.view, self.current_network.clone(), ctx);

            // Send model selection updates during session sharing (if viewer has Editor role)
            let current_network_for_models = self.current_network.clone();
            let terminal_view_id = self.view.id();
            let model_clone = self.model.clone();
            let model_remote_update_guard = self.viewer_remote_update_guard.clone();
            ctx.subscribe_to_model(&LLMPreferences::handle(ctx), move |_prefs, event, ctx| {
                // Only react to agent mode LLM changes
                if !matches!(event, LLMPreferencesEvent::UpdatedActiveAgentModeLLM) {
                    return;
                }

                let llm_prefs = &LLMPreferences::as_ref(ctx);
                let selected_model_id: String = llm_prefs
                    .get_active_base_model(ctx, Some(terminal_view_id))
                    .id
                    .clone()
                    .into();

                Self::send_input_context_update_to_current_network(
                    &model_remote_update_guard,
                    &model_clone,
                    &current_network_for_models,
                    UniversalDeveloperInputContextUpdate {
                        selected_model: Some(SelectedAgentModel::new(selected_model_id)),
                        ..Default::default()
                    },
                    ctx,
                );
            });

            // Send input mode updates during session sharing (if viewer has Editor role).
            // When AgentView is enabled, we only send updates when in an active agent view.
            // For ambient agent sessions, input mode is controlled locally, so we skip sending updates.
            let current_network_for_input_mode = self.current_network.clone();
            let model_clone_for_input = self.model.clone();
            let ai_input_model = self.view.as_ref(ctx).ai_input_model().clone();
            let weak_view_for_input_mode = self.view.downgrade();
            let input_mode_remote_update_guard = self.viewer_remote_update_guard.clone();
            ctx.subscribe_to_model(&ai_input_model, move |_, event, ctx| {
                // In ambient agent sessions, input mode is controlled locally.
                if model_clone_for_input
                    .lock()
                    .is_shared_ambient_agent_session()
                {
                    return;
                }

                // When AgentView is enabled, only send input mode updates when in an active agent view.
                if FeatureFlag::AgentView.is_enabled() {
                    let Some(view) = weak_view_for_input_mode.upgrade(ctx) else {
                        return;
                    };
                    let agent_view_controller = view.as_ref(ctx).agent_view_controller().clone();
                    if !agent_view_controller.as_ref(ctx).is_active() {
                        return;
                    }
                }

                let config = event.updated_config();

                Self::send_input_context_update_to_current_network(
                    &input_mode_remote_update_guard,
                    &model_clone_for_input,
                    &current_network_for_input_mode,
                    UniversalDeveloperInputContextUpdate {
                        input_mode: Some((*config).into()),
                        ..Default::default()
                    },
                    ctx,
                );
            });

            let agent_view_controller = self.view.as_ref(ctx).agent_view_controller().clone();
            let ai_context_model = self.view.as_ref(ctx).ai_context_model().clone();
            // Send selected conversation updates during session sharing (if viewer has Editor role)
            if FeatureFlag::AgentView.is_enabled() {
                // When agent view is enabled, we listen to the agent view controller
                // as the authoritative source for which conversation is selected.
                let current_network_for_conversation = self.current_network.clone();
                let model_for_conversation = self.model.clone();
                let ai_context_model_for_conversation = ai_context_model.clone();
                let conversation_remote_update_guard = self.viewer_remote_update_guard.clone();
                ctx.subscribe_to_model(
                    &agent_view_controller,
                    move |agent_view_controller, event, ctx| match event {
                        AgentViewControllerEvent::EnteredAgentView { .. }
                        | AgentViewControllerEvent::ExitedAgentView { .. } => {
                            Self::send_selected_conversation_update_for_viewer_to_current_network(
                                &conversation_remote_update_guard,
                                &model_for_conversation,
                                &current_network_for_conversation,
                                &agent_view_controller,
                                &ai_context_model_for_conversation,
                                ctx,
                            );
                        }
                        AgentViewControllerEvent::ExitConfirmed { .. } => {}
                    },
                );
            } else {
                // When agent view is disabled, we fallback to the legacy behavior
                // of listening for pending query state changes to know which conversation is selected.
                let current_network_for_conversation = self.current_network.clone();
                let model_for_conversation = self.model.clone();
                let agent_view_controller_for_conversation = agent_view_controller.clone();
                let conversation_remote_update_guard = self.viewer_remote_update_guard.clone();
                ctx.subscribe_to_model(&ai_context_model, move |ai_context_model, event, ctx| {
                    if !matches!(event, BlocklistAIContextEvent::PendingQueryStateUpdated) {
                        return;
                    }

                    Self::send_selected_conversation_update_for_viewer_to_current_network(
                        &conversation_remote_update_guard,
                        &model_for_conversation,
                        &current_network_for_conversation,
                        &agent_view_controller_for_conversation,
                        &ai_context_model,
                        ctx,
                    );
                });
            }

            // Send auto-approve updates during session sharing (if viewer has Editor role)
            let current_network_for_auto = self.current_network.clone();
            let model_clone_for_auto = self.model.clone();
            let view_id_for_auto = self.view.id();
            let weak_view_for_auto = self.view.downgrade();
            let auto_approve_remote_update_guard = self.viewer_remote_update_guard.clone();
            ctx.subscribe_to_model(
                &BlocklistAIHistoryModel::handle(ctx),
                move |_, event, ctx| {
                    // We intentionally keep this as a full match so new variants
                    // are forced to be handled here
                    #[allow(clippy::single_match)]
                    match event {
                        BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride {
                            terminal_view_id,
                        } => {
                            if *terminal_view_id != view_id_for_auto {
                                return;
                            }

                            let Some(view) = weak_view_for_auto.upgrade(ctx) else {
                                return;
                            };

                            let auto_approve = view
                                .as_ref(ctx)
                                .ai_context_model()
                                .as_ref(ctx)
                                .pending_query_autoexecute_override(ctx)
                                .is_autoexecute_any_action();
                            Self::send_input_context_update_to_current_network(
                                &auto_approve_remote_update_guard,
                                &model_clone_for_auto,
                                &current_network_for_auto,
                                UniversalDeveloperInputContextUpdate {
                                    auto_approve_agent_actions: Some(auto_approve),
                                    ..Default::default()
                                },
                                ctx,
                            );
                        }
                        _ => {}
                    }
                },
            );

            // Broadcast CLI agent rich input open/close changes from viewer back to sharer.
            let current_network_for_cli = self.current_network.clone();
            let model_for_cli = self.model.clone();
            let view_id_for_cli = self.view.id();
            let cli_remote_update_guard = self.viewer_remote_update_guard.clone();
            ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, ctx| {
                let CLIAgentSessionsModelEvent::InputSessionChanged {
                    terminal_view_id,
                    new_input_state,
                    ..
                } = event
                else {
                    return;
                };
                if *terminal_view_id != view_id_for_cli
                    || !cli_remote_update_guard.should_broadcast()
                {
                    return;
                }
                let cli_agent_session = {
                    let sessions_model = CLIAgentSessionsModel::as_ref(ctx);
                    match sessions_model.session(view_id_for_cli) {
                        Some(session) => CLIAgentSessionState::Active {
                            cli_agent: session.agent.to_serialized_name(),
                            is_rich_input_open: matches!(
                                new_input_state,
                                CLIAgentInputState::Open { .. }
                            ),
                        },
                        None => CLIAgentSessionState::Inactive,
                    }
                };
                Self::send_input_context_update_to_current_network(
                    &cli_remote_update_guard,
                    &model_for_cli,
                    &current_network_for_cli,
                    UniversalDeveloperInputContextUpdate {
                        cli_agent_session: Some(cli_agent_session),
                        ..Default::default()
                    },
                    ctx,
                );
            });

            self.outbound_handlers_registered = true;
        }
        self.network_state = NetworkState::Active(network);
    }

    // Aggregating these into a struct would just shift the same set of
    // fields into another type purely to placate Clippy without any
    // readability win, since the closure body still needs each clone
    // individually. Suppress the lint instead.
    #[allow(clippy::too_many_arguments)]
    fn handle_network_events(
        network: &ModelHandle<Network>,
        view: &ViewHandle<TerminalView>,
        model: Arc<FairMutex<TerminalModel>>,
        current_network: Arc<FairMutex<Option<ModelHandle<Network>>>>,
        prompt_type: ModelHandle<PromptType>,
        viewer_remote_update_guard: RemoteUpdateGuard,
        orchestration_viewer_model: Arc<FairMutex<Option<ModelHandle<OrchestrationViewerModel>>>>,
        enable_orchestration_polling: bool,
        ctx: &mut AppContext,
    ) {
        // We use a weak view handle instead of a strong reference because we may add a subscription to the view which moves a strong reference of the Model into the callback,
        // which would create a reference cycle and cause a memory leak. Instead, upgrade the weak view handle lazily.
        let weak_view_handle = view.downgrade();

        ctx.subscribe_to_model(network, move |network, event, ctx| match event {
            NetworkEvent::JoinedSuccessfully {
                active_prompt,
                viewer_id,
                viewer_firebase_uid,
                participant_list,
                input_replica_id,
                universal_developer_input_context,
                source_type,
            } => {
                model
                    .lock()
                    .set_shared_session_source_type(source_type.clone());

                Self::handle_active_prompt_update(
                    model.clone(),
                    prompt_type.clone(),
                    weak_view_handle.clone(),
                    active_prompt,
                    ctx,
                );

                // Apply the universal developer input context if present.
                let active_remote_update = viewer_remote_update_guard.start_remote_update();
                if let Some(universal_developer_input_context) = universal_developer_input_context {
                    if let Some(ref model) = universal_developer_input_context.selected_model {
                        Self::handle_selected_agent_model_update(&weak_view_handle, model, &active_remote_update, ctx);
                    }
                    if let Some(ref input_mode) = universal_developer_input_context.input_mode {
                        Self::handle_input_mode_update(&weak_view_handle, input_mode, &active_remote_update, ctx);
                    }
                    apply_cli_agent_state_update(
                        &weak_view_handle,
                        &universal_developer_input_context.cli_agent_session,
                        &active_remote_update,
                        ctx,
                    );
                    if let Some(ref selected_conversation) = universal_developer_input_context.selected_conversation {
                        Self::handle_selected_conversation_update(
                            &weak_view_handle,
                            selected_conversation,
                            &active_remote_update,
                            ctx,
                        );
                    }
                }
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };

                let ambient_task_id: Option<AmbientAgentTaskId> = match &source_type {
                    SessionSourceType::AmbientAgent { task_id } => {
                        task_id.as_deref().and_then(|s| s.parse().ok())
                    }
                    _ => None,
                };

                // Mark terminal view as a shared ambient agent session view.
                if matches!(&source_type, SessionSourceType::AmbientAgent { .. }) {
                    let terminal_view_id = view.id();
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _ctx| {
                        history.mark_terminal_view_as_ambient_agent_session_view(terminal_view_id);
                    });

                    // Register this ambient session as active for conversation list tracking.
                    if let Some(task_id) = ambient_task_id {
                        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
                            model.register_ambient_session(terminal_view_id, task_id, ctx);
                        });

                        // Spin up the orchestration viewer model on first
                        // join (`is_none()` guards against reconnect dupes).
                        if enable_orchestration_polling
                            && FeatureFlag::OrchestrationViewerPillBar.is_enabled()
                            && orchestration_viewer_model.lock().is_none()
                        {
                            let weak_view_handle_for_orch = weak_view_handle.clone();
                            let orchestration_viewer_model_slot =
                                orchestration_viewer_model.clone();
                            let handle = ctx.add_model(|model_ctx| {
                                OrchestrationViewerModel::new(
                                    task_id,
                                    terminal_view_id,
                                    weak_view_handle_for_orch,
                                    model_ctx,
                                )
                            });
                            *orchestration_viewer_model_slot.lock() = Some(handle);
                        }
                    }
                }

                let session_id = network.as_ref(ctx).session_id();
                Manager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.joined_share(weak_view_handle.clone(), session_id, ctx);
                });

                view.update(ctx, |terminal_view, ctx| {
                    if let Some(task_id) = ambient_task_id {
                        if let Some(ambient_agent_view_model) =
                            terminal_view.ambient_agent_view_model()
                        {
                            ambient_agent_view_model.update(ctx, |model, ctx| {
                                model.enter_viewing_existing_session(task_id, ctx);
                            });
                        }
                    }

                    terminal_view.on_session_share_joined(
                        viewer_id.clone(),
                        *viewer_firebase_uid,
                        input_replica_id.clone(),
                        participant_list.clone(),
                        session_id,
                        source_type.clone(),
                        ctx,
                    );
                });

                #[cfg(target_family = "wasm")]
                crate::platform::wasm::emit_event(crate::platform::wasm::WarpEvent::SessionJoined);
            }
            NetworkEvent::SessionEnded { reason } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                let is_ambient_agent = model.lock().is_shared_ambient_agent_session();
                if is_ambient_agent {
                    if !Self::end_current_ambient_session(
                        &view,
                        model.clone(),
                        &current_network,
                        &network,
                        ctx,
                    ) {
                        return;
                    }
                    // Non-owner viewers (read-only) won't get a follow-up
                    // session; owners may handoff via
                    // `attach_execution_session` (same `TerminalManager`,
                    // same orchestrator `task_id`), so keep their model.
                    let is_owner = view.read(ctx, |terminal_view, app| {
                        terminal_view.owned_ambient_agent_task_id(app).is_some()
                    });
                    if !is_owner {
                        Self::stop_orchestration_polling(&orchestration_viewer_model);
                    }
                } else {
                    Self::stop_orchestration_polling(&orchestration_viewer_model);
                    Self::shared_session_ended(&view, model.clone(), ctx);
                }
                view.update(ctx, |terminal_view, ctx| {
                    let reason_string = session_ended_reason_string(reason);
                    match reason {
                        SessionEndedReason::EndedBySharer
                        | SessionEndedReason::ExceededSizeLimit => {}
                        SessionEndedReason::InactivityLimitReached => {
                            terminal_view.show_persistent_toast(
                                reason_string,
                                ToastFlavor::Error,
                                ctx,
                            );
                        }
                        SessionEndedReason::InternalServerError if is_ambient_agent => {
                            // Don't show toast for cloud mode sessions - the error message
                            // "ask sharer to reshare" doesn't apply.
                        }
                        _ => {
                            terminal_view.show_persistent_toast(
                                reason_string,
                                ToastFlavor::Error,
                                ctx,
                            );
                        }
                    }
                });
            }
            NetworkEvent::ViewerRemoved { reason } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                // Viewer has been removed and will not re-attach; stop the
                // children-polling background work.
                Self::stop_orchestration_polling(&orchestration_viewer_model);
                Self::shared_session_ended(&view, model.clone(), ctx);
                view.update(ctx, |terminal_view, ctx| {
                    let reason_string = viewer_removed_reason_string(reason);
                    terminal_view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);
                });
            }
            NetworkEvent::FailedToJoin { reason } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| {
                    terminal_view.show_persistent_toast(
                        reason.user_facing_error_message().to_string(),
                        ToastFlavor::Error,
                        ctx,
                    );
                });
            }
            NetworkEvent::FailedToReconnect => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                // Reconnection has been abandoned; stop the children-polling
                // background work.
                Self::stop_orchestration_polling(&orchestration_viewer_model);
                Self::shared_session_ended(&view, model.clone(), ctx);
                view.update(ctx, |terminal_view, ctx| {
                    terminal_view.show_persistent_toast(
                        "Failed to reconnect. Please try again later.".to_owned(),
                        ToastFlavor::Error,
                        ctx,
                    );
                });
            }
            NetworkEvent::SharerActivePromptUpdated(active_prompt_update) => {
                Self::handle_active_prompt_update(
                    model.clone(),
                    prompt_type.clone(),
                    weak_view_handle.clone(),
                    &active_prompt_update.active_prompt,
                    ctx,
                );
            }
            NetworkEvent::UniversalDeveloperInputContextUpdated(context_update) => {
                let active_remote_update = viewer_remote_update_guard.start_remote_update();

                if let Some(ref model) = context_update.selected_model {
                    Self::handle_selected_agent_model_update(&weak_view_handle, model, &active_remote_update, ctx);
                }
                if let Some(ref input_mode) = context_update.input_mode {
                    Self::handle_input_mode_update(&weak_view_handle, input_mode, &active_remote_update, ctx);
                }
                if let Some(ref selected_conversation) = context_update.selected_conversation {
                    Self::handle_selected_conversation_update(
                        &weak_view_handle,
                        selected_conversation,
                        &active_remote_update,
                        ctx,
                    );
                }
                if let Some(auto_approve) = context_update.auto_approve_agent_actions {
                    apply_auto_approve_agent_actions_update(&weak_view_handle, auto_approve, &active_remote_update, ctx);
                }

                if model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_active_and_long_running()
                {
                    if let Some(interaction_state) =
                        context_update.long_running_command_agent_interaction_state
                    {
                        log::info!(
                            "[viewer] UniversalDeveloperInputContextUpdated: \
                             applying LRC interaction_state={interaction_state:?}"
                        );
                        if let Some(view) = weak_view_handle.upgrade(ctx) {
                            view.update(ctx, |view, ctx| {
                                view.apply_long_running_command_agent_interaction_state(
                                    interaction_state,
                                    ctx,
                                );
                            });
                        }
                    }
                }

                if let Some(ref cli_agent_session) = context_update.cli_agent_session {
                    apply_cli_agent_state_update(
                        &weak_view_handle,
                        cli_agent_session,
                        &active_remote_update,
                        ctx,
                    );
                }
            }
            NetworkEvent::Reconnecting => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |view, ctx| {
                    view.on_shared_session_reconnection_status_changed(true, ctx)
                });
            }
            NetworkEvent::ParticipantListUpdated(participant_list) => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };

                // A change to our role may have originated from the server,
                // make sure that our own state changes if it does.
                view.update(ctx, |view, ctx| {
                    view.on_self_role_maybe_changed(participant_list.as_ref(), ctx);
                });

                if let Some(presence_manager) = view.as_ref(ctx).shared_session_presence_manager() {
                    presence_manager.update(ctx, |presence_manager, ctx| {
                        presence_manager.update_participants(*participant_list.clone(), ctx)
                    });
                };

                if let Some(session_id) = view.as_ref(ctx).shared_session_id().cloned() {
                    SessionPermissionsManager::handle(ctx).update(
                        ctx,
                        |permissions_manager, ctx| {
                            permissions_manager.updated_guests(
                                ctx,
                                session_id,
                                participant_list.guests.clone(),
                                participant_list.pending_guests.clone(),
                            );
                        },
                    );
                }
            }
            NetworkEvent::ParticipantPresenceUpdated(update) => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };

                view.update(ctx, |view, ctx| {
                    view.on_participant_presence_updated(update, ctx);
                });
            }
            NetworkEvent::ReconnectedSuccessfully => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };

                view.update(ctx, |view, ctx| {
                    view.on_shared_session_reconnection_status_changed(false, ctx)
                });
            }
            NetworkEvent::ParticipantRoleChanged {
                participant_id,
                reason,
                role,
            } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };

                view.update(ctx, |view, ctx| {
                    view.maybe_show_role_changed_toast(participant_id, *reason, *role, ctx);
                    view.on_participant_role_changed(participant_id, *role, ctx);
                });
            }
            NetworkEvent::InputUpdated {
                block_id,
                operations,
            } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };

                view.update(ctx, |view, ctx| {
                    // In cloud-mode startup (before the first exchange), shared-session input
                    // sync reflects environment setup commands. Skip applying remote edits so
                    // the visible input isn't populated with setup-command text.
                    if FeatureFlag::CloudModeSetupV2.is_enabled()
                        && {
                            let model = view.model.lock();
                            is_cloud_agent_pre_first_exchange(
                                view.ambient_agent_view_model(),
                                view.agent_view_controller(),
                                &model,
                                ctx,
                            )
                        }
                    {
                        return;
                    }
                    view.apply_viewer_shared_session_input_update(block_id, operations.clone(), ctx);
                })
            }
            NetworkEvent::RoleRequestInFlight(role_request_id) => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };

                view.update(ctx, |view, ctx| {
                    view.on_shared_session_viewer_role_request_in_flight(
                        role_request_id.clone(),
                        ctx,
                    );
                });
            }
            NetworkEvent::RoleRequestResponse(role_request_response) => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };

                view.update(ctx, |view, ctx| {
                    view.on_shared_session_role_request_response(
                        role_request_response.clone(),
                        ctx,
                    );
                });
            }
            NetworkEvent::CommandExecutionRequestFailed { reason, .. } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| {
                    let reason_string = command_execution_failure_reason_string(reason);
                    terminal_view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);

                    // On command execution request, the input is frozen and set to a loading state.
                    // We only need to restore the input for errors that aren't the result of a new buffer.
                    if matches!(
                        reason,
                        CommandExecutionFailureReason::InsufficientPermissions
                    ) {
                        terminal_view.input().update(ctx, |input, ctx| {
                            input.on_execute_command_for_shared_session_participant_failure(ctx);
                        })
                    }
                });
            }
            NetworkEvent::WriteToPtyRequestFailed { reason } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| {
                    let reason_string = write_to_pty_failure_reason_string(reason);
                    terminal_view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);
                });
            }
            NetworkEvent::AgentPromptRequestInFlight(_id) => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| {
                    terminal_view.input().update(ctx, |input, ctx| {
                        input.unfreeze_and_clear_agent_input(ctx);
                    });
                });
            }
            NetworkEvent::AgentPromptRequestFailed { reason, .. } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| {
                    let reason_string = agent_prompt_failure_reason_string(reason);
                    terminal_view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);

                    terminal_view.input().update(ctx, |input, ctx| {
                        input.unfreeze_and_clear_agent_input(ctx);
                    });
                });
            }
            NetworkEvent::ControlActionRequestFailed { reason } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| {
                    let reason_string = control_action_failure_reason_string(reason);
                    terminal_view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);
                });
            }
            NetworkEvent::LinkAccessLevelUpdated { role } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| {
                    let Some(session_id) = terminal_view.shared_session_id() else {
                        return;
                    };
                    SessionPermissionsManager::handle(ctx).update(
                        ctx,
                        |permissions_manager, ctx| {
                            permissions_manager.updated_link_permissions(*session_id, *role, ctx);
                        },
                    );
                });
            }
            NetworkEvent::TeamAccessLevelUpdated { team_acl } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| {
                    let Some(session_id) = terminal_view.shared_session_id() else {
                        return;
                    };
                    SessionPermissionsManager::handle(ctx).update(
                        ctx,
                        |permissions_manager, ctx| {
                            permissions_manager.updated_team_permissions(
                                *session_id,
                                team_acl.clone(),
                                ctx,
                            );
                        },
                    );
                });
            }
            NetworkEvent::LinkAccessLevelUpdateResponse { response } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| match response {
                    LinkAccessLevelUpdateResponse::Ok { role } => {
                        let Some(session_id) = terminal_view.shared_session_id() else {
                            return;
                        };
                        SessionPermissionsManager::handle(ctx).update(
                            ctx,
                            |permissions_manager, ctx| {
                                permissions_manager.updated_link_permissions(
                                    *session_id,
                                    *role,
                                    ctx,
                                );
                            },
                        );
                    }
                    LinkAccessLevelUpdateResponse::Error => {
                        terminal_view.show_persistent_toast(
                            "Failed to update permissions for shared session".to_owned(),
                            ToastFlavor::Error,
                            ctx,
                        );
                    }
                });
            }
            NetworkEvent::TeamAccessLevelUpdateResponse { response } => {
                let Some(view) = weak_view_handle.upgrade(ctx) else {
                    return;
                };
                view.update(ctx, |terminal_view, ctx| match response {
                    TeamAccessLevelUpdateResponse::Success { team_acl, .. } => {
                        let Some(session_id) = terminal_view.shared_session_id() else {
                            return;
                        };
                        SessionPermissionsManager::handle(ctx).update(
                            ctx,
                            |permissions_manager, ctx| {
                                permissions_manager.updated_team_permissions(
                                    *session_id,
                                    team_acl.clone(),
                                    ctx,
                                );
                            },
                        );
                    }
                    TeamAccessLevelUpdateResponse::Error(_) => {
                        terminal_view.show_persistent_toast(
                            "Something went wrong. Please try again.".to_owned(),
                            ToastFlavor::Error,
                            ctx,
                        );
                    }
                });
            }
            NetworkEvent::AddGuestsResponse { response } => {
                if let AddGuestsResponse::Error(reason) = response {
                    let Some(view) = weak_view_handle.upgrade(ctx) else {
                        return;
                    };
                    view.update(ctx, |terminal_view, ctx| {
                        let reason_string = match reason {
                            session_sharing_protocol::common::FailedToAddGuestsReason::NotWarpUsers => {
                                "One or more of the emails are not Warp users.".to_owned()
                            }
                            session_sharing_protocol::common::FailedToAddGuestsReason::GuestAlreadyAdded => {
                                "One or more of the guests has already been added.".to_owned()
                            }
                            _ => "Something went wrong. Please try again.".to_owned(),
                        };
                        terminal_view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);
                    });
                }
            }
            NetworkEvent::RemoveGuestResponse { response } => {
                if let RemoveGuestResponse::Error(_) = response {
                    let Some(view) = weak_view_handle.upgrade(ctx) else {
                        return;
                    };
                    view.update(ctx, |terminal_view, ctx| {
                        terminal_view.show_persistent_toast(
                            "Something went wrong. Please try again.".to_owned(),
                            ToastFlavor::Error,
                            ctx,
                        );
                    });
                }
            }
            NetworkEvent::UpdatePendingUserRoleResponse { response } => {
                if let UpdatePendingUserRoleResponse::Error(_) = response {
                    let Some(view) = weak_view_handle.upgrade(ctx) else {
                        return;
                    };
                    view.update(ctx, |terminal_view, ctx| {
                        terminal_view.show_persistent_toast(
                            "Something went wrong. Please try again.".to_owned(),
                            ToastFlavor::Error,
                            ctx,
                        );
                    });
                }
            }
        });
    }

    fn handle_active_prompt_update(
        model: Arc<FairMutex<TerminalModel>>,
        prompt_type: ModelHandle<PromptType>,
        weak_view_handle: WeakViewHandle<TerminalView>,
        active_prompt: &ActivePrompt,
        ctx: &mut AppContext,
    ) {
        let mut model = model.lock();
        match active_prompt {
            ActivePrompt::WarpPrompt(serialized_prompt_snapshot) => {
                match serde_json::from_str::<PromptSnapshot>(serialized_prompt_snapshot) {
                    Ok(prompt_snapshot) => {
                        model.block_list_mut().set_honor_ps1(false);
                        // Overwrite the static prompt with the new snapshot.
                        prompt_type.update(ctx, |prompt_type, ctx| {
                            if let PromptType::Static { snapshot } = prompt_type {
                                *snapshot = prompt_snapshot;
                                ctx.notify();
                            } else {
                                log::warn!("Received ActivePrompt::WarpPrompt updated but prompt type is not Static");
                            }
                        });
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to deserialize prompt snapshot from shared session server: {e}"
                        )
                    }
                }
            }
            ActivePrompt::PS1 => {
                // The viewer already receives bytes from the pty for the PS1 prompt, so we only need to choose to render it.
                model.block_list_mut().set_honor_ps1(true);
            }
        }
        let Some(view) = weak_view_handle.upgrade(ctx) else {
            return;
        };
        // This is needed to re-render the input if we changed prompt types.
        view.update(ctx, |view, ctx| {
            view.input().update(ctx, |input, ctx| {
                input.notify_and_notify_children(ctx);
            })
        });
    }

    fn handle_selected_agent_model_update(
        weak_view_handle: &WeakViewHandle<TerminalView>,
        selected_model: &SelectedAgentModel,
        guard: &ActiveRemoteUpdate,
        ctx: &mut AppContext,
    ) {
        let Some(view) = weak_view_handle.upgrade(ctx) else {
            return;
        };

        let terminal_view_id = view.id();
        apply_selected_agent_model_update(terminal_view_id, selected_model, guard, ctx);
    }

    fn handle_input_mode_update(
        weak_view_handle: &WeakViewHandle<TerminalView>,
        input_mode: &session_sharing_protocol::common::InputMode,
        guard: &ActiveRemoteUpdate,
        ctx: &mut AppContext,
    ) {
        let Some(view) = weak_view_handle.upgrade(ctx) else {
            return;
        };
        // During cloud startup (pre-first-exchange), keep local input mode stable
        // and ignore remote shell/ai mode toggles from session-sharing context sync.
        let is_pre_first_exchange = FeatureFlag::CloudModeSetupV2.is_enabled() && {
            let view_ref = view.as_ref(ctx);
            let model = view_ref.model.lock();
            is_cloud_agent_pre_first_exchange(
                view_ref.ambient_agent_view_model(),
                view_ref.agent_view_controller(),
                &model,
                ctx,
            )
        };
        let suppress_input_mode_update =
            view.as_ref(ctx).is_shared_ambient_agent_session() || is_pre_first_exchange;
        if suppress_input_mode_update {
            return;
        }
        apply_input_mode_update(weak_view_handle, input_mode, guard, ctx);
    }

    fn handle_selected_conversation_update(
        weak_view_handle: &WeakViewHandle<TerminalView>,
        selected_conversation: &session_sharing_protocol::common::SelectedConversation,
        guard: &ActiveRemoteUpdate,
        ctx: &mut AppContext,
    ) {
        apply_selected_conversation_update(weak_view_handle, selected_conversation, guard, ctx);
    }

    fn handle_view_events(
        current_network: Arc<FairMutex<Option<ModelHandle<Network>>>>,
        view: &ViewHandle<TerminalView>,
        model: Arc<FairMutex<TerminalModel>>,
        viewer_remote_update_guard: RemoteUpdateGuard,
        ctx: &mut AppContext,
    ) {
        ctx.subscribe_to_view(view, move |view, event, ctx| match event {
            TerminalViewEvent::SelectedBlocksChanged | TerminalViewEvent::SelectedTextChanged => {
                let selection = view.read(ctx, |view, ctx| {
                    view.get_shared_session_presence_selection(ctx)
                });
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_presence_selection_if_changed(selection);
                });
            }
            TerminalViewEvent::RequestSharedSessionRole(role) => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_role_request(*role);
                });
            }
            TerminalViewEvent::CancelRoleRequest(role_request_id) => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_cancel_role_request(role_request_id.clone());
                });
            }
            TerminalViewEvent::InputEditorUpdated {
                block_id,
                operations,
            } => {
                let should_send_input_update = view.read(ctx, |view, ctx| {
                    let model = model.lock();
                    model.block_list().active_block_id() == block_id
                        && model.shared_session_status().is_executor()
                        && view.should_publish_shared_session_input_editor_update(&model, ctx)
                });
                if should_send_input_update {
                    Self::update_current_network(&current_network, ctx, |network, _| {
                        network.send_input_update(block_id, operations.iter());
                    });
                }
            }
            TerminalViewEvent::ExecuteCommand(ExecuteCommandEvent {
                command, source, ..
            }) => {
                // For a viewer, only the SharedSession execution source is valid.
                let CommandExecutionSource::SharedSession { block_id, .. } = source
                else {
                    log::warn!("Got a TerminalViewEvent::ExecuteCommand in viewer::TerminalManager where the source was not SharedSession");
                    return;
                };

                // If the block ID has become stale by the time we get here,
                // we don't need to send this update to the server.
                if model.lock().block_list().active_block_id() != block_id {
                    return;
                }

                // Only send command execution request if the viewer is an executor.
                if model.lock().shared_session_status().is_executor() {
                    Self::update_current_network(&current_network, ctx, |network, _| {
                        network.send_command_execution_request(block_id, command.to_owned());
                    });
                }
            }
            TerminalViewEvent::RejoinCurrentSession => {
                Self::update_current_network(&current_network, ctx, |network, ctx| {
                    network.reauthenticate_viewer(ctx);
                });
            }
            TerminalViewEvent::SendAgentPrompt {
                server_conversation_token,
                prompt,
                attachments,
            } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_agent_prompt_request(
                        *server_conversation_token,
                        prompt.clone(),
                        attachments.clone(),
                    );
                });
            }
            TerminalViewEvent::CancelSharedSessionConversation {
                server_conversation_token,
            } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_cancel_control_action(*server_conversation_token);
                });
            }
            TerminalViewEvent::ReportViewerTerminalSize { window_size } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_report_terminal_size(*window_size);
                });
            }
            TerminalViewEvent::LongRunningCommandAgentInteractionStateChanged { state } => {
                Self::send_input_context_update_to_current_network(
                    &viewer_remote_update_guard,
                    &model,
                    &current_network,
                    UniversalDeveloperInputContextUpdate {
                        long_running_command_agent_interaction_state: Some(*state),
                        ..Default::default()
                    },
                    ctx,
                );
            }
            TerminalViewEvent::UpdateSessionLinkPermissions { role } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_link_permission_update(*role);
                });
            }
            TerminalViewEvent::UpdateSessionTeamPermissions { role, team_uid } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_team_permission_update(*role, team_uid.clone());
                });
            }
            TerminalViewEvent::AddGuests { emails, role } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_add_guests(emails.clone(), *role);
                });
            }
            TerminalViewEvent::RemoveGuest { user_uid } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_remove_guest(*user_uid);
                });
            }
            TerminalViewEvent::RemovePendingGuest { email } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_remove_pending_guest(email.clone());
                });
            }
            TerminalViewEvent::UpdateUserRole { user_uid, role } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_user_role_update(*user_uid, *role);
                });
            }
            TerminalViewEvent::UpdatePendingUserRole { email, role } => {
                Self::update_current_network(&current_network, ctx, |network, _| {
                    network.send_pending_user_role_update(email.clone(), *role);
                });
            }
            _ => (),
        });
    }

    fn handle_network_status_events(
        view: &ViewHandle<TerminalView>,
        current_network: Arc<FairMutex<Option<ModelHandle<Network>>>>,
        ctx: &mut AppContext,
    ) {
        let weak_view_handle = view.downgrade();
        let network_status = NetworkStatus::handle(ctx);

        ctx.subscribe_to_model(&network_status, move |_, event, ctx| {
            let Some(view) = weak_view_handle.upgrade(ctx) else {
                return;
            };
            let NetworkStatusEvent::NetworkStatusChanged { new_status } = event;
            match new_status {
                NetworkStatusKind::Online => {
                    if Self::current_network(&current_network)
                        .is_some_and(|network| network.as_ref(ctx).is_connected())
                    {
                        view.update(ctx, |view, ctx| {
                            view.on_shared_session_reconnection_status_changed(false, ctx)
                        });
                    }
                }
                NetworkStatusKind::Offline => {
                    view.update(ctx, |view, ctx| {
                        view.on_shared_session_reconnection_status_changed(true, ctx)
                    });
                }
            }
        });
    }

    /// Drops the [`OrchestrationViewerModel`] from the shared slot if one
    /// exists. Called from terminal session-end paths. The model's
    /// `ctx.spawn` continuations are entity-scoped, so dropping the
    /// entity makes them no-ops; no explicit `.abort()` needed.
    fn stop_orchestration_polling(
        orchestration_viewer_model: &Arc<FairMutex<Option<ModelHandle<OrchestrationViewerModel>>>>,
    ) {
        *orchestration_viewer_model.lock() = None;
    }

    fn shared_session_ended(
        terminal_view: &ViewHandle<TerminalView>,
        model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut AppContext,
    ) {
        let terminal_view_id = terminal_view.id();

        // When a shared session ends for a viewer, cancel any in-progress conversations.
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            history_model
                .all_live_conversations_for_terminal_view(terminal_view_id)
                .filter(|conversation| conversation.status().is_in_progress())
                .map(|conversation| conversation.id())
                .collect::<Vec<_>>()
                .into_iter()
                .for_each(|conversation_id| {
                    history_model.update_conversation_status(
                        terminal_view_id,
                        conversation_id,
                        ConversationStatus::Cancelled,
                        ctx,
                    )
                });
        });

        Manager::handle(ctx).update(ctx, |manager, _| {
            manager.left_share(terminal_view_id);
        });

        terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.on_session_share_ended(ctx);
        });

        model
            .lock()
            .set_shared_session_status(SharedSessionStatus::FinishedViewer);
        model
            .lock()
            .clear_write_to_pty_events_for_shared_session_tx();
    }

    fn end_current_ambient_session(
        terminal_view: &ViewHandle<TerminalView>,
        model: Arc<FairMutex<TerminalModel>>,
        current_network: &Arc<FairMutex<Option<ModelHandle<Network>>>>,
        ended_network: &ModelHandle<Network>,
        ctx: &mut AppContext,
    ) -> bool {
        let ended_session_id = ended_network.as_ref(ctx).session_id();
        if !Self::current_network(current_network)
            .is_some_and(|network| network.as_ref(ctx).session_id() == ended_session_id)
        {
            return false;
        }
        Manager::handle(ctx).update(ctx, |manager, _| {
            manager.left_share(terminal_view.id());
        });

        model
            .lock()
            .clear_write_to_pty_events_for_shared_session_tx();
        if FeatureFlag::HandoffCloudCloud.is_enabled() {
            terminal_view.update(ctx, |terminal_view, ctx| {
                // Owned ambient tasks remain editable Cloud Mode panes after their live shared
                // session ends; non-owners are still read-only viewers of a finished session.
                let owns_ambient_task = terminal_view.owned_ambient_agent_task_id(ctx).is_some();
                model
                    .lock()
                    .set_shared_session_status(if owns_ambient_task {
                        SharedSessionStatus::NotShared
                    } else {
                        SharedSessionStatus::FinishedViewer
                    });
                if let Some(ambient_agent_view_model) =
                    terminal_view.ambient_agent_view_model().cloned()
                {
                    ambient_agent_view_model.update(ctx, |model, ctx| {
                        model.record_ambient_execution_ended(ended_session_id, ctx);
                    });
                }
                terminal_view.on_ambient_agent_execution_ended(ctx);
            });
        }
        if Self::current_network(current_network)
            .is_some_and(|network| network.as_ref(ctx).session_id() == ended_session_id)
        {
            *current_network.lock() = None;
        }
        true
    }
}

impl crate::terminal::TerminalManager for TerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn view(&self) -> ViewHandle<TerminalView> {
        self.view.clone()
    }

    fn on_view_detached(&self, detach_type: DetachType, app: &mut AppContext) {
        // Keep the network + shared-session state alive for non-permanent detaches:
        // - `HiddenForClose`: the pane may be restored from the undo-close stack. If the tab is
        //   never restored, we'll be invoked again with `Closed` from the grace-period expiry
        //   and tear down then.
        // - `Moved`: the same `TerminalManager` is reused in the target pane group (the
        //   `Box<dyn AnyPaneContent>` is transferred via `remove_pane_for_move` and then
        //   immediately re-attached), so tearing down the network would break the live session.
        if !matches!(detach_type, DetachType::Closed) {
            return;
        }

        let terminal_view_id = self.view.id();
        ActiveAgentViewsModel::handle(app).update(app, |model, ctx| {
            model.unregister_agent_view_controller(terminal_view_id, ctx);
            model.unregister_ambient_session(terminal_view_id, ctx);
        });

        if let NetworkState::Active(ref network) = self.network_state {
            network.update(app, |network, _| {
                network.close_without_reconnection();
            });
        }
        self.model
            .lock()
            .set_shared_session_status(SharedSessionStatus::FinishedViewer);
        self.view
            .update(app, |view, ctx| view.on_session_share_ended(ctx));
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
