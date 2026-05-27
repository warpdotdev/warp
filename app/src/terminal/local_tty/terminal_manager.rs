use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{SendError, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::Context as _;
use async_broadcast::InactiveReceiver;
use parking_lot::{FairMutex, Mutex};
use pathfinder_geometry::vector::Vector2F;
use session_sharing_protocol::common::{
    ActivePrompt, AgentPromptFailureReason, CLIAgentSessionState, CommandExecutionFailureReason,
    ControlAction, ControlActionFailureReason, SelectedAgentModel,
    UniversalDeveloperInputContextUpdate, WriteToPtyFailureReason,
};
#[cfg(not(any(test, feature = "integration_tests")))]
use session_sharing_protocol::common::{
    LongRunningCommandAgentInteractionState, SelectedConversation, UniversalDeveloperInputContext,
};
use session_sharing_protocol::sharer::{
    AddGuestsResponse, FailedToInitializeSessionReason, Lifetime, LinkAccessLevelUpdateResponse,
    QuotaType, RemoveGuestResponse, SessionEndedReason, SessionSourceType,
    TeamAccessLevelUpdateResponse, UpdatePendingUserRoleResponse,
};
use settings::Setting as _;
use warp_core::execution_mode::AppExecutionMode;
use warp_core::send_telemetry_from_ctx;
use warpui::r#async::executor::Background;
use warpui::{AppContext, ModelContext, ModelHandle, SingletonEntity, ViewHandle, WindowId};
#[cfg(unix)]
use {
    super::terminal_attributes::TerminalAttributesPoller,
    crate::terminal::local_tty::terminal_attributes::Event as TerminalAttributesPollerEvent,
    crate::terminal::model::terminal_model::BlockIndex,
    crate::terminal::session_settings::NotificationsMode, nix::sys::termios::LocalFlags,
};

use super::event_loop::EventLoop;
use super::shell::{ShellStarter, ShellStarterSource};
use super::{mio_channel, recorder};
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::AIConversation;
use crate::ai::aws_credentials::AwsCredentialRefresher as _;
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::ai::blocklist::{
    BlocklistAIContextEvent, BlocklistAIContextModel, BlocklistAIControllerEvent,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, InputConfig, SerializedBlockListItem,
};
use crate::ai::llms::{LLMPreferences, LLMPreferencesEvent};
use crate::auth::auth_state::AuthState;
use crate::auth::AuthStateProvider;
use crate::banner::BannerState;
use crate::context_chips::current_prompt::CurrentPrompt;
use crate::context_chips::prompt_snapshot::PromptSnapshot;
use crate::context_chips::prompt_type::PromptType;
use crate::editor::CrdtOperation;
use crate::features::FeatureFlag;
use crate::network::{NetworkStatusEvent, NetworkStatusKind};
use crate::pane_group::TerminalViewResources;
use crate::persistence::ModelEvent;
use crate::server::telemetry::{TelemetryAgentViewEntryOrigin, TelemetryEvent};
use crate::settings::{DebugSettings, PrivacySettings, SshSettings};
use crate::terminal::available_shells::{AvailableShell, AvailableShells};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::{Pty, PtyOptions};
use crate::terminal::model::session::Sessions;
use crate::terminal::model::terminal_model::ExitReason;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::safe_mode_settings::get_secret_obfuscation_mode;
use crate::terminal::session_settings::{SessionSettings, SessionSettingsChangedEvent};
use crate::terminal::shared_session::manager::Manager;
use crate::terminal::shared_session::permissions_manager::SessionPermissionsManager;
use crate::terminal::shared_session::presence_manager::PresenceManager;
use crate::terminal::shared_session::replay_agent_conversations::reconstruct_response_events_from_conversations;
use crate::terminal::shared_session::settings::SharedSessionSettings;
use crate::terminal::shared_session::shared_handlers::{
    apply_auto_approve_agent_actions_update, apply_cli_agent_state_update, apply_input_mode_update,
    apply_selected_agent_model_update, apply_selected_conversation_update,
    build_selected_conversation_update, RemoteUpdateGuard,
};
use crate::terminal::shared_session::sharer::network::{
    failed_to_add_guests_user_error, failed_to_initialize_session_user_error,
    session_terminated_reason_string, Network, NetworkEvent,
};
use crate::terminal::shared_session::{
    IsSharedSessionCreator, SharedSessionActionSource, SharedSessionScrollbackType,
    SharedSessionSource, SharedSessionStatus,
};
use crate::terminal::shell::ShellName;
use crate::terminal::view::{ConversationRestorationInNewPaneType, Event as TerminalViewEvent};
use crate::terminal::warpify::settings::WarpifySettings;
use crate::terminal::writeable_pty::pty_controller::{EventLoopSendError, EventLoopSender};
use crate::terminal::writeable_pty::terminal_manager_util::{
    init_pty_controller_model, init_remote_server_controller, wire_up_pty_controller_with_view,
    wire_up_remote_server_controller_with_view,
};
use crate::terminal::writeable_pty::{self, Message};
use crate::terminal::{
    terminal_manager, ShellLaunchData, ShellLaunchState, TerminalManager as _, TerminalModel,
    TerminalView, PTY_READS_BROADCAST_CHANNEL_SIZE,
};
use crate::view_components::ToastFlavor;
use crate::{send_telemetry_on_executor, NetworkStatus};

type PtyController = writeable_pty::PtyController<mio_channel::Sender<Message>>;
type RemoteServerController =
    writeable_pty::remote_server_controller::RemoteServerController<mio_channel::Sender<Message>>;

const ACL_UPDATE_FAILURE_RESPONSE: &str = "Something went wrong. Please try again.";

/// Whether the given CRDT operation should be dropped when broadcasting
/// sharer input to viewers. In ambient agent sessions the sharer is a
/// headless worker — forwarding its selection ops would produce a phantom
/// cursor on the viewer side. Content ops (Edit / Undo) are kept so the
/// buffer stays in sync.
fn should_skip_sharer_op(is_ambient_session: bool, op: &CrdtOperation) -> bool {
    is_ambient_session && matches!(op, CrdtOperation::UpdateSelections(_))
}

/// The TerminalManager is responsible for
/// - creating the terminal model
/// - starting the local PTY
/// - creating the TerminalView
/// - wiring up the view with any dependencies necessary
///
/// It also holds onto any data that needs to live as long as the session does
/// (e.g. the event loop join handle).
pub struct TerminalManager {
    event_loop_tx: Arc<Mutex<mio_channel::Sender<Message>>>,
    /// This is an `Option` so that we can take ownership of the inner
    /// `JoinHandle` in `TerminalManager::drop`.
    event_loop_handle: Option<JoinHandle<()>>,
    model: Arc<FairMutex<TerminalModel>>,
    view: ViewHandle<TerminalView>,

    /// The manager is responsible for managing the lifetime
    /// of the terminal attributes poller. None if the event loop has not yet started.
    #[cfg(unix)]
    #[allow(dead_code)]
    terminal_attributes_poller: Option<ModelHandle<TerminalAttributesPoller>>,

    /// The manager is responsible for managing the lifetime
    /// of the PTY controller.
    #[allow(dead_code)]
    pty_controller: ModelHandle<PtyController>,

    /// The manager is responsible for managing the lifetime of the remote server controller.
    #[expect(dead_code)]
    remote_server_controller: ModelHandle<RemoteServerController>,

    /// The process ID of the PTY. Purely used for integration tests. None if the PTY has not yet
    /// been started.
    #[cfg(feature = "integration_tests")]
    pid: Option<u32>,

    /// An inactive receiver for PTY reads that we can upgrade to an active
    /// receiver as needed. We prefer to not create active receivers eagerly
    /// to avoid unnecessary allocations of data coming from the PTY (high throughput).
    /// Note that we need to hold onto the inactive receiver so that the channel isn't closed prematurely.
    #[allow(dead_code)]
    inactive_pty_reads_rx: InactiveReceiver<Arc<Vec<u8>>>,

    /// The model responsible for implementing the sharer's side of the
    /// session sharing protocol. Only [`Some`] when there is a shared session
    /// connection ongoing.
    #[allow(dead_code)]
    session_sharer: Rc<RefCell<Option<ModelHandle<Network>>>>,
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        self.shutdown_event_loop();
    }
}

impl TerminalManager {
    /// Sends a shutdown message to the PTY event loop and waits for it to
    /// process that event.
    fn shutdown_event_loop(&mut self) {
        let shutdown_res = self.event_loop_tx.lock().send(Message::Shutdown);
        // Happens normally if the event loop has already been terminated (so the channel is now gone).
        if let Err(e) = shutdown_res {
            log::info!("Failed to send Shutdown {e:?}");
        }

        if let Some(join_handle) = self.event_loop_handle.take() {
            if let Err(e) = join_handle.join() {
                log::error!("Failed to join event loop handle {e:?}");
            }
        } else {
            log::error!("No event loop handle to join when dropping terminal manager.")
        }

        self.inactive_pty_reads_rx.close();
    }

    /// Creates a terminal manager model. Note that the order of operations
    /// in this constructor are important! Specifically, we want to
    /// 1. Create the TerminalModel.
    /// 2. Set the pending local shell path on the model.
    /// 3. Start the PTY and its corresponding event loop.
    /// 4. Initialize the PtyController.
    /// 5. Create the TerminalView.
    /// 6. Wire up any dependencies between the view and any of the models.
    /// 7. Finally create the TerminalManager.
    #[allow(clippy::too_many_arguments)]
    pub fn create_model(
        startup_directory: Option<PathBuf>,
        env_vars: HashMap<OsString, OsString>,
        is_shared_session_creator: IsSharedSessionCreator,
        resources: TerminalViewResources,
        restored_blocks: Option<&Vec<SerializedBlockListItem>>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        initial_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        window_id: WindowId,
        chosen_shell: Option<AvailableShell>,
        initial_input_config: Option<InputConfig>,
        ctx: &mut AppContext,
    ) -> ModelHandle<Box<dyn crate::terminal::TerminalManager>> {
        // Create all the necessary channels we need for communication.
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, events_rx) = async_channel::unbounded();
        let (executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let (event_loop_tx, event_loop_rx) = mio_channel::channel();

        // Create the broadcast channel to receive data from the PTY, but deactivate it immediately.
        // We only want to create active receivers as necessary.
        let (pty_reads_tx, pty_reads_rx) =
            async_broadcast::broadcast(PTY_READS_BROADCAST_CHANNEL_SIZE);
        let inactive_pty_reads_rx = pty_reads_rx.deactivate();

        let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);

        // Initialize the sessions model.
        let sessions = ctx.add_model(|ctx| Sessions::new(executor_command_tx.clone(), ctx));

