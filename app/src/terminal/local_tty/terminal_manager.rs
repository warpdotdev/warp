use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{SendError, SyncSender};
use std::thread::JoinHandle;

use anyhow::Context as _;
use async_broadcast::InactiveReceiver;
#[cfg(unix)]
use nix::sys::termios::LocalFlags;
use parking_lot::{FairMutex, Mutex};
use pathfinder_geometry::vector::Vector2F;
use settings::Setting as _;
use warp_core::SessionId;
use warp_errors::report_error;
use warpui::r#async::executor::Background;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity, ViewHandle};

use super::event_loop::EventLoop;
use super::shell::{ShellStarter, ShellStarterSource};
#[cfg(unix)]
use super::terminal_attributes::TerminalAttributesPoller;
use super::{mio_channel, recorder};
use crate::ai::aws_credentials::AwsCredentialRefresher as _;
use crate::ai::blocklist::SerializedBlockListItem;
use crate::auth::AuthStateProvider;
use crate::auth::auth_state::AuthState;
use crate::banner::BannerState;
use crate::context_chips::ContextChipKind;
use crate::context_chips::prompt::Prompt;
use crate::features::FeatureFlag;
use crate::persistence::ModelEvent;
use crate::send_telemetry_on_executor;
use crate::server::telemetry::TelemetryEvent;
use crate::settings::{DebugSettings, PrivacySettings, SshSettings};
use crate::terminal::available_shells::{AvailableShell, AvailableShells};
use crate::terminal::color::List as ColorList;
use crate::terminal::event_listener::ChannelEventListener;
#[cfg(unix)]
use crate::terminal::local_tty::terminal_attributes::Event as TerminalAttributesPollerEvent;
use crate::terminal::local_tty::{Pty, PtyOptions};
use crate::terminal::model::session::Sessions;
#[cfg(unix)]
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::model::terminal_model::ExitReason;
#[cfg(unix)]
use crate::terminal::model_events::ModelEvent as TerminalModelEvent;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::session_settings::{SessionSettings, ToolbarChipSelection};
use crate::terminal::shared_session::sharer::network::Network;
use crate::terminal::shared_session::{IsSharedSessionCreator, SharedSessionStatus};
use crate::terminal::shell::ShellName;
use crate::terminal::terminal_manager::BlockSpacing;
use crate::terminal::warpify::settings::WarpifySettings;
use crate::terminal::writeable_pty::pty_controller::{EventLoopSendError, EventLoopSender};
use crate::terminal::writeable_pty::terminal_manager_util::{
    init_pty_controller_model, init_remote_server_controller, wire_up_pty_controller_with_surface,
};
use crate::terminal::writeable_pty::{self, Message, PtyIntentEvent, TerminalSurface};
use crate::terminal::{
    PTY_READS_BROADCAST_CHANNEL_SIZE, ShellLaunchData, ShellLaunchState, SizeInfo,
    TerminalManager as TerminalManagerTrait, TerminalModel, terminal_manager,
};

type PtyController = writeable_pty::PtyController<mio_channel::Sender<Message>>;
type RemoteServerController =
    writeable_pty::remote_server_controller::RemoteServerController<mio_channel::Sender<Message>>;

/// Owns a local terminal session: the terminal model, PTY event loop, PTY
/// controller, and a terminal surface.
///
/// Holds onto data that needs to live as long as the session does (e.g. the
/// event loop join handle).
pub struct TerminalManager<S> {
    event_loop_tx: Arc<Mutex<mio_channel::Sender<Message>>>,
    /// This is an `Option` so that we can take ownership of the inner
    /// `JoinHandle` in `TerminalManager::drop`.
    event_loop_handle: Option<JoinHandle<()>>,
    pub(super) model: Arc<FairMutex<TerminalModel>>,
    pub(super) view: ViewHandle<S>,

