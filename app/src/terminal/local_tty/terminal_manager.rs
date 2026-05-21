use crate::ai::aws_credentials::AwsCredentialRefresher as _;
use crate::terminal::model::terminal_model::ExitReason;
use crate::terminal::shell::ShellName;
use crate::terminal::TerminalManager as _;
use anyhow::Context as _;
use async_broadcast::InactiveReceiver;
use std::any::Any;
use std::sync::mpsc::{SendError, SyncSender};
use std::{collections::HashMap, ffi::OsString, path::PathBuf, sync::Arc, thread::JoinHandle};

use crate::terminal::available_shells::{AvailableShell, AvailableShells};
use crate::terminal::ShellLaunchData;
use crate::terminal::ShellLaunchState;

use parking_lot::{FairMutex, Mutex};
use pathfinder_geometry::vector::Vector2F;

use settings::Setting as _;
use warpui::{AppContext, ModelContext, ModelHandle, SingletonEntity, ViewHandle, WindowId};

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::blocklist::{InputConfig, SerializedBlockListItem};
use crate::terminal::view::ConversationRestorationInNewPaneType;

use crate::banner::BannerState;
use crate::context_chips::current_prompt::CurrentPrompt;
use crate::context_chips::prompt_type::PromptType;
use crate::features::FeatureFlag;
use crate::pane_group::TerminalViewResources;
use crate::persistence::ModelEvent;
use crate::settings::DebugSettings;

use crate::terminal::model::session::Sessions;

use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::session_settings::{SessionSettings, SessionSettingsChangedEvent};
use crate::terminal::writeable_pty::pty_controller::{EventLoopSendError, EventLoopSender};
use crate::terminal::writeable_pty::terminal_manager_util::{
    init_pty_controller_model, wire_up_pty_controller_with_view,
};
use crate::terminal::writeable_pty::{self, Message};
use crate::terminal::{
    event_listener::ChannelEventListener,
    local_tty::{Pty, PtyOptions},
    TerminalModel,
};
use crate::terminal::{terminal_manager, TerminalView, PTY_READS_BROADCAST_CHANNEL_SIZE};

use super::mio_channel;
use super::recorder;
use super::shell::ShellStarter;
use super::{event_loop::EventLoop, shell::ShellStarterSource};

#[cfg(unix)]
use {
    super::terminal_attributes::TerminalAttributesPoller,
    crate::terminal::local_tty::terminal_attributes::Event as TerminalAttributesPollerEvent,
    crate::terminal::model::terminal_model::BlockIndex,
    crate::terminal::session_settings::NotificationsMode,
    crate::terminal::view::Event as TerminalViewEvent, nix::sys::termios::LocalFlags,
    std::cell::RefCell, std::rc::Rc,
};

type PtyController = writeable_pty::PtyController<mio_channel::Sender<Message>>;
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

        let wsl_name_or_shell_starter = ShellStarter::init(preferred_shell.clone());

        // If we have explicit restored_blocks, prioritize those (these come from db on startup).
        // Otherwise if there's a conversation we're restoring, get blocks from those.
        let all_restored_blocks =
            restored_blocks
                .cloned()
                .or_else(|| match &conversation_restoration {
                    Some(ConversationRestorationInNewPaneType::Historical {
                        conversation, ..
                    })
                    | Some(ConversationRestorationInNewPaneType::Forked { conversation, .. }) => {
                        Some(conversation.to_serialized_blocklist_items())
                    }
                    _ => None,
                });

        // Create the terminal model with all restored blocks
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

        // Initialize the PtyController.
        let pty_controller = init_pty_controller_model(
            event_loop_tx.clone(),
            executor_command_rx,
            model_events.clone(),
            sessions.clone(),
            model.clone(),
            ctx,
        );

        let current_prompt = ctx.add_model(|ctx| {
            CurrentPrompt::new_with_model_events(sessions.clone(), Some(&model_events), ctx)
        });
        let prompt_type = ctx.add_model(|ctx| PromptType::new_dynamic(current_prompt.clone(), ctx));
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

        wire_up_pty_controller_with_view(
            &pty_controller,
            &view,
            model.clone(),
            sessions,
            model_event_sender,
            ctx,
        );

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), move |_, event, ctx| {
            if let SessionSettingsChangedEvent::HonorPS1 { .. } = event {
                let _ = *SessionSettings::as_ref(ctx).honor_ps1;
            }
        });

        let terminal_view_id = view.id();
        let agent_view_controller = view.as_ref(ctx).agent_view_controller().clone();
        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_agent_view_controller(&agent_view_controller, terminal_view_id, ctx);
        });

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

            #[cfg(feature = "integration_tests")]
            pid: None,

            inactive_pty_reads_rx,
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
        let is_fallback_shell = matches!(
            shell_starter_source,
            Some(ShellStarterSource::Fallback { .. })
        );
        let shell_starter = shell_starter_source.map(get_shell_starter_internal);
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
            recorder::record_pty_throughput(
                pty_reads_rx.activate(),
                model,
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
        let enable_ssh_wrapper = false;

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

    #[cfg(feature = "integration_tests")]
    pub fn pid(&self) -> Option<u32> {
        self.pid
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
        .map(get_shell_starter_internal)
}

fn get_shell_starter_internal(shell_starter_source: ShellStarterSource) -> ShellStarter {
    match shell_starter_source {
        ShellStarterSource::Override(shell_starter) => shell_starter,
        ShellStarterSource::Environment(starter) | ShellStarterSource::UserDefault(starter) => {
            ShellStarter::Direct(starter)
        }
        ShellStarterSource::Fallback { starter } => ShellStarter::Direct(starter),
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
        _detach_type: crate::pane_group::pane::DetachType,
        _app: &mut AppContext,
    ) {
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
