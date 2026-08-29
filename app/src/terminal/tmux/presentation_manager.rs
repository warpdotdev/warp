use std::any::Any;
use std::sync::Arc;

use parking_lot::FairMutex;
use pathfinder_geometry::vector::Vector2F;
use warpui::{AppContext, ModelHandle, ViewHandle, WindowId};

use crate::context_chips::prompt_type::PromptType;
use crate::pane_group::TerminalViewResources;
use crate::pane_group::pane::DetachType;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::session::Sessions;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::shell::{ShellName, ShellType};
use crate::terminal::terminal_manager::BlockSpacing;
use crate::terminal::{ShellLaunchState, TerminalManager, TerminalModel, TerminalView};

pub struct TmuxPresentationManager {
    model: Arc<FairMutex<TerminalModel>>,
}

pub struct TmuxPresentationManagerInit {
    pub manager: ModelHandle<Box<dyn TerminalManager>>,
    pub view: ViewHandle<TerminalView>,
}

impl TmuxPresentationManager {
    pub fn create_model(
        resources: TerminalViewResources,
        initial_size: Vector2F,
        window_id: WindowId,
        gateway_window: Option<WindowId>,
        instance_id: Option<u64>,
        ctx: &mut AppContext,
    ) -> TmuxPresentationManagerInit {
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, events_rx) = async_channel::unbounded();
        let (pty_reads_tx, _pty_reads_rx) = async_broadcast::broadcast(1);
        let (executor_command_tx, _executor_command_rx) = async_channel::unbounded();
        let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);

        let mut model = crate::terminal::terminal_manager::create_terminal_model(
            None,
            None,
            initial_size,
            channel_event_proxy,
            ShellLaunchState::ShellSpawned {
                available_shell: None,
                display_name: ShellName::LessDescriptive("tmux".to_owned()),
                shell_type: ShellType::Zsh,
            },
            BlockSpacing::for_gui(ctx),
            ctx,
        );
        model.set_tmux_presentation(true);
        model.set_tmux_control_mode(true);
        let runtime = match instance_id {
            Some(id) => crate::terminal::tmux::bridge::TmuxRuntime::for_id(
                crate::terminal::tmux::bridge::TmuxInstanceId::from_u64(id),
            ),
            None => gateway_window
                .and_then(crate::terminal::tmux::bridge::TmuxRuntime::for_gateway)
                .or_else(|| {
                    crate::terminal::tmux::bridge::TmuxRuntime::for_presentation(window_id)
                }),
        };
        if let Some(runtime) = runtime {
            runtime.bind_presentation(window_id);
            model.set_tmux_instance_id(Some(runtime.id().as_u64()));
            if let Some(shell_type) = runtime.shell_type() {
                model.set_login_shell_spawned(shell_type);
            }
            if let Some(session_id) = runtime.spawned_expected_session() {
                model.set_tmux_expected_session_id(Some(session_id));
                model.register_session_id(session_id);
            }
        }
        let colors = model.colors();
        let model = Arc::new(FairMutex::new(model));

        let sessions: ModelHandle<Sessions> =
            ctx.add_model(|ctx| Sessions::new(executor_command_tx, ctx));
        let model_events_dispatcher =
            ctx.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));
        let cloned_model = model.clone();
        let prompt_type =
            ctx.add_model(|ctx| PromptType::new_dynamic_from_sessions(sessions.clone(), ctx));
        let view = ctx.add_typed_action_view(window_id, |ctx| {
            let size_info = cloned_model.lock().block_list().size().to_owned();
            TerminalView::new(
                resources,
                wakeups_rx,
                model_events_dispatcher.clone(),
                cloned_model,
                sessions.clone(),
                size_info,
                colors,
                None,
                prompt_type,
                None,
                None,
                None,
                false,
                ctx,
            )
        });
        view.update(ctx, |_view, ctx| {
            ctx.spawn(futures::future::pending::<()>(), move |_, _, _| {
                std::mem::drop(model_events_dispatcher);
            });
        });

        let terminal_view = view;
        let manager_model = ctx.add_model(|_ctx| {
            let manager: Box<dyn TerminalManager> = Box::new(Self { model });
            manager
        });
        TmuxPresentationManagerInit {
            manager: manager_model,
            view: terminal_view,
        }
    }
}

impl TerminalManager for TmuxPresentationManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn on_view_detached(&self, detach_type: DetachType, _app: &mut AppContext) {
        let _ = detach_type;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