    /// The manager is responsible for managing the lifetime
    /// of the terminal attributes poller. None if the event loop has not yet started.
    #[cfg(unix)]
    #[allow(dead_code)]
    terminal_attributes_poller: Option<ModelHandle<TerminalAttributesPoller>>,

    /// The manager is responsible for managing the lifetime
    /// of the PTY controller.
    pty_controller: ModelHandle<PtyController>,

    /// The manager is responsible for managing the lifetime of the remote server controller.
    remote_server_controller: ModelHandle<RemoteServerController>,

    /// The process ID of the PTY. Purely used for integration tests. None if the PTY has not yet
    /// been started.
    #[cfg(feature = "integration_tests")]
    pub(super) pid: Option<u32>,

    /// An inactive receiver for PTY reads that we can upgrade to an active
    /// receiver as needed. We prefer to not create active receivers eagerly
    /// to avoid unnecessary allocations of data coming from the PTY (high throughput).
    /// Note that we need to hold onto the inactive receiver so that the channel isn't closed prematurely.
    inactive_pty_reads_rx: InactiveReceiver<Arc<Vec<u8>>>,

    /// The sharer side of the session sharing protocol. [`Some`] only when a
    /// shared session connection is ongoing.
    pub(super) session_sharer: Rc<RefCell<Option<ModelHandle<Network>>>>,
}

/// Shared inputs needed to construct a terminal surface for a local PTY.
pub struct TerminalSurfaceInit {
    pub wakeups_rx: async_channel::Receiver<()>,
    pub model_events: ModelHandle<ModelEventDispatcher>,
    pub model: Arc<FairMutex<TerminalModel>>,
    pub sessions: ModelHandle<Sessions>,
    pub size_info: SizeInfo,
    pub colors: ColorList,
    pub inactive_pty_reads_rx: InactiveReceiver<Arc<Vec<u8>>>,
}

#[cfg(any(test, all(feature = "tui", feature = "test-util")))]
impl TerminalSurfaceInit {
    /// Creates mock terminal surface inputs without spawning a PTY.
    pub fn new_for_test(ctx: &mut AppContext) -> Self {
        let (_wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (_events_tx, events_rx) = async_channel::unbounded();
        let (pty_reads_tx, pty_reads_rx) =
            async_broadcast::broadcast(PTY_READS_BROADCAST_CHANNEL_SIZE);
        drop(pty_reads_tx);
        let sessions = ctx.add_model(|_| Sessions::new_for_test());
        let model_events =
            ctx.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));
        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let colors = model.lock().colors();
        let size_info = model.lock().block_list().size().to_owned();
        Self {
            wakeups_rx,
            model_events,
            model,
            sessions,
            size_info,
            colors,
            inactive_pty_reads_rx: pty_reads_rx.deactivate(),
        }
    }
}

/// A newly constructed terminal surface and its manager post-wiring callback.
pub struct TerminalSurfaceResult<S, PostWire> {
    pub surface: ViewHandle<S>,
    pub post_wire: PostWire,
}

/// One-shot resources consumed when the shell is determined and the PTY starts.
struct ShellStartupResources {
    event_loop_rx: mio_channel::Receiver<Message>,
    channel_event_proxy: ChannelEventListener,
    #[cfg(unix)]
    model_events: ModelHandle<ModelEventDispatcher>,
}

/// Handles created for a local terminal manager and its surface.
pub struct TerminalManagerInit<S> {
    pub manager: ModelHandle<Box<dyn TerminalManagerTrait>>,
    pub surface: ViewHandle<S>,
}
/// Adapts a TUI-owned surface manager to Warp's type-erased manager contract.
struct TuiTerminalManager<S>(TerminalManager<S>);

impl<S: 'static> TerminalManagerTrait for TuiTerminalManager<S> {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.0.model()
    }

    fn as_any(&self) -> &dyn Any {
        &self.0
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut self.0
    }
}

impl<S> Drop for TerminalManager<S> {
    fn drop(&mut self) {
        self.shutdown_event_loop();
    }
}

