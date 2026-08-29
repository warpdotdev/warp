use std::any::Any;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;

use parking_lot::FairMutex;
use pathfinder_geometry::vector::Vector2F;
use warp_errors::report_error;
use warpui::{AppContext, ModelHandle, SingletonEntity, ViewHandle, WindowId};

use crate::ai::blocklist::SerializedBlockListItem;
use crate::context_chips::prompt_type::PromptType;
use crate::pane_group::TerminalViewResources;
use crate::pane_group::pane::DetachType;
use crate::persistence::ModelEvent;
use crate::terminal::available_shells::AvailableShells;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::mio_channel;
use crate::terminal::model::session::Sessions;
use crate::terminal::model::terminal_model::ExitReason;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::shell::{ShellLaunchData, ShellName};
use crate::terminal::terminal_manager::BlockSpacing;
use crate::terminal::tmux::event_loop::{SharedControlState, TmuxControlSender};
use crate::terminal::tmux::gateway::spawn_control_client;
use crate::terminal::tmux::protocol::{
    fallback_supported_shell, pane_bootstrap_for_available_shell, pane_bootstrap_for_shell,
    schedule_kill_dedicated_server,
};
use crate::terminal::writeable_pty::pty_controller::EventLoopSender as _;
use crate::terminal::writeable_pty::terminal_manager_util::{
    init_pty_controller_model, wire_up_pty_controller_with_surface,
};
use crate::terminal::writeable_pty::{Message, PtyController, TerminalSurface};
use crate::terminal::{
    PTY_READS_BROADCAST_CHANNEL_SIZE, ShellLaunchState, TerminalManager as TerminalManagerTrait,
    TerminalModel, TerminalView, terminal_manager,
};

fn should_kill_dedicated_server(detach_type: DetachType) -> bool {
    matches!(detach_type, DetachType::Closed)
}

/// Owns the lifecycle of a Warp-managed tmux control-mode pane.
pub struct TmuxTerminalManager {
    model: Arc<FairMutex<TerminalModel>>,
    event_loop_tx: TmuxControlSender,
    event_loop_handle: Option<JoinHandle<()>>,
    socket: Option<std::path::PathBuf>,
    /// Kept alive so subscriptions between the surface and PTY writes are not dropped.
    #[allow(dead_code)]
    pty_controller: ModelHandle<PtyController<TmuxControlSender>>,
}

pub struct TmuxTerminalManagerInit {
    pub manager: ModelHandle<Box<dyn TerminalManagerTrait>>,
    pub view: ViewHandle<TerminalView>,
}

impl TmuxTerminalManager {
    pub fn create_model(
        resources: TerminalViewResources,
        initial_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        window_id: WindowId,
        ctx: &mut AppContext,
    ) -> TmuxTerminalManagerInit {
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, events_rx) = async_channel::unbounded();
        let (pty_reads_tx, pty_reads_rx) =
            async_broadcast::broadcast(PTY_READS_BROADCAST_CHANNEL_SIZE);
        let inactive_pty_reads_rx = pty_reads_rx.deactivate();
        let (executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let (event_loop_tx, event_loop_rx) = mio_channel::channel();

        let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);
        let sessions = ctx.add_model(|ctx| Sessions::new(executor_command_tx, ctx));
        let model_events =
            ctx.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));

        let preferred_shell = AvailableShells::handle(ctx)
            .read(ctx, |shells, ctx| shells.get_user_preferred_shell(ctx));
        let model = terminal_manager::create_terminal_model(
            None,
            None::<&Vec<SerializedBlockListItem>>,
            initial_size,
            channel_event_proxy.clone(),
            ShellLaunchState::DeterminingShell {
                available_shell: Some(preferred_shell.clone()),
                display_name: ShellName::LessDescriptive("tmux".to_owned()),
            },
            BlockSpacing::for_gui(ctx),
            ctx,
        );
        let colors = model.colors();
        let model = Arc::new(FairMutex::new(model));

        let prompt_type =
            ctx.add_model(|ctx| PromptType::new_dynamic_from_sessions(sessions.clone(), ctx));
        let cloned_model = model.clone();
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
                prompt_type,
                None,
                None,
                Some(inactive_pty_reads_rx),
                false,
                ctx,
            )
        });

        let shared = Arc::new(SharedControlState::new());
        let control_sender = TmuxControlSender::new(event_loop_tx.clone(), shared.clone());
        let pty_controller = init_pty_controller_model(
            control_sender.clone(),
            executor_command_rx,
            model_events,
            sessions.clone(),
            model.clone(),
            ctx,
        );
        wire_up_pty_controller_with_surface(
            &pty_controller,
            &view,
            model.clone(),
            sessions,
            model_event_sender,
            ctx,
        );

        let bootstrap = pane_bootstrap_for_available_shell(preferred_shell).or_else(|| {
            fallback_supported_shell()
                .map(|(path, shell_type)| pane_bootstrap_for_shell(path, shell_type))
        });

        let mut event_loop_handle = None;
        let mut socket = None;
        if let Some(bootstrap) = bootstrap {
            model.lock().register_session_id(bootstrap.session_id);
            model.lock().set_login_shell_spawned(bootstrap.shell_type);
            model
                .lock()
                .set_pending_shell_launch_data(ShellLaunchData::Executable {
                    executable_path: bootstrap.shell_path.clone(),
                    shell_type: bootstrap.shell_type,
                });
            match spawn_control_client(
                &bootstrap,
                model.clone(),
                channel_event_proxy,
                event_loop_rx,
                shared,
                ctx,
            ) {
                Ok(spawned) => {
                    event_loop_handle = Some(spawned.event_loop_handle);
                    socket = Some(spawned.socket);
                    view.update(ctx, |view, ctx| {
                        view.on_shell_determined(ctx);
                    });
                }
                Err(err) => {
                    report_error!(&err);
                    view.update(ctx, |view, ctx| {
                        view.on_pty_spawn_failed(err, ctx);
                    });
                    model.lock().exit(ExitReason::PtySpawnFailed);
                }
            }
        } else {
            let err = anyhow::anyhow!("Could not find a supported shell for the tmux pane");
            report_error!(&err);
            view.update(ctx, |view, ctx| {
                view.on_pty_spawn_failed(err, ctx);
            });
            model.lock().exit(ExitReason::ShellNotFound);
        }

        let terminal_view = view.clone();
        let manager = Self {
            model,
            event_loop_tx: control_sender,
            event_loop_handle,
            socket,
            pty_controller,
        };
        let manager_model = ctx.add_model(|_ctx| {
            let manager: Box<dyn TerminalManagerTrait> = Box::new(manager);
            manager
        });
        TmuxTerminalManagerInit {
            manager: manager_model,
            view: terminal_view,
        }
    }
}

impl Drop for TmuxTerminalManager {
    fn drop(&mut self) {
        let _ = self.event_loop_tx.send(Message::Shutdown);
        // Do not join the reader or wait on kill-server: either can stall the UI thread.
        let _ = self.event_loop_handle.take();
        if let Some(socket) = self.socket.take() {
            schedule_kill_dedicated_server(socket);
        }
    }
}

impl TerminalManagerTrait for TmuxTerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn on_view_detached(&self, detach_type: DetachType, _app: &mut AppContext) {
        if !should_kill_dedicated_server(detach_type) {
            return;
        }
        let _ = self.event_loop_tx.send(Message::Shutdown);
        if let Some(socket) = self.socket.clone() {
            schedule_kill_dedicated_server(socket);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
#[path = "terminal_manager_tests.rs"]
mod tests;