        let model_events =
            ctx.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));

        // Have ApiKeyManager subscribe to block completion events for AWS credential refresh
        ai::api_keys::ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.register_model_event_dispatcher(&model_events, ctx);
        });

        let preferred_shell = chosen_shell.unwrap_or_else(|| {
            AvailableShells::handle(ctx)
                .read(ctx, |shells, ctx| shells.get_user_preferred_shell(ctx))
        });

        let session_sharer: Rc<RefCell<Option<ModelHandle<Network>>>> = Rc::new(RefCell::new(None));
        let wsl_name_or_shell_starter = ShellStarter::init(preferred_shell.clone());

        // If we have explicit non-empty restored_blocks, prioritize those (these come from db on startup).
        // Otherwise if there's a conversation we're restoring, get blocks from those (including when
        // restored_blocks is missing or an empty vec).
        let all_restored_blocks = restored_blocks
            .filter(|blocks| !blocks.is_empty())
            .cloned()
            .or_else(|| match &conversation_restoration {
                Some(ConversationRestorationInNewPaneType::Historical { conversation, .. })
                | Some(ConversationRestorationInNewPaneType::Forked { conversation, .. }) => {
                    Some(conversation.to_serialized_blocklist_items())
                }
                Some(ConversationRestorationInNewPaneType::Startup { conversations, .. }) => {
                    let mut items: Vec<_> = conversations
                        .iter()
                        .flat_map(|c| c.to_serialized_blocklist_items())
                        .collect();
                    // Because there are multiple conversations that may have interleaved timestamps, we need to sort by start_ts
                    items.sort_by_key(|item| item.start_ts());
                    if items.is_empty() {
                        None
                    } else {
                        Some(items)
                    }
                }
                _ => None,
            });

        // Create the terminal model with all restored blocks
        log::info!(
            "Creating terminal model with {} restored blocks",
            all_restored_blocks
                .as_ref()
                .map(|blocks| blocks.len())
                .unwrap_or(0)
        );
        let model = terminal_manager::create_terminal_model(
            startup_directory.clone(),
            all_restored_blocks.as_ref(),
            initial_size,
            channel_event_proxy.clone(),
            ShellLaunchState::DeterminingShell {
                available_shell: Some(preferred_shell),
                display_name: wsl_name_or_shell_starter
                    .as_ref()
                    .map(|wsl_name_or_shell_starter| wsl_name_or_shell_starter.name())
                    .unwrap_or(ShellName::LessDescriptive("Shell".to_owned())),
            },
            ctx,
        );
        let colors = model.colors();

        let model = Arc::new(FairMutex::new(model));

        // This is purely for measuring throughput on WarpDev.
        if FeatureFlag::RecordPtyThroughput.is_enabled() {
            Self::record_pty_throughput(inactive_pty_reads_rx.clone(), model.clone(), ctx);
        }

        // If this session should be a shared-session creator, configure its initial
        // shared-session state before we construct the view, so that bootstrap
        // events can observe the correct pending status and source type.
        match is_shared_session_creator {
            IsSharedSessionCreator::Yes { source }
                if FeatureFlag::CreatingSharedSessions.is_enabled() =>
            {
                model.lock().set_shared_session_status(
                    SharedSessionStatus::SharePendingPreBootstrap { source },
                );
                log::info!("Configured terminal to start sharing after bootstrap");
            }
            IsSharedSessionCreator::Yes { .. } => {
                log::warn!(
                    "Session sharing was requested, but CreatingSharedSessions is disabled; \
                     skipping shared-session startup"
                );
            }
            IsSharedSessionCreator::No => {}
        }

        // Initialize the PtyController.
        let pty_controller = init_pty_controller_model(
            event_loop_tx.clone(),
            executor_command_rx,
            model_events.clone(),
            sessions.clone(),
            model.clone(),
            ctx,
        );

        // Initialize the RemoteServerController.
        let remote_server_controller =
            init_remote_server_controller(&pty_controller, &model_events, ctx);

        let current_prompt = ctx.add_model(|ctx| {
            CurrentPrompt::new_with_model_events(sessions.clone(), Some(&model_events), ctx)
        });
        let prompt_type = ctx.add_model(|ctx| PromptType::new_dynamic(current_prompt.clone(), ctx));
        let session_sharer_clone = session_sharer.clone();

        // Send warp prompt updates.
        ctx.observe_model(&current_prompt, move |current_prompt, ctx| {
            // If for some reason ctx.notify() was called on the warp prompt but we're using ps1, do nothing.
            if *SessionSettings::as_ref(ctx).honor_ps1 {
                return
            }
            let prompt_snapshot = current_prompt.read(ctx, |current_prompt, ctx| {
                PromptSnapshot::from_current_prompt(current_prompt, ctx)
            });
            if let Some(network) = session_sharer_clone.borrow().as_ref() {
                let Ok(serialized_prompt) = serde_json::to_string(&prompt_snapshot) else {
                    log::error!("Failed to serialize prompt snapshot to send active prompt update to shared session server");
                    return
                };
                network.update(ctx, |network, _| {
                    network.send_active_prompt_update_if_changed(session_sharing_protocol::common::ActivePrompt::WarpPrompt(serialized_prompt))
                });
            }
        });

        let has_restored_command_blocks = all_restored_blocks
            .as_ref()
            .is_some_and(|blocks| !blocks.is_empty());
        let has_conversation_restoration = matches!(
            &conversation_restoration,
            Some(
                ConversationRestorationInNewPaneType::Startup { .. }
                    | ConversationRestorationInNewPaneType::Historical { .. }
            )
        );
        let is_historical = matches!(
            &conversation_restoration,
            Some(ConversationRestorationInNewPaneType::Historical { .. })
        );
        // Create the view.
        let cloned_model = model.clone();
        let should_use_live_appearance = conversation_restoration
            .as_ref()
            .map(|restoration| restoration.should_use_live_appearance())
            .unwrap_or(false);
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
                model_event_sender.clone(),
                prompt_type.clone(),
                initial_input_config,
                conversation_restoration,
                Some(inactive_pty_reads_rx.clone()),
                false,
                ctx,
            )
        });

        // We need to append the session restoration separator to the block list if there are any
        // restored blocks (command blocks or AI conversations) to show.
        // Add separator if we have restored command blocks or we're restoring from historical or startup.
        let should_show_restoration_separator = (has_conversation_restoration
            || has_restored_command_blocks)
            && !should_use_live_appearance;

        if should_show_restoration_separator {
            model
                .lock()
                .block_list_mut()
                .append_session_restoration_separator_to_block_list(is_historical);
        }

        // In unit tests, we know we aren't going to bootstrap a shell
        // so if we're waiting on starting a shared session until bootstrapped,
        // just attempt to start it now.
        #[cfg(test)]
        if matches!(
            model.lock().shared_session_status(),
            SharedSessionStatus::SharePendingPreBootstrap { .. }
        ) {
            view.update(ctx, |view, ctx| {
                view.attempt_to_share_session(
                    SharedSessionScrollbackType::All,
                    None,
                    SharedSessionSource::user(None),
                    false,
                    ctx,
                )
            });
        }

        wire_up_pty_controller_with_view(
            &pty_controller,
            &view,
            model.clone(),
            sessions,
            model_event_sender,
            ctx,
        );

        wire_up_remote_server_controller_with_view(&remote_server_controller, &view, ctx);

        let session_sharer_clone = session_sharer.clone();
        ctx.subscribe_to_model(&SessionSettings::handle(ctx), move |_, event, ctx| {
            if let SessionSettingsChangedEvent::HonorPS1 { .. } = event {
                if !*SessionSettings::as_ref(ctx).honor_ps1 {
                    // We don't need to send a WarpPrompt message here when turning off PS1 because this will be sent
                    // as part of observing the warp prompt and sending messages on updates.
                    return;
                }
                if let Some(network) = session_sharer_clone.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_active_prompt_update_if_changed(
                            session_sharing_protocol::common::ActivePrompt::PS1,
                        )
                    });
                }
            }
        });

        let sharer_remote_update_guard = RemoteUpdateGuard::new();

        // Send model selection updates during session sharing
        let session_sharer_for_models = session_sharer.clone();
        let terminal_view_id = view.id();
        let model_remote_update_guard = sharer_remote_update_guard.clone();
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), move |_prefs, event, ctx| {
            // Only react to agent mode LLM changes
            if !matches!(event, LLMPreferencesEvent::UpdatedActiveAgentModeLLM) {
                return;
            }

            if !model_remote_update_guard.should_broadcast() {
                return;
            }

            if let Some(network) = session_sharer_for_models.borrow().as_ref() {
                let llm_prefs = LLMPreferences::as_ref(ctx);
                let selected_model_id: String = llm_prefs
                    .get_active_base_model(ctx, Some(terminal_view_id))
                    .id
                    .clone()
                    .into();

                // The send method will check if it actually changed and skip if not
                network.update(ctx, |network, _| {
                    network.send_universal_developer_input_context_update(
                        UniversalDeveloperInputContextUpdate {
                            selected_model: Some(SelectedAgentModel::new(selected_model_id)),
                            ..Default::default()
                        },
                    )
                });
            }
        });

        // Send input mode updates during session sharing.
        // When AgentView is enabled, we only send updates when in an active agent view.
        // For ambient agent sessions, input mode is controlled locally, so we skip sending updates.
        let session_sharer_for_input_mode = session_sharer.clone();
        let ai_input_model = view.as_ref(ctx).ai_input_model().clone();
        let agent_view_controller_for_input_mode = view.as_ref(ctx).agent_view_controller().clone();
        let model_for_input_mode = model.clone();
        let input_mode_remote_update_guard = sharer_remote_update_guard.clone();
        ctx.subscribe_to_model(&ai_input_model, move |_, event, ctx| {
            if !input_mode_remote_update_guard.should_broadcast() {
                return;
            }

            // In ambient agent sessions, input mode is controlled locally.
            if model_for_input_mode
                .lock()
                .is_shared_ambient_agent_session()
            {
                return;
            }

            // When AgentView is enabled, only send input mode updates when in an active agent view.
            if FeatureFlag::AgentView.is_enabled()
                && !agent_view_controller_for_input_mode.as_ref(ctx).is_active()
            {
                return;
            }

            let config = event.updated_config();
            if let Some(network) = session_sharer_for_input_mode.borrow().as_ref() {
                // The send method will check if it actually changed and skip if not
                network.update(ctx, |network, _| {
                    network.send_universal_developer_input_context_update(
                        UniversalDeveloperInputContextUpdate {
                            input_mode: Some((*config).into()),
                            ..Default::default()
                        },
                    )
                });
            }
        });

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

        let ai_context_model = view.as_ref(ctx).ai_context_model().clone();

        // Send selected conversation updates during session sharing.
        if FeatureFlag::AgentView.is_enabled() {
            // When agent view is enabled, we listen to the agent view controller
            // as the authoritative source for which conversation is selected.
            let session_sharer_for_conversation = session_sharer.clone();
            let ai_context_model_for_conversation = ai_context_model.clone();
            let conversation_remote_update_guard = sharer_remote_update_guard.clone();
            ctx.subscribe_to_model(
                &agent_view_controller,
                move |agent_view_controller, event, ctx| match event {
                    AgentViewControllerEvent::EnteredAgentView { .. } => {
                        if conversation_remote_update_guard.should_broadcast() {
                            Self::send_selected_conversation_update_for_sharer(
                                &session_sharer_for_conversation,
                                &agent_view_controller,
                                &ai_context_model_for_conversation,
                                ctx,
                            );
                        }
                    }
                    AgentViewControllerEvent::ExitedAgentView {
                        origin,
                        final_exchange_count,
                        ..
                    } => {
                        if conversation_remote_update_guard.should_broadcast() {
                            Self::send_selected_conversation_update_for_sharer(
                                &session_sharer_for_conversation,
                                &agent_view_controller,
                                &ai_context_model_for_conversation,
                                ctx,
                            );
                        }
                        send_telemetry_from_ctx!(
                            TelemetryEvent::AgentViewExited {
                                origin: TelemetryAgentViewEntryOrigin::from(*origin),
                                was_empty: *final_exchange_count == 0,
                            },
                            ctx
                        );
                    }
                    AgentViewControllerEvent::ExitConfirmed { .. } => {}
                },
            );
        } else {
            // When agent view is disabled, we fallback to the legacy behavior
            // of listening for pending query state changes to know which conversation is selected.
            let session_sharer_for_conversation = session_sharer.clone();
            let agent_view_controller_for_conversation = agent_view_controller.clone();
            let conversation_remote_update_guard = sharer_remote_update_guard.clone();
            ctx.subscribe_to_model(&ai_context_model, move |ai_context_model, event, ctx| {
                if !matches!(event, BlocklistAIContextEvent::PendingQueryStateUpdated) {
                    return;
                }

                if !conversation_remote_update_guard.should_broadcast() {
                    return;
                }

                Self::send_selected_conversation_update_for_sharer(
                    &session_sharer_for_conversation,
                    &agent_view_controller_for_conversation,
                    &ai_context_model,
                    ctx,
                );
            });
        }
        // Also send after a request is submitted so viewers stay pinned to the intended conversation
        let session_sharer_for_sent_request = session_sharer.clone();
        let agent_view_controller_for_sent_request = agent_view_controller.clone();
        let ai_context_model_for_sent_request = ai_context_model.clone();
        let ai_controller_for_sent_request = view.as_ref(ctx).ai_controller().clone();
        ctx.subscribe_to_model(&ai_controller_for_sent_request, move |_, event, ctx| {
            if let BlocklistAIControllerEvent::SentRequest { .. } = event {
                Self::send_selected_conversation_update_for_sharer(
                    &session_sharer_for_sent_request,
                    &agent_view_controller_for_sent_request,
                    &ai_context_model_for_sent_request,
                    ctx,
                );
            }
        });
        // Finally, when the server assigns a token, resend with the concrete token,
        // & when the user toggles auto-approve, fan out an update.
        let session_sharer_for_stream_init = session_sharer.clone();
        let view_id_for_stream_init = view.id();
        let weak_view_for_stream_init = view.downgrade();
        let auto_approve_remote_update_guard = sharer_remote_update_guard.clone();
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |_, event, ctx| {
                match event {
                    BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                        terminal_view_id,
                        conversation_id,
                        ..
                    } => {
                        if *terminal_view_id != view_id_for_stream_init {
                            return;
                        }

                        let Some(view) = weak_view_for_stream_init.upgrade(ctx) else {
                            return;
                        };
                        let ai_context_model = view.as_ref(ctx).ai_context_model().clone();
                        let agent_view_controller =
                            view.as_ref(ctx).agent_view_controller().clone();

                        let history_model = BlocklistAIHistoryModel::handle(ctx);

                        // if the conversation is not selected or does not have a token,
                        // don't emit an update.
                        if !ai_context_model
                            .as_ref(ctx)
                            .selected_conversation_id(ctx)
                            .is_some_and(|sel| sel == *conversation_id)
                        {
                            return;
                        }
                        if history_model
                            .as_ref(ctx)
                            .conversation(conversation_id)
                            .and_then(|c| c.server_conversation_token())
                            .is_none()
                        {
                            return;
                        }

                        Self::send_selected_conversation_update_for_sharer(
                            &session_sharer_for_stream_init,
                            &agent_view_controller,
                            &ai_context_model,
                            ctx,
                        );
                    }
                    BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { terminal_view_id } => {
                        if *terminal_view_id != view_id_for_stream_init {
                            return;
                        }

                        if !auto_approve_remote_update_guard.should_broadcast() {
                            return;
                        }

                        let Some(view) = weak_view_for_stream_init.upgrade(ctx) else {
                            return;
                        };
                        let ai_context_model = view.as_ref(ctx).ai_context_model().clone();

                        if let Some(network) = session_sharer_for_stream_init.borrow().as_ref() {
                            let auto_approve = ai_context_model
                                .as_ref(ctx)
                                .pending_query_autoexecute_override(ctx)
                                .is_autoexecute_any_action();

                            network.update(ctx, |network, _| {
                                network.send_universal_developer_input_context_update(
                                    UniversalDeveloperInputContextUpdate {
                                        auto_approve_agent_actions: Some(auto_approve),
                                        ..Default::default()
                                    },
                                );
                            });
                        }
                    }
                    // Upgrade a manual `User` share's sidecar `source_task_id`
                    // from `None` to `Some(_)` once the active conversation
                    // gets its `task_id`, so inherited child shares can
                    // discover the orchestrator task. Existing viewers stay
                    // on the old value (the protocol has no
                    // `UpdateSourceType` upstream message) until they
                    // reconnect.
                    BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                        terminal_view_id,
                        conversation_id,
                    } => {
                        if *terminal_view_id != view_id_for_stream_init {
                            return;
                        }

                        let Some(view) = weak_view_for_stream_init.upgrade(ctx) else {
                            return;
                        };

                        let model = view.as_ref(ctx).model.clone();
                        let needs_upgrade = {
                            let model_lock = model.lock();
                            model_lock.shared_session_source().is_some_and(|s| {
                                matches!(s.source_type, SessionSourceType::User)
                                    && s.source_task_id.is_none()
                            })
                        };
                        if !needs_upgrade {
                            return;
                        }

                        let task_id = BlocklistAIHistoryModel::as_ref(ctx)
                            .conversation(conversation_id)
                            .and_then(|c| c.task_id());
                        let Some(task_id) = task_id else {
                            return;
                        };

                        model
                            .lock()
                            .set_shared_session_source_task_id(Some(task_id.to_string()));
                    }
                    _ => {}
                }
            },
        );

        // Always wire up the model but check the flag when a share is attempted.
        Self::wire_up_session_sharer_with_view(
            &view,
            prompt_type,
            session_sharer.clone(),
            model.clone(),
            window_id,
            sharer_remote_update_guard,
            ctx,
        );

        Self::handle_network_status_events(&view, session_sharer.clone(), ctx);

        #[cfg(windows)]
        let event_loop_tx_clone = event_loop_tx.clone();

        // Create the terminal manager itself.
        let terminal_manager = Self {
            event_loop_tx: Arc::new(Mutex::new(event_loop_tx)),
            model,
            event_loop_handle: None,
            view,
            #[cfg(unix)]
            terminal_attributes_poller: None,
            pty_controller,
            remote_server_controller,

            #[cfg(feature = "integration_tests")]
            pid: None,

            inactive_pty_reads_rx,
            session_sharer,
        };

        let terminal_manager_model = ctx.add_model(|ctx| {
            let terminal_manager: Box<dyn crate::terminal::TerminalManager> =
                Box::new(terminal_manager);

            ctx.spawn(
                async move {
                    match wsl_name_or_shell_starter {
                        Some(starter_source) => starter_source.to_shell_starter_source().await,
                        None => None,
                    }
                },
                move |terminal_manager: &mut Box<dyn crate::terminal::TerminalManager>,
                      shell_starter_source,
                      ctx| {
                    let Some(terminal_manager) =
                        crate::terminal::TerminalManager::as_any_mut(terminal_manager.as_mut())
                            .downcast_mut::<TerminalManager>()
                    else {
                        return;
                    };

                    terminal_manager.on_shell_determined(
                        startup_directory,
                        env_vars,
                        user_default_shell_unsupported_banner_model_handle,
                        #[cfg(windows)]
                        event_loop_tx_clone,
                        event_loop_rx,
                        channel_event_proxy,
                        shell_starter_source,
                        ctx,
                    )
                },
            );

            terminal_manager
        });

        terminal_manager_model
    }

    /// Callback invoked upon determining the shell to be spawned when starting the event loop.
    #[allow(clippy::too_many_arguments)]
    fn on_shell_determined(
        &mut self,
        startup_directory: Option<PathBuf>,
        env_vars: HashMap<OsString, OsString>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        #[cfg(windows)] event_loop_tx: mio_channel::Sender<Message>,
        event_loop_rx: mio_channel::Receiver<Message>,
        channel_event_proxy: ChannelEventListener,
        shell_starter_source: Option<ShellStarterSource>,
        ctx: &mut ModelContext<Box<dyn crate::terminal::TerminalManager>>,
    ) {
        // This is executed as a callback and the window could be closed in the interim.
        if !ctx.is_window_open(self.view.window_id(ctx)) {
            log::warn!("Window was closed before shell was determined, aborting shell startup.");
            return;
        }

        log::debug!("Using shell starter source {shell_starter_source:?}");
        let bg_executor = ctx.background_executor();
        let auth_state = AuthStateProvider::as_ref(ctx).get();

        let is_fallback_shell = matches!(
            shell_starter_source,
            Some(ShellStarterSource::Fallback { .. })
        );
        let shell_starter = shell_starter_source
            .map(|source| get_shell_starter_internal(source, bg_executor, auth_state));
        let shell_starter = match shell_starter {
            Some(shell_starter) => shell_starter,
            None => {
                log::error!("Could not compute fallback shell");
                self.view.update(ctx, |terminal_view, ctx| {
                    terminal_view.on_pty_spawn_failed(
                        anyhow::Error::msg("Could not find a fallback shell. If you have PowerShell or WSL installed, please file an issue."),
                        ctx,
                    );
                });
                self.model().lock().exit(ExitReason::ShellNotFound);
                return;
            }
        };

        // In WSL, default to the WSL home directory, not the native Windows home directory.
        let startup_directory = if let (ShellStarter::Wsl(wsl_shell_starter), None) =
            (&shell_starter, &startup_directory)
        {
            wsl_shell_starter.home_directory()
        } else {
            startup_directory
        };

        // Show a "shell unsupported" banner, if applicable.
        if is_fallback_shell
            && user_default_shell_unsupported_banner_model_handle.as_ref(ctx)
                == &BannerState::NotDismissed
        {
            user_default_shell_unsupported_banner_model_handle.update(ctx, |model, ctx| {
                *model = BannerState::Open;
                ctx.notify();
            })
        }

        self.model()
            .lock()
            .set_login_shell_spawned(shell_starter.shell_type());

        let shell_launch_data = match &shell_starter {
            ShellStarter::Direct(shell_starter) => ShellLaunchData::Executable {
                executable_path: shell_starter.logical_shell_path().to_owned(),
                shell_type: shell_starter.shell_type(),
            },
            ShellStarter::DockerSandbox(docker_starter) => ShellLaunchData::Executable {
                executable_path: docker_starter.logical_shell_path().to_owned(),
                shell_type: docker_starter.shell_type(),
            },
            ShellStarter::Wsl(shell_starter) => ShellLaunchData::WSL {
                distro: shell_starter.distribution().to_owned(),
            },
            ShellStarter::MSYS2(shell_starter) => ShellLaunchData::MSYS2 {
                executable_path: shell_starter.logical_shell_path().to_owned(),
                shell_type: shell_starter.shell_type(),
            },
        };

        // This needs to be done before bootstrapping starts (i.e. before spawning the event loop below).
        self.model()
            .lock()
            .set_pending_shell_launch_data(shell_launch_data.clone());

        // Enqueue the init shell script (for shells that need it), then create
        // the PTY and start its corresponding event loop.
        let model = self.model();
        let pty = match self
            .enqueue_init_script(&shell_starter)
            .context("Failed to write shell init script to the pty")
            .and_then(|_| {
                Self::create_pty(
                    startup_directory,
                    shell_starter,
                    env_vars,
                    model.clone(),
                    #[cfg(windows)]
                    event_loop_tx,
                    ctx,
                )
            }) {
            Ok(pty) => pty,
            Err(err) => {
                log::error!("Failed to spawn pty: {err:#}");
                self.view.update(ctx, |terminal_view, ctx| {
                    terminal_view.on_pty_spawn_failed(err, ctx);
                });
                self.model().lock().exit(ExitReason::PtySpawnFailed);
                return;
            }
        };

        #[cfg(feature = "integration_tests")]
        let pid = pty.get_pid();
        #[cfg(unix)]
        let fd = pty.get_fd();

        // Create the channel above and pass the receving side to the event loop.
        let event_loop_handle = Self::start_pty_event_loop(
            pty,
            event_loop_rx,
            model.clone(),
            channel_event_proxy.clone(),
        );

        self.event_loop_handle = Some(event_loop_handle);
        #[cfg(feature = "integration_tests")]
        {
            self.pid = Some(pid);
        }

        self.view.update(ctx, |terminal_view, ctx| {
            terminal_view.on_shell_determined(ctx);
            terminal_view.on_active_shell_launch_data_updated(Some(shell_launch_data), ctx);
        });

        // Initialize the terminal attributes poller.
        // TODO(CORE-2297): Implement TerminalPoller on Windows.
        #[cfg(unix)]
        {
            let terminal_attributes_poller = ctx.add_model(|_| TerminalAttributesPoller::new(fd));
            TerminalManager::wire_up_terminal_attribute_poller_with_view(
                &terminal_attributes_poller,
                &self.view,
                model.clone(),
                ctx,
            );

            self.terminal_attributes_poller = Some(terminal_attributes_poller);
        }
    }

    /// Sends bindkey to notify shell process to switch to PS1 logic for prompt
    /// with the combined prompt/command grid (we restore the saved PS1 value).
    pub fn send_switch_to_ps1_bindkey(&self, app_ctx: &mut AppContext) {
        self.pty_controller.update(app_ctx, |pty_controller, ctx| {
            pty_controller.send_switch_to_ps1_bindkey(ctx);
        });
    }

    /// Sends bindkey to notify shell process to switch to Warp prompt logic for prompt
    /// with the combined prompt/command grid (we unset the PS1, but save the value for potential
    /// future restoration).
    pub fn send_switch_to_warp_prompt_bindkey(&self, app_ctx: &mut AppContext) {
        self.pty_controller.update(app_ctx, |pty_controller, ctx| {
            pty_controller.send_switch_to_warp_prompt_bindkey(ctx);
        });
    }

    /// Records the PTY throughput by emitting a metric whenever the throughput
    /// is non-zero over some time interval.
    fn record_pty_throughput(
        pty_reads_rx: InactiveReceiver<Arc<Vec<u8>>>,
        model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut AppContext,
    ) {
        if FeatureFlag::RecordPtyThroughput.is_enabled() {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            recorder::record_pty_throughput(
                pty_reads_rx.activate(),
                model,
                auth_state.clone(),
                ctx.background_executor().to_owned(),
            );
        }
    }

    fn enqueue_init_script(&self, shell_starter: &ShellStarter) -> Result<(), SendError<Message>> {
        let shell_type = shell_starter.shell_type();
        if shell_type == crate::terminal::shell::ShellType::Zsh
            // For more on why this is necessary on Git Bash, see https://linear.app/warpdotdev/issue/CORE-3202.
            || shell_starter.is_msys2()
        {
            let init_shell_script =
                crate::terminal::bootstrap::init_shell_script_for_shell(shell_type, &crate::ASSETS);
            let tx = self.event_loop_tx.lock();
            tx.send(Message::Input(init_shell_script.into_bytes().into()))?;
            tx.send(Message::Input(shell_type.execute_command_bytes().into()))
        } else {
            Ok(())
        }
    }

    fn create_pty(
        startup_directory: Option<PathBuf>,
        shell_starter: ShellStarter,
        env_vars: HashMap<OsString, OsString>,
        model: Arc<FairMutex<TerminalModel>>,
        #[cfg(windows)] event_loop_tx: mio_channel::Sender<Message>,
        ctx: &mut AppContext,
    ) -> anyhow::Result<Pty> {
        let is_shell_debug_mode_enabled = *DebugSettings::as_ref(ctx)
            .is_shell_debug_mode_enabled
            .value();
        let is_honor_ps1_enabled = *SessionSettings::as_ref(ctx).honor_ps1;
        let is_crash_reporting_enabled = PrivacySettings::as_ref(ctx).is_crash_reporting_enabled;

        // The TMUX SSH wrapper supercedes the original ControlMaster wrapper.
        let enable_ssh_wrapper = if FeatureFlag::SSHTmuxWrapper.is_enabled() {
            *WarpifySettings::as_ref(ctx)
                .enable_ssh_warpification
                .value()
                && !*WarpifySettings::as_ref(ctx).use_ssh_tmux_wrapper.value()
        } else {
            *SshSettings::as_ref(ctx).enable_legacy_ssh_wrapper.value()
        };

        let size: crate::terminal::SizeInfo = model.lock().block_list().size().to_owned();
        let options = PtyOptions {
            size,
            window_id: None,
            shell_starter,
            start_dir: startup_directory,
            env_vars,
            enable_ssh_wrapper,
            shell_debug_mode: is_shell_debug_mode_enabled,
            honor_ps1: is_honor_ps1_enabled,
            close_fds: true,
        };

        Pty::new(
            options,
            is_crash_reporting_enabled,
            #[cfg(windows)]
            event_loop_tx,
            ctx,
        )
    }

    /// Start's the PTY event loop, returning a sender for the event loop and the event loop's join handle.
    fn start_pty_event_loop(
        pty: Pty,
        rx: mio_channel::Receiver<Message>,
        model: Arc<FairMutex<TerminalModel>>,
        channel_event_proxy: ChannelEventListener,
    ) -> JoinHandle<()> {
        // Create the event loop and get a handle to the injector.
        let event_loop = EventLoop::new(model, channel_event_proxy, pty, rx);

        // Spawn the event loop on a separate thread to interact with the PTY and write the data back
        // to the terminal.
        event_loop.spawn()
    }

    /// Configures bi-directional communication between the terminal attributes poller
    /// and the terminal view.
    ///
    /// NOTE: we cannot simply use the strong references (the handle arguments to this wire_up fn)
    /// in the subscription callbacks because that will create a reference cycle. Instead,
    /// we should use weak handles and upgrade them lazily.
    ///
    /// TODO: while there is a lot of notification-heavy logic below, this will eventually
    /// be in a dedicated terminal::NotificationSender. For now though, this logic cannot
    /// live in TerminalView (because termios is a *nix thing).
    #[cfg(unix)]
    fn wire_up_terminal_attribute_poller_with_view(
        terminal_attributes_poller: &ModelHandle<TerminalAttributesPoller>,
        terminal_view: &ViewHandle<TerminalView>,
        model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ModelContext<Box<dyn crate::terminal::TerminalManager>>,
    ) {
        let poller_weak_handle = terminal_attributes_poller.downgrade();
        let view_weak_handle = terminal_view.downgrade();
        let view_weak_handle_2 = view_weak_handle.clone();

        // Used to track the index of the started block across the view <-> poller interactions.
        let block_index: Rc<RefCell<Option<BlockIndex>>> = Rc::new(RefCell::new(None));
        let block_index_clone = block_index.clone();

        // Whenever we get a BlockStarted, we want to start the terminal attribute poller.
        // Whenever the block is completed, we can stop the terminal attribute poller.
        ctx.subscribe_to_view(terminal_view, move |_view, event, ctx| {
            let Some(poller) = poller_weak_handle.upgrade(ctx) else {
                return;
            };

            match event {
                TerminalViewEvent::BlockStarted {
                    is_for_in_band_command,
                } if !is_for_in_band_command => {
                    *block_index.borrow_mut() =
                        Some(model.lock().block_list().active_block_index());

                    let password_notification_setting_on = show_password_notifications(ctx);
                    let pane_handling_ssh_upload =
                        view_weak_handle_2.upgrade(ctx).is_some_and(|view| {
                            view.update(ctx, |terminal_view, _ctx| terminal_view.is_ssh_uploader())
                        });
                    let should_poll_for_password_prompt = password_notification_setting_on
                        || (pane_handling_ssh_upload && FeatureFlag::SshDragAndDrop.is_enabled());

                    if should_poll_for_password_prompt {
                        poller.update(ctx, |model, ctx| {
                            model.start_polling(ctx);
                        });
                    }
                }
                TerminalViewEvent::BlockCompleted { block, .. } => {
                    // If the poller was turned on, that means that the next BlockCompleted
                    // event would have been for the same block.
                    poller.update(ctx, |model, _ctx| {
                        model.stop_polling();
                    });

                    if let Some(view) = view_weak_handle_2.upgrade(ctx) {
                        view.update(ctx, |terminal_view, ctx| {
                            if terminal_view.is_ssh_uploader() {
                                let exit_code = block.exit_code;
                                terminal_view.propagate_upload_finished_event(exit_code, ctx);
                            }
                        })
                    }
                }
                _ => {}
            }
        });

        // Whenever the terminal attribute poller yields a termios struct, we might
        // want to trigger a password notification depending on the termios attributes.
        // TODO: eventually, this logic will be in a terminal::NotificationSender where
        // it makes far more sense to be reading the termios struct. For now though,
        // this logic can't live in TerminalView (because termios is a *nix thing).
        ctx.subscribe_to_model(
            terminal_attributes_poller,
            move |terminal_manager, event, ctx| {
                let Some(view) = view_weak_handle.upgrade(ctx) else {
                    return;
                };

                let TerminalAttributesPollerEvent::TermiosQueryFinished { termios } = event;

                // A PTY likely has a password prompt if it is not echoing characters back (ECHO disabled)
                // AND is in canonical mode (ICANON enabled).
                //
                // We need to check ICANON because apps like neovim disable both ECHO and ICANON
                // when entering raw mode (for character-by-character input handling), which would
                // otherwise cause false positive password notifications. A real password prompt
                // typically keeps ICANON enabled since password entry is still line-based.
                let might_be_password_prompt = !termios.local_flags.contains(LocalFlags::ECHO)
                    && termios.local_flags.contains(LocalFlags::ICANON);

                if might_be_password_prompt {
                    if FeatureFlag::SshDragAndDrop.is_enabled() {
                        view.update(ctx, |view, ctx| {
                            view.propagate_password_request(ctx);
                        });
                    }

                    // Only send the notification if the user is navigated away from the window
                    // when the password prompt appears. If the password prompt appears and they
                    // are not navigated away, don't poll again since we would then send a notification
                    // for something the user already knows.
                    let is_navigated_away_from_window =
                        ctx.windows().active_window() != Some(view.window_id(ctx));
                    let password_notification_setting_on = show_password_notifications(ctx);
                    if is_navigated_away_from_window && password_notification_setting_on {
                        if let Some(block_index) = block_index_clone.borrow_mut().take() {
                            view.update(ctx, |view, ctx| {
                                view.maybe_send_password_notification(block_index, ctx);
                            });
                        }
                    }

                    // TODO: this stops the notification stream for a single command
                    // after one password notification. We should track the output progress
                    // instead so that we can send multiple notifications for a single command.
                    let Some(terminal_manager) =
                        crate::terminal::TerminalManager::as_any_mut(terminal_manager.as_mut())
                            .downcast_mut::<TerminalManager>()
                    else {
                        return;
                    };

                    if let Some(poller) = &mut terminal_manager.terminal_attributes_poller {
                        poller.update(ctx, |poller, _ctx| {
                            poller.stop_polling();
                        });
                    }
                }
            },
        );
    }

    /// Streams all historical agent conversations from this terminal to viewers.
    /// This is called when starting a shared  session mid-conversation so that viewers
    /// can see all conversation history and properly continue conversations.
    fn stream_historical_agent_conversations(
        terminal_view: &ViewHandle<TerminalView>,
        model: &Arc<FairMutex<TerminalModel>>,
        ctx: &mut AppContext,
    ) {
        // Get all conversations for this terminal view
        // Any conversation could be continued during session sharing
        let conversations: Vec<AIConversation> = BlocklistAIHistoryModel::as_ref(ctx)
            .all_live_conversations_for_terminal_view(terminal_view.id())
            .filter(|conv| conv.exchange_count() > 0)
            .cloned()
            .collect();

        if conversations.is_empty() {
            return;
        }

        // Get the sharer's participant id to use for historical conversations
        let sharer_id = terminal_view
            .as_ref(ctx)
            .shared_session_presence_manager()
            .map(|manager| manager.as_ref(ctx).sharer_id());

        model
            .lock()
            .send_agent_conversation_replay_started_for_shared_session();

        // Reconstruct and send all conversations' messages as ResponseEvent objects
        // Exchanges are sorted chronologically to handle interleaved conversations
        // Historical events use the original conversation token, so no need to pass forked_from.
        let events = reconstruct_response_events_from_conversations(&conversations);
        for event in events {
            model
                .lock()
                .send_agent_response_for_shared_session(&event, sharer_id.clone(), None);
        }
        model
            .lock()
            .send_agent_conversation_replay_ended_for_shared_session();
    }

    /// Send selected_conversation update to viewers based on current selection.
    fn send_selected_conversation_update_for_sharer(
        session_sharer: &Rc<RefCell<Option<ModelHandle<Network>>>>,
        agent_view_controller: &ModelHandle<AgentViewController>,
        ai_context_model: &ModelHandle<BlocklistAIContextModel>,
        ctx: &mut AppContext,
    ) {
        if let Some(network) = session_sharer.borrow().as_ref() {
            if let Some(update) =
                build_selected_conversation_update(agent_view_controller, ai_context_model, ctx)
            {
                network.update(ctx, |network, _| {
                    network.send_universal_developer_input_context_update(update)
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_sharing_session(
        terminal_view: ViewHandle<TerminalView>,
        prompt_type: ModelHandle<PromptType>,
        shared_session_model: Rc<RefCell<Option<ModelHandle<Network>>>>,
        scrollback_type: SharedSessionScrollbackType,
        lifetime: Lifetime,
        source: SharedSessionSource,
        model: Arc<FairMutex<TerminalModel>>,
        window_id: WindowId,
        sharer_remote_update_guard: RemoteUpdateGuard,
        ctx: &mut AppContext,
    ) {
        let mut session_sharer = shared_session_model.borrow_mut();

        // If it's already being shared, then this should no-op.
        // In practice, this event shouldn't even be emitted if that's the case.
        if session_sharer.is_some() {
            log::warn!("Tried to share a session that's already being shared.");
            return;
        }
        log::info!("Starting shared session");

        // Record the source on the model so we can distinguish ambient agent
        // sessions from user-initiated shared sessions in the UI logic, and so
        // the orchestrator task id is discoverable regardless of which variant
        // the share is.
        model.lock().set_shared_session_source(source.clone());
        Self::log_shared_session_lifecycle(
            &terminal_view,
            &model,
            "start_requested",
            "trigger=terminal_view_start_sharing",
            ctx,
        );
        if matches!(source.source_type, SessionSourceType::AmbientAgent { .. }) {
            let terminal_view_id = terminal_view.id();
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _ctx| {
                history.mark_terminal_view_as_ambient_agent_session_view(terminal_view_id);
            });
        }

        // Snapshot the conversation the user has selected at click time so the
        // share is linked to that run, even if selection drifts before the
        // server confirms session creation.
        let selected_conversation_id = terminal_view
            .as_ref(ctx)
            .ai_context_model()
            .as_ref(ctx)
            .selected_conversation_id(ctx);

        let active_prompt = if *SessionSettings::as_ref(ctx).honor_ps1 {
            ActivePrompt::PS1
        } else {
            let current_prompt_snapshot = prompt_type.as_ref(ctx).snapshot(ctx);
            let Ok(serialized_prompt) = serde_json::to_string(&current_prompt_snapshot) else {
                log::error!(
                    "Failed to serialize prompt snapshot to send active prompt update to shared session server"
                );
                return;
            };
            ActivePrompt::WarpPrompt(serialized_prompt)
        };

        let selection = terminal_view.read(ctx, |view, ctx| {
            view.get_shared_session_presence_selection(ctx)
        });

        let (events_tx, events_rx) = async_channel::unbounded();
        let input_replica_id = terminal_view
            .as_ref(ctx)
            .input()
            .as_ref(ctx)
            .editor()
            .as_ref(ctx)
            .replica_id(ctx);

        let scrollback_first_block_index = scrollback_type.first_block_index(&model.lock());

        // TODO: rather than picking which constructor we use here,
        // we might want to use a dedicated terminal manager for tests.
        cfg_if::cfg_if! {
            if #[cfg(any(test, feature = "integration_tests"))] {
                let _ = lifetime;
                let network = ctx.add_model(|ctx| Network::new_for_test(
                    model.clone(),
                    events_rx,
                    scrollback_type,
                    active_prompt,
                    selection,
                    input_replica_id,
                    ctx,
                ));
            } else {
                let input_config = terminal_view.as_ref(ctx).input_config(ctx);
                // Compute current auto-approve state from the AI context model
                let auto_approve_agent_actions = terminal_view
                    .as_ref(ctx)
                    .ai_context_model()
                    .as_ref(ctx)
                    .pending_query_autoexecute_override(ctx)
                    .is_autoexecute_any_action();

                // Get selected conversation token to send in initial context
                let agent_view_controller =
                    terminal_view.as_ref(ctx).agent_view_controller().clone();
                let context_model = terminal_view.as_ref(ctx).ai_context_model().clone();
                let selected_conversation: Option<SelectedConversation> =
                    build_selected_conversation_update(
                        &agent_view_controller,
                        &context_model,
                        ctx,
                    )
                    .and_then(|update| update.selected_conversation);

                let long_running_command_agent_interaction_state = {
                    let model = model.lock();
                    let active_block = model.block_list().active_block();
                    let state = if active_block.is_active_and_long_running() {
                        if active_block.is_agent_in_control() {
                            LongRunningCommandAgentInteractionState::InControl
                        } else if active_block.is_agent_tagged_in() {
                            LongRunningCommandAgentInteractionState::TaggedIn
                        } else {
                            LongRunningCommandAgentInteractionState::NotInteracting
                        }
                    } else {
                        LongRunningCommandAgentInteractionState::NotInteracting
                    };
                    Some(state)
                };

                // Include CLI agent session state in initial context so
                // late-joining viewers see the footer immediately.
                let terminal_view_id = terminal_view.id();
                let cli_agent_session = {
                    let sessions_model = CLIAgentSessionsModel::as_ref(ctx);
                    match sessions_model.session(terminal_view_id) {
                        Some(session) => CLIAgentSessionState::Active {
                            cli_agent: session.agent.to_serialized_name(),
                            is_rich_input_open: sessions_model.is_input_open(terminal_view_id),
                        },
                        None => CLIAgentSessionState::Inactive,
                    }
                };

                let universal_developer_input_context = UniversalDeveloperInputContext {
                    input_mode: Some(input_config.into()),
                    selected_conversation,
                    auto_approve_agent_actions: Some(auto_approve_agent_actions),
                    selected_model: None,
                    long_running_command_agent_interaction_state,
                    cli_agent_session,
                };

                let network = ctx.add_model(|ctx| {
                    Network::new(
                        model.clone(),
                        events_rx,
                        scrollback_type,
                        active_prompt,
                        selection,
                        input_replica_id,
                        terminal_view.id(),
                        universal_developer_input_context,
                        lifetime,
                        source.clone(),
                        ctx,
                    )
                });
            }
        }

        // Secret redaction relies on a lookback, so it can't work with
        // real-time session sharing.
        model
            .lock()
            .disable_secret_obfuscation_for_shared_sesson_creator(scrollback_first_block_index);

        // Set the event sender on the model for ordered terminal events.
        model
            .lock()
            .set_ordered_terminal_events_for_shared_session_tx(events_tx);

        let shared_session_model_clone = shared_session_model.clone();
        ctx.subscribe_to_model(&network, move |network, event, ctx| match event {
            NetworkEvent::SharedSessionCreatedSuccessfully {
                session_id,
                sharer_id,
                sharer_firebase_uid,
            } => {
                // Change the status of the session to reflect that the share is now active.
                model
                    .lock()
                    .set_shared_session_status(SharedSessionStatus::ActiveSharer);

                // Let the terminal view know the share is active so it can reflect that in its view.
                terminal_view.update(ctx, |view, ctx| {
                    view.on_session_share_started(
                        sharer_id.clone(),
                        *sharer_firebase_uid,
                        scrollback_type,
                        *session_id,
                        source.source_type.clone(),
                        ctx,
                    );

                    // Set the sharer's participant id on the AI controller for tracking query initiators
                    view.ai_controller().update(ctx, |controller, _ctx| {
                        controller.set_sharer_participant_id(sharer_id.clone());
                    });
                });
                Self::log_shared_session_lifecycle(
                    &terminal_view,
                    &model,
                    "session_established",
                    "outcome=active_sharer",
                    ctx,
                );

                // Let the manager know the share is active with the relevant metadata.
                Manager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.started_share(terminal_view.downgrade(), *session_id, window_id, ctx);
                });

                // Lifecycle event for downstream subscribers.
                if let Some(conversation_id) = selected_conversation_id {
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |_, ctx| {
                        ctx.emit(BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                            conversation_id,
                            session_id: *session_id,
                        });
                    });
                }

                // Flush the initial input operations that the sharer performed
                // in the latest buffer before the share was started.
                let is_ambient = model.lock().is_shared_ambient_agent_session();
                let init_input_ops: Vec<CrdtOperation> = terminal_view
                    .as_ref(ctx)
                    .input()
                    .as_ref(ctx)
                    .latest_buffer_operations()
                    .filter(|op| !should_skip_sharer_op(is_ambient, op))
                    .cloned()
                    .collect();
                if !init_input_ops.is_empty() {
                    network.update(ctx, |network, _ctx| {
                        network.send_input_update(
                            model.lock().block_list().active_block_id(),
                            init_input_ops.iter(),
                        );
                    });
                }

                // Stream historical agent conversations so viewers have conversation and task context.
                if FeatureFlag::AgentSharedSessions.is_enabled() {
                    Self::stream_historical_agent_conversations(&terminal_view, &model, ctx);
                }

                // `LocalAgentTaskSyncModel` fires the (task_id,
                // session_id) link in response to the event emitted above.
            }
            NetworkEvent::FailedToCreateSharedSession {
                reason,
                cause,
            } => {
                log::warn!("Failed to create shared session: reason={reason:?}, cause={cause:?}");

                model
                    .lock()
                    .set_shared_session_status(SharedSessionStatus::NotShared);

                Manager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.share_failed(window_id, ctx);
                });

                terminal_view.update(ctx, |view, ctx| {
                    let reason_string = failed_to_initialize_session_user_error(reason);

                    if matches!(
                        reason,
                        FailedToInitializeSessionReason::NoUserQuotaRemaining {
                            quota_type: QuotaType::SessionsCreated
                        }
                    ) {
                        view.open_share_session_denied_modal(ctx);
                    } else {
                        view.show_persistent_toast(reason_string.clone(), ToastFlavor::Error, ctx);
                    }

                    ctx.emit(TerminalViewEvent::FailedToShareSession {
                        reason: reason_string,
                        cause: cause.clone(),
                    });
                });

                // Drop the network so we can create a new one when trying again.
                shared_session_model_clone.borrow_mut().take();
            }
            NetworkEvent::SessionTerminated { reason } => {
                Self::shared_session_terminated(
                    &terminal_view,
                    shared_session_model_clone.clone(),
                    model.clone(),
                    ctx,
                );

                let max_session_size = network.as_ref(ctx).max_session_size();
                terminal_view.update(ctx, |view, ctx| {
                    let reason_string = session_terminated_reason_string(reason, max_session_size);
                    view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);
                });
            }
            NetworkEvent::Reconnecting => {
                // TODO(roland): add some limiting in a time frame to avoid possible infinite retry in this case:
                // Server disconnects
                // ---- begin loop
                // We reconnect here, and it's successful
                // The server immediately replies with a retryable error, or terminates the connection unexpectedly
                // We emit an event and attempt to reconnect immediately
                // ---- end loop
                terminal_view.update(ctx, |view, ctx| {
                    view.on_shared_session_reconnection_status_changed(true, ctx)
                });
            }
            NetworkEvent::ReconnectedSuccessfully => {
                terminal_view.update(ctx, |view, ctx| {
                    view.on_shared_session_reconnection_status_changed(false, ctx)
                });
            }
            NetworkEvent::FailedToReconnect => {
                Self::shared_session_terminated(
                    &terminal_view,
                    shared_session_model_clone.clone(),
                    model.clone(),
                    ctx,
                );

                terminal_view.update(ctx, |view, ctx| {
                    view.show_persistent_toast(
                        "Something went wrong. Please try sharing again.".to_string(),
                        ToastFlavor::Error,
                        ctx,
                    );
                });
            }
            NetworkEvent::ControlActionRequested {
                participant_id,
                request_id,
                action,
            } => {
                if !FeatureFlag::AgentSharedSessions.is_enabled() {
                    return;
                }

                let viewer_is_executor = terminal_view
                    .as_ref(ctx)
                    .shared_session_presence_manager()
                    .and_then(|manager| manager.as_ref(ctx).viewer_role(participant_id))
                    .map(|role| role.can_execute())
                    .unwrap_or_else(|| {
                        log::warn!("Failed to get viewer's role during control action request");
                        false
                    });

                if !viewer_is_executor {
                    network.update(ctx, |network, _ctx| {
                        network.send_control_action_rejection(
                            participant_id.clone(),
                            request_id.clone(),
                            ControlActionFailureReason::InsufficientPermissions,
                        );
                    });
                    return;
                };

                match action {
                    ControlAction::CancelConversation {
                        server_conversation_token,
                    } => {
                        terminal_view.update(ctx, |view, ctx| {
                            view.ai_controller().update(ctx, |controller, ctx| {
                                controller
                                    .handle_shared_session_cancel_action(*server_conversation_token, ctx);
                            });
                        });
                    }
                }
            }
            NetworkEvent::ParticipantListUpdated(participant_list) => {
                let was_viewer_driven_sizing_eligible = terminal_view
                    .update(ctx, |view, ctx| view.is_viewer_driven_sizing_eligible(true, ctx));

                if let Some(presence_manager) =
                    terminal_view.as_ref(ctx).shared_session_presence_manager()
                {
                    presence_manager.update(ctx, |presence_manager, ctx| {
                        presence_manager.update_participants(*participant_list.clone(), ctx)
                    });
                }

                // Check eligibility from the incoming participant list directly,
                // since the presence manager processes new viewers asynchronously.
                if was_viewer_driven_sizing_eligible {
                    let is_ambient_agent = terminal_view
                        .as_ref(ctx)
                        .is_shared_session_for_ambient_agent();
                    // We never want to reset back to the sharer size if we are a cloud agent,
                    // since it was a default. Prefer to keep the viewer-set size for transcript
                    // persistence.
                    if !is_ambient_agent {
                        let sharer_uid =
                            participant_list.sharer.info.profile_data.firebase_uid.as_str();
                        let still_eligible =
                            PresenceManager::single_distinct_present_viewer_uid_from_viewers(
                                participant_list.viewers.iter(),
                            )
                            .is_some_and(|viewer_uid| viewer_uid == sharer_uid);
                        if !still_eligible {
                            terminal_view.update(ctx, |view, ctx| {
                                view.restore_pty_to_sharer_size(ctx);
                            });
                        }
                    }
                }

                if let Some(session_id) = terminal_view.as_ref(ctx).shared_session_id().cloned() {
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
                terminal_view.update(ctx, |view, ctx| {
                    view.on_participant_presence_updated(update, ctx);
                });
            }
            NetworkEvent::RoleRequested {
                participant_id,
                role_request_id,
                role,
            } => {
                terminal_view.update(ctx, |view, ctx| {
                    view.on_role_requested(
                        participant_id.clone(),
                        role_request_id.clone(),
                        *role,
                        ctx,
                    );
                });
            }
            NetworkEvent::RoleRequestCancelled {
                participant_id,
                role_request_id,
            } => {
                terminal_view.update(ctx, |view, ctx| {
                    view.on_role_request_cancelled(
                        participant_id.clone(),
                        role_request_id.clone(),
                        ctx,
                    );
                });
            }
            NetworkEvent::ParticipantRoleChanged {
                participant_id,
                role,
            } => {
                terminal_view.update(ctx, |view, ctx| {
                    view.on_participant_role_changed(participant_id, *role, ctx);
                });
            }
            NetworkEvent::InputUpdated {
                block_id,
                operations,
            } => {
                // For the sharer, we're always up to speed so if this block ID
                // is not the latest, then it's an old block ID and we don't need
                // these operations.
                if model.lock().block_list().active_block_id() != block_id {
                    return;
                }

                terminal_view.update(ctx, |view, ctx| {
                    view.input().update(ctx, |input, ctx| {
                        input.process_remote_edits(block_id, operations.clone(), ctx);
                    });
                });
            }
            NetworkEvent::CommandExecutionRequested {
                id,
                participant_id,
                block_id,
                command,
            } => {
                let (is_block_id_latest, is_currently_long_running) = {
                    let model = model.lock();
                    let active_block = model.block_list().active_block();
                    (
                        active_block.id() == block_id,
                        active_block.is_active_and_long_running(),
                    )
                };

                // If the viewer is trying to execute for an old block ID (they can never be ahead)
                // or the active block is long running, we need to reject this request.
                if !is_block_id_latest || is_currently_long_running {
                    network.update(ctx, |network, _ctx| {
                        network.send_command_execution_rejection(
                            id.clone(),
                            participant_id.clone(),
                            CommandExecutionFailureReason::StaleBuffer,
                        );
                    });
                    return;
                }

                // If the viewer is no longer an executor, we need to reject the request.
                let Some(viewer_role) = terminal_view
                    .as_ref(ctx)
                    .shared_session_presence_manager()
                    .and_then(|manager| manager.as_ref(ctx).viewer_role(participant_id))
                else {
                    log::warn!("Failed to get viewer's role during command");
                    return;
                };
                if !viewer_role.can_execute() {
                    network.update(ctx, |network, _ctx| {
                        network.send_command_execution_rejection(
                            id.clone(),
                            participant_id.clone(),
                            CommandExecutionFailureReason::InsufficientPermissions,
                        );
                    });
                    return;
                }

                terminal_view.update(ctx, |view, ctx| {
                    view.input().update(ctx, |input, ctx| {
                        input.try_execute_command_on_behalf_of_shared_session_participant(
                            command,
                            participant_id.clone(),
                            ctx,
                        );
                    });
                });
            }
            NetworkEvent::WriteToPtyRequested { id, bytes } => {
                if !FeatureFlag::SharedSessionWriteToLongRunningCommands.is_enabled() {
                    return;
                }

                let is_currently_long_running = {
                    let model = model.lock();
                    model
                        .block_list()
                        .active_block()
                        .is_active_and_long_running()
                };
                if !is_currently_long_running {
                    network.update(ctx, |network, _ctx| {
                        network.send_write_to_pty_rejection(
                            id.clone(),
                            WriteToPtyFailureReason::StaleBuffer,
                        );
                    });
                    return;
                }

                // If the viewer is no longer an executor, we need to reject the request.
                let Some(viewer_role) = terminal_view
                    .as_ref(ctx)
                    .shared_session_presence_manager()
                    .and_then(|manager| manager.as_ref(ctx).viewer_role(&id.participant_id))
                else {
                    log::warn!("Failed to get viewer's role during write to pty requested");
                    return;
                };
                if !viewer_role.can_execute() {
                    network.update(ctx, |network, _ctx| {
                        network.send_write_to_pty_rejection(
                            id.clone(),
                            WriteToPtyFailureReason::InsufficientPermissions,
                        );
                    });
                    return;
                }

                terminal_view.update(ctx, |view, ctx| {
                    view.write_viewer_bytes_to_pty(bytes.clone(), ctx);
                });
            }
            NetworkEvent::AgentPromptRequested {
                id,
                participant_id,
                request,
            } => {
                if !FeatureFlag::AgentSharedSessions.is_enabled() {
                    return;
                }

                // Validate permissions for the participant that initiated the prompt.
                // For viewers, we require Executor role. For the sharer, we allow the prompt
                // even if they are not present in the viewer list.
                let mut is_sharer = false;
                let viewer_role_opt = terminal_view
                    .as_ref(ctx)
                    .shared_session_presence_manager()
                    .and_then(|manager| {
                        let manager_ref = manager.as_ref(ctx);
                        if manager_ref.sharer_id() == *participant_id {
                            is_sharer = true;
                            None
                        } else {
                            manager_ref.viewer_role(participant_id)
                        }
                    });

                if !is_sharer {
                    let Some(viewer_role) = viewer_role_opt else {
                        log::warn!(
                            "Failed to get viewer's role during agent prompt request for participant_id={participant_id} (not sharer)"
                        );
                        network.update(ctx, |network, _ctx| {
                            network.send_agent_prompt_rejection(
                                id.clone(),
                                participant_id.clone(),
                                AgentPromptFailureReason::InsufficientPermissions,
                            );
                        });
                        return;
                    };

                    if !viewer_role.can_execute() {
                        network.update(ctx, |network, _ctx| {
                            network.send_agent_prompt_rejection(
                                id.clone(),
                                participant_id.clone(),
                                AgentPromptFailureReason::InsufficientPermissions,
                            );
                        });
                        return;
                    }

                    // Reject the prompt if AI is disabled on the sharer's machine.
                    // TODO(APP-2894): We should create a failure variant that better matches the error.
                    if !crate::settings::ai::AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                        network.update(ctx, |network, _ctx| {
                            network.send_agent_prompt_rejection(
                                id.clone(),
                                participant_id.clone(),
                                AgentPromptFailureReason::InvalidConversation,
                            );
                        });
                        return;
                    }
                }

                // If a third-party CLI harness (e.g. Claude Code) is running, write
                // the follow-up prompt directly to the PTY. The CLI handles it as
                // interactive input. 
                let terminal_view_id = terminal_view.id();
                let has_active_cli_agent = CLIAgentSessionsModel::as_ref(ctx)
                    .session(terminal_view_id)
                    .is_some();
                if has_active_cli_agent {
                    // Reuse the rich input submit pipeline so agent-specific
                    // strategies are applied. Bypasses the rich-input-UI side effects 
  					// (telemetry, draft clear, editor buffer clear, pending-image consumption).
                    terminal_view.update(ctx, |view, ctx| {
                        view.submit_text_to_cli_agent_pty(request.prompt.clone(), ctx);
                    });
                    return;
                }

                // Execute the agent prompt in the Oz-harness case
                terminal_view.update(ctx, |view, ctx| {
                    // Clear the sharer's input (as the prompt in the input is now being executed)
                    view.input().update(ctx, |input, ctx| {
                        input.unfreeze_and_clear_agent_input(ctx);
                    });

                    view.ai_controller().update(ctx, |ai_controller, ctx| {
                        ai_controller.execute_agent_prompt_for_shared_session(
                            request.prompt.clone(),
                            request.server_conversation_token,
                            request.attachments.clone(),
                            participant_id.clone(),
                            ctx,
                        );
                    });
                });
            }
            NetworkEvent::LinkAccessLevelUpdateResponse { response } => {
                terminal_view.update(ctx, |view, ctx| match response {
                    LinkAccessLevelUpdateResponse::Ok { role } => {
                        let Some(session_id) = view.shared_session_id() else {
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
                        let reason_string =
                            "Failed to update permissions for shared session".to_owned();
                        view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);
                    }
                });
            }
            NetworkEvent::TeamAccessLevelUpdateResponse { response } => {
                terminal_view.update(ctx, |view, ctx| match response {
                    TeamAccessLevelUpdateResponse::Success { team_acl, .. } => {
                        let Some(session_id) = view.shared_session_id() else {
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
                        view.show_persistent_toast(
                            ACL_UPDATE_FAILURE_RESPONSE.to_owned(),
                            crate::view_components::ToastFlavor::Error,
                            ctx,
                        );
                    }
                });
            }
            NetworkEvent::AddGuestsResponse { response } => {
                if let AddGuestsResponse::Error(reason) = response {
                    terminal_view.update(ctx, |view, ctx| {
                        let reason_string = failed_to_add_guests_user_error(reason);
                        view.show_persistent_toast(reason_string, ToastFlavor::Error, ctx);
                    });
                }
            }
            NetworkEvent::RemoveGuestResponse { response } => {
                if let RemoveGuestResponse::Error(_) = response {
                    terminal_view.update(ctx, |view, ctx| {
                        view.show_persistent_toast(
                            ACL_UPDATE_FAILURE_RESPONSE.to_owned(),
                            crate::view_components::ToastFlavor::Error,
                            ctx,
                        );
                    });
                }
            }
            NetworkEvent::UpdatePendingUserRoleResponse { response } => {
                if let UpdatePendingUserRoleResponse::Error(_) = response {
                    terminal_view.update(ctx, |view, ctx| {
                        view.show_persistent_toast(
                            ACL_UPDATE_FAILURE_RESPONSE.to_owned(),
                            crate::view_components::ToastFlavor::Error,
                            ctx,
                        );
                    });
                }
            }
            NetworkEvent::ViewerTerminalSizeReported {
                window_size,
            } => {
                if !*SharedSessionSettings::as_ref(ctx).viewer_driven_sizing_enabled {
                    return;
                }
                let eligible = terminal_view
                    .update(ctx, |view, ctx| view.is_viewer_driven_sizing_eligible(true, ctx));
                if eligible {
                    terminal_view.update(ctx, |view, ctx| {
                        view.resize_from_viewer_report(*window_size, ctx);
                    });
                }
            }
            NetworkEvent::UniversalDeveloperInputContextUpdated(context_update) => {
                let active_remote_update = sharer_remote_update_guard.start_remote_update();

                if let Some(ref model) = context_update.selected_model {
                    let terminal_view_id = terminal_view.id();

                    // Update LLMPreferences to match the selected model received from the server.
                    apply_selected_agent_model_update(terminal_view_id, model, &active_remote_update, ctx);
                }
                if let Some(ref input_mode) = context_update.input_mode {
                    let weak_view_handle = terminal_view.downgrade();
                    apply_input_mode_update(&weak_view_handle, input_mode, &active_remote_update, ctx);
                }
                if let Some(ref selected_conversation) = context_update.selected_conversation {
                    let weak_view_handle = terminal_view.downgrade();
                    apply_selected_conversation_update(
                        &weak_view_handle,
                        selected_conversation,
                        &active_remote_update,
                        ctx,
                    );
                }
                if let Some(auto_approve) = context_update.auto_approve_agent_actions {
                    let weak_view_handle = terminal_view.downgrade();
                    apply_auto_approve_agent_actions_update(
                        &weak_view_handle,
                        auto_approve,
                        &active_remote_update,
                        ctx,
                    );
                }

                // Apply CLI agent rich input state from the viewer.
                if let Some(ref cli_agent_session) = context_update.cli_agent_session {
                    let weak_view_handle = terminal_view.downgrade();
                    apply_cli_agent_state_update(
                        &weak_view_handle,
                        cli_agent_session,
                        &active_remote_update,
                        ctx,
                    );
                }

                // Only apply agent control / tagged-in updates if there is an active long-running command.
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
                            "[sharer] UniversalDeveloperInputContextUpdated: \
                             applying LRC interaction_state={interaction_state:?}"
                        );
                        terminal_view.update(ctx, |view, ctx| {
                            view.apply_long_running_command_agent_interaction_state(
                                interaction_state,
                                ctx,
                            );
                        });
                    }
                }
            }
        });

        *session_sharer = Some(network);
    }

    fn log_shared_session_lifecycle(
        terminal_view: &ViewHandle<TerminalView>,
        model: &Arc<FairMutex<TerminalModel>>,
        event: &'static str,
        details: impl std::fmt::Display,
        ctx: &AppContext,
    ) {
        let session_id = terminal_view.as_ref(ctx).shared_session_id().cloned();
        let (source_type, source_task_id) = {
            let model = model.lock();
            match model.shared_session_source() {
                Some(source) => {
                    let source_type = match &source.source_type {
                        SessionSourceType::User => "user",
                        SessionSourceType::AmbientAgent { .. } => "ambient_agent",
                    };
                    (
                        source_type,
                        source.orchestrator_task_id().map(str::to_owned),
                    )
                }
                None => ("unknown", None),
            }
        };
        log::info!(
            "Shared session local lifecycle: event={event} session_id={session_id:?} source_type={source_type} source_task_id={source_task_id:?} {details}"
        );
    }

    /// Contains necessary logic for stopping the current shared session.
    fn cleanup_shared_session(
        terminal_view: &ViewHandle<TerminalView>,
        model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut AppContext,
    ) {
        let mut model_lock = model.lock();
        if !model_lock.shared_session_status().is_sharer() {
            log::warn!("Attempted to stop sharing current session that is not being shared");
            return;
        }

        // Change the status of the session to unshared.
        model_lock.set_shared_session_status(SharedSessionStatus::NotShared);
        model_lock.set_obfuscate_secrets(get_secret_obfuscation_mode(ctx));
        model_lock.clear_ordered_terminal_events_for_shared_session_tx();

        // Drop the lock so that it can be taken by the other entities that
        // need to do cleanup.
        drop(model_lock);

        // Let the manager know we've stopped sharing.
        Manager::handle(ctx).update(ctx, |manager, ctx| {
            manager.stopped_share(terminal_view.id(), ctx);
        });

        terminal_view.update(ctx, |view, ctx| {
            view.on_session_share_ended(ctx);
        });
    }

    /// Called when the server terminates the current session.
    fn shared_session_terminated(
        terminal_view: &ViewHandle<TerminalView>,
        session_sharer: Rc<RefCell<Option<ModelHandle<Network>>>>,
        model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut AppContext,
    ) {
        Self::cleanup_shared_session(terminal_view, model, ctx);
        // Drop the ModelHandle<Network> and set session_sharer to None.
        session_sharer.borrow_mut().take();
    }

    /// Called when the client explicitly wants to end the current session.
    /// Guarantees we also notify viewers of a session ended reason.
    fn end_shared_session(
        terminal_view: &ViewHandle<TerminalView>,
        session_sharer: Rc<RefCell<Option<ModelHandle<Network>>>>,
        reason: SessionEndedReason,
        model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut AppContext,
    ) {
        Self::log_shared_session_lifecycle(
            terminal_view,
            &model,
            "end_requested",
            format_args!("reason={reason:?}"),
            ctx,
        );
        Self::cleanup_shared_session(terminal_view, model, ctx);

        // Drop the ModelHandle<Network> and set session_sharer to None.
        if let Some(network_handle) = session_sharer.borrow_mut().take() {
            // Dropping the ModelHandle<Network> may not necessarily drop the Network within if there are other references to it, so we explicitly close the websocket just in case.
            // We also notify viewers with the given reason.
            network_handle.update(ctx, |network, _| network.end_session(reason));
        }
    }

    fn wire_up_session_sharer_with_view(
        terminal_view: &ViewHandle<TerminalView>,
        prompt_type: ModelHandle<PromptType>,
        shared_session_model: Rc<RefCell<Option<ModelHandle<Network>>>>,
        model: Arc<FairMutex<TerminalModel>>,
        window_id: WindowId,
        sharer_remote_update_guard: RemoteUpdateGuard,
        ctx: &mut AppContext,
    ) {
        let session_sharer = shared_session_model.clone();
        let model = model.clone();

        let is_ambient_agent = FeatureFlag::AgentSharedSessions.is_enabled()
            && AppExecutionMode::as_ref(ctx).is_autonomous();
        // TODO(ben): This is a very suboptimal way of exposing this; lifetime should be a user-visible option.
        let session_lifetime = if is_ambient_agent {
            Lifetime::Lingering
        } else {
            Lifetime::Ephemeral
        };

        // Clone before the subscribe_to_view closure moves the original.
        let sharer_remote_update_guard_for_cli = sharer_remote_update_guard.clone();
        ctx.subscribe_to_view(terminal_view, move |view, event, ctx| match event {
            TerminalViewEvent::StartSharingCurrentSession {
                scrollback_type,
                source,
            } if FeatureFlag::CreatingSharedSessions.is_enabled() => {
                Self::start_sharing_session(
                    view.clone(),
                    prompt_type.clone(),
                    session_sharer.clone(),
                    *scrollback_type,
                    session_lifetime,
                    source.clone(),
                    model.clone(),
                    window_id,
                    sharer_remote_update_guard.clone(),
                    ctx,
                );
            }
            TerminalViewEvent::StartSharingCurrentSession { .. } => {
                log::warn!(
                    "Ignoring request to start sharing current session because \
                     CreatingSharedSessions is disabled"
                );
            }
            TerminalViewEvent::StopSharingCurrentSession { reason } => {
                Self::end_shared_session(&view, session_sharer.clone(), *reason, model.clone(), ctx)
            }
            TerminalViewEvent::SelectedBlocksChanged | TerminalViewEvent::SelectedTextChanged => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    let selection = view.read(ctx, |view, ctx| {
                        view.get_shared_session_presence_selection(ctx)
                    });
                    network.update(ctx, |network, _| {
                        network.send_presence_selection_if_changed(selection);
                    });
                }
            }
            TerminalViewEvent::UpdateRole {
                participant_id,
                role,
            } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_role_update(participant_id.clone(), *role);
                    });
                }
            }
            TerminalViewEvent::UpdateUserRole { user_uid, role } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_user_role_update(*user_uid, *role);
                    });
                }
            }
            TerminalViewEvent::UpdatePendingUserRole { email, role } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_pending_user_role_update(email.clone(), *role);
                    });
                }
            }
            TerminalViewEvent::AddGuests { emails, role } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_add_guests(emails.clone(), *role);
                    });
                }
            }
            TerminalViewEvent::RemoveGuest { user_uid } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_remove_guest(*user_uid);
                    });
                }
            }
            TerminalViewEvent::RemovePendingGuest { email } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_remove_pending_guest(email.clone());
                    });
                }
            }
            TerminalViewEvent::MakeAllParticipantsReaders { reason } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_make_all_participants_readers(*reason);
                    });
                }
            }
            TerminalViewEvent::RespondToRoleRequest {
                participant_id,
                role_request_id,
                response,
            } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_role_request_response(
                            participant_id.clone(),
                            role_request_id.clone(),
                            response.clone(),
                        );
                    });
                }
            }
            TerminalViewEvent::InputEditorUpdated {
                block_id,
                operations,
            } => {
                // If the block ID has become stale by the time we get here,
                // we don't need to send this update to the server.
                if model.lock().block_list().active_block_id() != block_id {
                    return;
                }

                let is_ambient = model.lock().is_shared_ambient_agent_session();
                let filtered: Vec<_> = operations
                    .iter()
                    .filter(|op| !should_skip_sharer_op(is_ambient, op))
                    .collect();
                if filtered.is_empty() {
                    return;
                }
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_input_update(block_id, filtered.into_iter());
                    });
                }
            }
            TerminalViewEvent::UpdateSessionLinkPermissions { role } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_link_permission_update(*role);
                    });
                }
            }
            TerminalViewEvent::UpdateSessionTeamPermissions { role, team_uid } => {
                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_team_permission_update(*role, team_uid.clone());
                    });
                }
            }
            TerminalViewEvent::LongRunningCommandAgentInteractionStateChanged { state } => {
                if !sharer_remote_update_guard.should_broadcast() {
                    return;
                }

                if let Some(network) = session_sharer.borrow().as_ref() {
                    network.update(ctx, |network, _| {
                        network.send_universal_developer_input_context_update(
                            UniversalDeveloperInputContextUpdate {
                                long_running_command_agent_interaction_state: Some(*state),
                                ..Default::default()
                            },
                        )
                    });
                }
            }
            _ => (),
        });

        // Broadcast CLI agent session lifecycle events to viewers.
        let session_sharer_for_cli = shared_session_model.clone();
        let cli_guard = sharer_remote_update_guard_for_cli;
        let terminal_view_id = terminal_view.id();
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, ctx| {
            if event.terminal_view_id() != terminal_view_id || !cli_guard.should_broadcast() {
                return;
            }
            let Some(network) = session_sharer_for_cli.borrow().as_ref().cloned() else {
                return;
            };
            let update = match event {
                CLIAgentSessionsModelEvent::Started { agent, .. } => {
                    UniversalDeveloperInputContextUpdate {
                        cli_agent_session: Some(CLIAgentSessionState::Active {
                            cli_agent: agent.to_serialized_name(),
                            is_rich_input_open: false,
                        }),
                        ..Default::default()
                    }
                }
                CLIAgentSessionsModelEvent::InputSessionChanged {
                    agent,
                    new_input_state,
                    ..
                } => UniversalDeveloperInputContextUpdate {
                    cli_agent_session: Some(CLIAgentSessionState::Active {
                        cli_agent: agent.to_serialized_name(),
                        is_rich_input_open: matches!(
                            new_input_state,
                            &CLIAgentInputState::Open { .. }
                        ),
                    }),
                    ..Default::default()
                },
                CLIAgentSessionsModelEvent::Ended { .. } => UniversalDeveloperInputContextUpdate {
                    cli_agent_session: Some(CLIAgentSessionState::Inactive),
                    ..Default::default()
                },
                // StatusChanged / SessionUpdated are enriched by OSC events;
                // no protocol send needed.
                _ => return,
            };
            network.update(ctx, |network, _| {
                network.send_universal_developer_input_context_update(update);
            });
        });
    }

    fn handle_network_status_events(
        view: &ViewHandle<TerminalView>,
        session_sharer: Rc<RefCell<Option<ModelHandle<Network>>>>,
        ctx: &mut AppContext,
    ) {
        let weak_view_handle = view.downgrade();
        let network_status = NetworkStatus::handle(ctx);

        ctx.subscribe_to_model(&network_status, move |_, event, ctx| {
            let binding = session_sharer.borrow();
            let Some(network) = binding.as_ref() else {
                return;
            };
            let Some(view) = weak_view_handle.upgrade(ctx) else {
                return;
            };
            let NetworkStatusEvent::NetworkStatusChanged { new_status } = event;
            match new_status {
                NetworkStatusKind::Online => {
                    if network.as_ref(ctx).is_connected() {
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

    #[cfg(feature = "integration_tests")]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    #[cfg(test)]
    pub fn session_sharer(&self) -> Rc<RefCell<Option<ModelHandle<Network>>>> {
        self.session_sharer.clone()
    }
}

/// Determine whether to show password notifications based on the user's settings.
/// This returns true if the user hasn't set notification settings yet, or if
/// they have explicitly enabled notifications for password prompts.
#[cfg(unix)]
fn show_password_notifications(
    ctx: &ModelContext<Box<dyn crate::terminal::TerminalManager>>,
) -> bool {
    let notification_settings = &SessionSettings::as_ref(ctx).notifications;
    matches!(notification_settings.mode, NotificationsMode::Unset)
        || (matches!(notification_settings.mode, NotificationsMode::Enabled)
            && notification_settings.is_needs_attention_enabled)
}

pub fn get_shell_starter(
    chosen_shell: Option<AvailableShell>,
    auth_state: &AuthState,
    ctx: &mut AppContext,
) -> Option<ShellStarter> {
    let preferred_shell = chosen_shell.unwrap_or_else(|| {
        AvailableShells::handle(ctx).read(ctx, |shells, ctx| shells.get_user_preferred_shell(ctx))
    });
    let shell_starter_or_wsl_name = ShellStarter::init(preferred_shell);

    // TODO(alokedesai): Further refactor this function to make it clear that it's expensive.
    shell_starter_or_wsl_name
        .and_then(|starter| {
            warpui::r#async::block_on(async { starter.to_shell_starter_source().await })
        })
        .map(|starter_source| {
            get_shell_starter_internal(
                starter_source,
                ctx.background_executor().clone(),
                auth_state,
            )
        })
}

fn get_shell_starter_internal(
    shell_starter_source: ShellStarterSource,
    background_executor: Arc<Background>,
    auth_state: &AuthState,
) -> ShellStarter {
    match shell_starter_source {
        ShellStarterSource::Override(shell_starter) => shell_starter,
        ShellStarterSource::Environment(starter) | ShellStarterSource::UserDefault(starter) => {
            ShellStarter::Direct(starter)
        }
        ShellStarterSource::Fallback {
            unsupported_shell,
            starter,
        } => {
            if let Some(unsupported_shell) = unsupported_shell {
                send_telemetry_on_executor!(
                    auth_state,
                    TelemetryEvent::UnsupportedShell {
                        shell: unsupported_shell
                    },
                    background_executor
                );
            }

            ShellStarter::Direct(starter)
        }
    }
}

/// Send a Shutdown event to each PTY's event loop and waits for the
/// event loop to terminate.
/// This is needed on Windows to ensure all OpenConsole processes are
/// cleaned up before the main thread exits.
#[cfg(windows)]
pub fn shutdown_all_pty_event_loops(ctx: &mut AppContext) {
    let terminal_managers: Vec<ModelHandle<Box<dyn crate::terminal::TerminalManager>>> =
        ctx.models_of_type();
    terminal_managers.into_iter().for_each(|terminal_manager| {
        terminal_manager.update(ctx, |terminal_manager, _ctx| {
            if let Some(manager) = terminal_manager
                .as_any_mut()
                .downcast_mut::<TerminalManager>()
            {
                manager.shutdown_event_loop();
            }
        })
    })
}

impl crate::terminal::TerminalManager for TerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn view(&self) -> ViewHandle<TerminalView> {
        self.view.clone()
    }

    fn on_view_detached(
        &self,
        // The detach type is intentionally ignored: a sharer always stops sharing immediately,
        // even on a reversible `HiddenForClose` detach. This is desirable for security — a sharer
        // should not continue accepting commands from viewers while the session is not visible.
        detach_type: crate::pane_group::pane::DetachType,
        app: &mut AppContext,
    ) {
        let shared_session_status = self.model.lock().shared_session_status().clone();
        if shared_session_status.is_sharer() {
            Self::log_shared_session_lifecycle(
                &self.view,
                &self.model,
                "view_detached",
                format_args!("detach_type={detach_type:?}"),
                app,
            );
            let is_confirm_close_session =
                *SessionSettings::as_ref(app).should_confirm_close_session;
            self.view.update(app, |terminal_view, ctx| {
                // This emits an event that is handled in [`Self::end_shared_session`].
                // We still need to call this in order to emit a telemetry event.
                terminal_view.stop_sharing_session(
                    SharedSessionActionSource::Closed {
                        is_confirm_close_session,
                    },
                    ctx,
                )
            });
            // The window could close before the event from above is processed, so directly stop sharing here.
            Self::end_shared_session(
                &self.view,
                self.session_sharer.clone(),
                SessionEndedReason::EndedBySharer,
                self.model.clone(),
                app,
            )
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl EventLoopSender for mio_channel::Sender<Message> {
    fn send(&self, message: Message) -> Result<(), EventLoopSendError> {
        self.send(message).map_err(|error| match error {
            SendError(_) => EventLoopSendError::Disconnected,
        })
    }
}