impl<S> TerminalManager<S> {
    /// Creates a local terminal manager model and terminal surface.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_model<PostWire>(
        startup_directory: Option<PathBuf>,
        env_vars: HashMap<OsString, OsString>,
        is_shared_session_creator: IsSharedSessionCreator,
        all_restored_blocks: Option<&Vec<SerializedBlockListItem>>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        initial_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        chosen_shell: Option<AvailableShell>,
        ctx: &mut AppContext,
        create_surface: impl FnOnce(
            TerminalSurfaceInit,
            &mut AppContext,
        ) -> TerminalSurfaceResult<S, PostWire>,
    ) -> TerminalManagerInit<S>
    where
        S: TerminalSurface,
        <S as Entity>::Event: PtyIntentEvent,
        Self: TerminalManagerTrait,
        PostWire: FnOnce(&mut Self, &ViewHandle<S>, &mut AppContext),
    {
        Self::create_model_with_manager(
            startup_directory,
            env_vars,
            is_shared_session_creator,
            all_restored_blocks,
            user_default_shell_unsupported_banner_model_handle,
            initial_size,
            model_event_sender,
            chosen_shell,
            BlockSpacing::for_gui(ctx),
            ctx,
            create_surface,
            |manager| Box::new(manager),
        )
    }

    /// Creates a local terminal manager for a TUI-owned terminal surface.
    /// `block_spacing` is the TUI frontend's spacing baked into block heights.
    #[allow(clippy::too_many_arguments)]
    pub fn create_tui_model<PostWire>(
        startup_directory: Option<PathBuf>,
        env_vars: HashMap<OsString, OsString>,
        is_shared_session_creator: IsSharedSessionCreator,
        all_restored_blocks: Option<&Vec<SerializedBlockListItem>>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        initial_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        chosen_shell: Option<AvailableShell>,
        block_spacing: BlockSpacing,
        ctx: &mut AppContext,
        create_surface: impl FnOnce(
            TerminalSurfaceInit,
            &mut AppContext,
        ) -> TerminalSurfaceResult<S, PostWire>,
    ) -> TerminalManagerInit<S>
    where
        S: TerminalSurface,
        <S as Entity>::Event: PtyIntentEvent,
        PostWire: FnOnce(&mut Self, &ViewHandle<S>, &mut AppContext),
    {
        Self::create_model_with_manager(
            startup_directory,
            env_vars,
            is_shared_session_creator,
            all_restored_blocks,
            user_default_shell_unsupported_banner_model_handle,
            initial_size,
            model_event_sender,
            chosen_shell,
            block_spacing,
            ctx,
            create_surface,
            |manager| Box::new(TuiTerminalManager(manager)),
        )
    }

    /// Creates a manager using the supplied type-erasure adapter.
    #[allow(clippy::too_many_arguments)]
    fn create_model_with_manager<PostWire, BoxManager>(
        startup_directory: Option<PathBuf>,
        env_vars: HashMap<OsString, OsString>,
        is_shared_session_creator: IsSharedSessionCreator,
        all_restored_blocks: Option<&Vec<SerializedBlockListItem>>,
        user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
        initial_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        chosen_shell: Option<AvailableShell>,
        block_spacing: BlockSpacing,
        ctx: &mut AppContext,
        create_surface: impl FnOnce(
            TerminalSurfaceInit,
            &mut AppContext,
        ) -> TerminalSurfaceResult<S, PostWire>,
        box_manager: BoxManager,
    ) -> TerminalManagerInit<S>
    where
        S: TerminalSurface,
        <S as Entity>::Event: PtyIntentEvent,
        PostWire: FnOnce(&mut Self, &ViewHandle<S>, &mut AppContext),
        BoxManager: FnOnce(Self) -> Box<dyn TerminalManagerTrait> + 'static,
    {
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
            all_restored_blocks,
            initial_size,
            channel_event_proxy.clone(),
            ShellLaunchState::DeterminingShell {
                available_shell: Some(preferred_shell),
                display_name: wsl_name_or_shell_starter
                    .as_ref()
                    .map(|wsl_name_or_shell_starter| wsl_name_or_shell_starter.name())
                    .unwrap_or(ShellName::LessDescriptive("Shell".to_owned())),
            },
            block_spacing,
            ctx,
        );
        let colors = model.colors();
        let model = Arc::new(FairMutex::new(model));

        // This is purely for measuring throughput on WarpDev.
        if FeatureFlag::RecordPtyThroughput.is_enabled() {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            recorder::record_pty_throughput(
                inactive_pty_reads_rx.clone().activate(),
                model.clone(),
                auth_state.clone(),
                ctx.background_executor().to_owned(),
            );
        }

        // If this session should be a shared-session creator, configure its initial
        // shared-session state before the surface is constructed, so that bootstrap
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
        let size_info = model.lock().block_list().size().to_owned();
        let TerminalSurfaceResult { surface, post_wire } = create_surface(
            TerminalSurfaceInit {
                wakeups_rx,
                model_events: model_events.clone(),
                model: model.clone(),
                sessions: sessions.clone(),
                size_info,
                colors,
                inactive_pty_reads_rx: inactive_pty_reads_rx.clone(),
            },
            ctx,
        );
        wire_up_pty_controller_with_surface(
            &pty_controller,
            &surface,
            model.clone(),
            sessions.clone(),
            model_event_sender,
            ctx,
        );
        let mut terminal_manager = Self {
            event_loop_tx: Arc::new(Mutex::new(event_loop_tx)),
            model,
            event_loop_handle: None,
            view: surface.clone(),
            #[cfg(unix)]
            terminal_attributes_poller: None,
            pty_controller,
            remote_server_controller,
            #[cfg(feature = "integration_tests")]
            pid: None,
            inactive_pty_reads_rx,
            session_sharer: Rc::new(RefCell::new(None)),
        };

        // Run surface-specific wiring after the manager exists, because this
        // step may need manager-owned controllers and retained handles.
        post_wire(&mut terminal_manager, &surface, ctx);

        let terminal_surface = surface.clone();
        let shell_startup_resources = ShellStartupResources {
            event_loop_rx,
            channel_event_proxy,
            #[cfg(unix)]
            model_events,
        };

        let terminal_manager_model = ctx.add_model(|ctx| {
            let terminal_manager = box_manager(terminal_manager);
            ctx.spawn(
                async move {
                    match wsl_name_or_shell_starter {
                        Some(starter_source) => starter_source.to_shell_starter_source().await,
                        None => None,
                    }
                },
                move |terminal_manager: &mut Box<dyn TerminalManagerTrait>,
                      shell_starter_source,
                      ctx| {
                    let Some(terminal_manager) =
                        TerminalManagerTrait::as_any_mut(terminal_manager.as_mut())
                            .downcast_mut::<Self>()
                    else {
                        return;
                    };

                    on_shell_determined(
                        terminal_manager,
                        startup_directory,
                        env_vars,
                        user_default_shell_unsupported_banner_model_handle,
                        shell_startup_resources,
                        shell_starter_source,
                        ctx,
                    )
                },
            );

            terminal_manager
        });

        TerminalManagerInit {
            manager: terminal_manager_model,
            surface: terminal_surface,
        }
    }

    /// Returns the terminal model owned by this manager.
    pub(crate) fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    /// Returns the remote server controller owned by this manager.
    pub(super) fn remote_server_controller(&self) -> ModelHandle<RemoteServerController> {
        self.remote_server_controller.clone()
    }

    /// Sends a shutdown message to the PTY event loop and waits for it to
    /// process that event.
    pub(super) fn shutdown_event_loop(&mut self) {
        let shutdown_res = self.event_loop_tx.lock().send(Message::Shutdown);
        // Happens normally if the event loop has already been terminated (so the channel is now gone).
        if let Err(e) = shutdown_res {
            log::info!("Failed to send Shutdown {e:?}");
        }

        match self.event_loop_handle.take() {
            Some(join_handle) => {
                if let Err(e) = join_handle.join() {
                    report_error!(
                        "Failed to join event loop handle",
                        extra: { "error" => ?e }
                    );
                }
            }
            _ => {
                log::warn!("No event loop handle to join when dropping terminal manager.");
            }
        }

        self.inactive_pty_reads_rx.close();
    }
}

/// Callback invoked upon determining the shell to be spawned when starting the event loop.
#[allow(clippy::too_many_arguments)]
fn on_shell_determined<S: TerminalSurface>(
    manager: &mut TerminalManager<S>,
    startup_directory: Option<PathBuf>,
    env_vars: HashMap<OsString, OsString>,
    user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
    shell_startup_resources: ShellStartupResources,
    shell_starter_source: Option<ShellStarterSource>,
    ctx: &mut ModelContext<Box<dyn TerminalManagerTrait>>,
) where
    <S as Entity>::Event: PtyIntentEvent,
{
    // This is executed as a callback and the window could be closed in the interim.
    if !ctx.is_window_open(manager.view.window_id(ctx)) {
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
            report_error!("Could not compute fallback shell");
            manager.view.update(ctx, |surface, ctx| {
                surface.on_pty_spawn_failed(
                    anyhow::Error::msg("Could not find a fallback shell. If you have PowerShell or WSL installed, please file an issue."),
                    ctx,
                );
            });
            manager.model().lock().exit(ExitReason::ShellNotFound);
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

    manager
        .model()
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
    manager
        .model()
        .lock()
        .set_pending_shell_launch_data(shell_launch_data.clone());

    // Register the session ID that was generated during shell starter construction.
    // For bash, fish, and PowerShell, the session ID is already baked into the command
    // args. For zsh and MSYS2, enqueue_init_script injects this same ID.
    let generated_session_id = match &shell_starter {
        ShellStarter::Direct(starter) | ShellStarter::MSYS2(starter) => starter.session_id(),
        ShellStarter::DockerSandbox(starter) => starter.session_id(),
        ShellStarter::Wsl(starter) => starter.session_id(),
    };
    manager
        .model()
        .lock()
        .register_session_id(generated_session_id);

    // Enqueue the init shell script (for shells that need it), then create
    // the PTY and start its corresponding event loop.
    let ShellStartupResources {
        event_loop_rx,
        channel_event_proxy,
        #[cfg(unix)]
        model_events,
    } = shell_startup_resources;
    let model = manager.model();
    #[cfg(windows)]
    let event_loop_tx = manager.event_loop_tx.lock().clone();
    let pty = match manager
        .enqueue_init_script(&shell_starter, generated_session_id)
        .context("Failed to write shell init script to the pty")
        .and_then(|_| {
            TerminalManager::<S>::create_pty(
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
            report_error!(&err);
            manager.view.update(ctx, |surface, ctx| {
                surface.on_pty_spawn_failed(err, ctx);
            });
            manager.model().lock().exit(ExitReason::PtySpawnFailed);
            return;
        }
    };

    #[cfg(feature = "integration_tests")]
    let pid = pty.get_pid();
    #[cfg(unix)]
    let fd = pty.get_fd();

    // Create the channel above and pass the receving side to the event loop.
    let event_loop_handle = TerminalManager::<S>::start_pty_event_loop(
        pty,
        event_loop_rx,
        model.clone(),
        channel_event_proxy,
    );

    manager.event_loop_handle = Some(event_loop_handle);
    #[cfg(feature = "integration_tests")]
    {
        manager.pid = Some(pid);
    }

    manager.view.update(ctx, |surface, ctx| {
        surface.on_shell_determined(ctx);
        surface.on_active_shell_launch_data_updated(Some(shell_launch_data), ctx);
    });

    // Initialize the terminal attributes poller.
    // TODO(CORE-2297): Implement TerminalPoller on Windows.
    #[cfg(unix)]
    {
        let terminal_attributes_poller = ctx.add_model(|_| TerminalAttributesPoller::new(fd));
        wire_up_terminal_attribute_poller_with_surface(
            &terminal_attributes_poller,
            &manager.view,
            &model_events,
            model.clone(),
            ctx,
        );

        manager.terminal_attributes_poller = Some(terminal_attributes_poller);
    }
}

impl<S> TerminalManager<S> {
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

    fn enqueue_init_script(
        &self,
        shell_starter: &ShellStarter,
        session_id: SessionId,
    ) -> Result<(), SendError<Message>> {
        let shell_type = shell_starter.shell_type();
        if shell_type == crate::terminal::shell::ShellType::Zsh
            // For more on why this is necessary on Git Bash, see https://linear.app/warpdotdev/issue/CORE-3202.
            || shell_starter.is_msys2()
        {
            let init_shell_script = crate::terminal::bootstrap::init_shell_script_for_shell(
                shell_type,
                &crate::ASSETS,
                session_id,
            );
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

        // Determine whether the Node.js Version chip is enabled anywhere it could be
        // shown (the Warp prompt, the agent footer, or the CLI agent footer). When it
        // is not, the shell bootstrap skips the expensive per-prompt `node --version`
        // detection. The chip value is fed by the same precmd payload regardless of
        // where it is displayed, so we must check all three locations.
        let node_version_chip_enabled = {
            let in_prompt = !is_honor_ps1_enabled
                && Prompt::as_ref(ctx)
                    .chip_kinds()
                    .contains(&ContextChipKind::NodeVersion);
            let settings = SessionSettings::as_ref(ctx);
            in_prompt
                || settings
                    .agent_footer_chip_selection
                    .all_chips()
                    .contains(&ContextChipKind::NodeVersion)
                || settings
                    .cli_agent_footer_chip_selection
                    .all_chips()
                    .contains(&ContextChipKind::NodeVersion)
        };

        // `enable_ssh_warpification` is the single source of truth for whether the SSH
        // wrapper is active. The bootstrap scripts check `WARP_USE_SSH_WRAPPER` (derived
        // from this value) before invoking `warp_ssh_helper`, which spawns the ControlMaster
        // and opens agent-protocol channels.
        let enable_ssh_wrapper = *WarpifySettings::as_ref(ctx)
            .enable_ssh_warpification
            .value();

        // Only meaningful when the legacy ControlMaster wrapper is active.
        let reuse_ssh_control_master = enable_ssh_wrapper
            && *SshSettings::as_ref(ctx)
                .reuse_existing_control_master
                .value();

        let size: SizeInfo = model.lock().block_list().size().to_owned();
        let options = PtyOptions {
            size,
            window_id: None,
            shell_starter,
            start_dir: startup_directory,
            env_vars,
            enable_ssh_wrapper,
            reuse_ssh_control_master,
            shell_debug_mode: is_shell_debug_mode_enabled,
            honor_ps1: is_honor_ps1_enabled,
            node_version_chip_enabled,
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
}

/// Wires the Unix terminal-attributes poller to a terminal surface.
///
/// The poller is driven off `ModelEventDispatcher` block start/complete events
/// rather than surface-specific view events, and termios results are routed back
/// through the [`TerminalSurface`] Unix hooks so the manager stays surface-agnostic.
///
/// NOTE: we cannot simply use the strong references (the handle arguments to this
/// wire_up fn) in the subscription callbacks because that will create a reference
/// cycle. Instead, we use weak handles and upgrade them lazily.
#[cfg(unix)]
fn wire_up_terminal_attribute_poller_with_surface<S: TerminalSurface>(
    terminal_attributes_poller: &ModelHandle<TerminalAttributesPoller>,
    surface: &ViewHandle<S>,
    model_events: &ModelHandle<ModelEventDispatcher>,
    model: Arc<FairMutex<TerminalModel>>,
    ctx: &mut ModelContext<Box<dyn TerminalManagerTrait>>,
) where
    <S as Entity>::Event: PtyIntentEvent,
{
    let poller_weak_handle = terminal_attributes_poller.downgrade();
    let poller_weak_handle_for_termios = poller_weak_handle.clone();
    let surface_weak_handle = surface.downgrade();
    let surface_weak_handle_for_termios = surface_weak_handle.clone();

    // Tracks the index of the started block across the model-event <-> poller interactions.
    let block_index: Rc<RefCell<Option<BlockIndex>>> = Rc::new(RefCell::new(None));
    let block_index_for_termios = block_index.clone();

    // On block start, record the active block index and ask the surface whether
    // password-prompt polling is useful before starting the poller. On block
    // completion, stop polling and let the surface react to the completed block.
    ctx.subscribe_to_model(model_events, move |_manager, _dispatcher, event, ctx| {
        let Some(poller) = poller_weak_handle.upgrade(ctx) else {
            return;
        };

        match event {
            TerminalModelEvent::AfterBlockStarted {
                command,
                is_for_in_band_command: false,
                ..
            } => {
                let should_poll = surface_weak_handle.upgrade(ctx).is_some_and(|surface| {
                    surface.read(ctx, |surface, ctx| {
                        surface.should_start_password_prompt_polling(command, ctx)
                    })
                });
                if should_poll {
                    *block_index.borrow_mut() =
                        Some(model.lock().block_list().active_block_index());
                    poller.update(ctx, |poller, ctx| {
                        poller.start_polling(ctx);
                    });
                } else {
                    *block_index.borrow_mut() = None;
                }
            }
            TerminalModelEvent::AfterBlockCompleted(completed) => {
                if let Some(surface) = surface_weak_handle.upgrade(ctx) {
                    let should_stop = surface.read(ctx, |surface, _ctx| {
                        surface.should_stop_password_prompt_polling(completed)
                    });
                    if should_stop {
                        poller.update(ctx, |poller, _ctx| {
                            poller.stop_polling();
                        });
                        surface.update(ctx, |surface, ctx| {
                            surface.on_polled_block_completed(completed, ctx);
                        });
                    }
                }
            }
            _ => {}
        }
    });

    // When the poller detects termios consistent with a password prompt, route the
    // result through the surface and stop after the first detection to preserve
    // today's one-notification-per-command behavior.
    ctx.subscribe_to_model(
        terminal_attributes_poller,
        move |_manager, _poller, event, ctx| {
            let TerminalAttributesPollerEvent::TermiosQueryFinished { termios } = event;

            // A PTY likely has a password prompt if ECHO is disabled but ICANON is
            // still enabled. Apps like neovim disable both in raw mode, so requiring
            // ICANON avoids false positives.
            let might_be_password_prompt = !termios.local_flags.contains(LocalFlags::ECHO)
                && termios.local_flags.contains(LocalFlags::ICANON);
            if !might_be_password_prompt {
                return;
            }

            let Some(surface) = surface_weak_handle_for_termios.upgrade(ctx) else {
                return;
            };
            let block_index = block_index_for_termios.borrow_mut().take();
            surface.update(ctx, |surface, ctx| {
                surface.on_possible_password_prompt(block_index, ctx);
            });

            if let Some(poller) = poller_weak_handle_for_termios.upgrade(ctx) {
                poller.update(ctx, |poller, _ctx| {
                    poller.stop_polling();
                });
            }
        },
    );
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

impl EventLoopSender for mio_channel::Sender<Message> {
    fn send(&self, message: Message) -> Result<(), EventLoopSendError> {
        self.send(message).map_err(|error| match error {
            SendError(_) => EventLoopSendError::Disconnected,
        })
    }
}
