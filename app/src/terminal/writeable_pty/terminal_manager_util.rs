use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use async_channel::Receiver;
use parking_lot::FairMutex;
use warpui::{AppContext, Entity, ModelHandle, ViewHandle};

use crate::persistence::ModelEvent;
use crate::terminal::line_editor_status::LineEditorStatus;
use crate::terminal::model::session::{ExecutorCommandEvent, Sessions};
use crate::terminal::model::terminal_model::ExitReason;
use crate::terminal::writeable_pty::command_history::update_command_history;
use crate::terminal::writeable_pty::pty_controller::EventLoopSender;
use crate::terminal::writeable_pty::{
    PtyController, PtyControllerEvent, PtyIntent, PtyIntentEvent, TerminalSurface,
};
#[cfg(not(target_family = "wasm"))]
use crate::terminal::{view, TerminalView};
use crate::terminal::{ModelEventDispatcher, TerminalModel};

/// Wires up bi-directional communication between the PtyController and a terminal surface.
/// Note that this interaction can't live in the surface itself because the surface must be
/// manager-agnostic.
///
/// NOTE: we cannot simply use the strong references (the handle arguments to this wire_up fn)
/// in the subscription callbacks because that will create a reference cycle. Instead,
/// we should use weak handles and upgrade them lazily.
pub fn wire_up_pty_controller_with_surface<T: EventLoopSender, S: TerminalSurface>(
    pty_controller: &ModelHandle<PtyController<T>>,
    surface: &ViewHandle<S>,
    model: Arc<FairMutex<TerminalModel>>,
    sessions: ModelHandle<Sessions>,
    model_event_sender: Option<SyncSender<ModelEvent>>,
    ctx: &mut AppContext,
) where
    <S as Entity>::Event: PtyIntentEvent,
{
    let controller_weak_handle = pty_controller.downgrade();
    let surface_weak_handle = surface.downgrade();
    let model_clone = model.clone();

    ctx.subscribe_to_view(surface, move |_surface, event, ctx| {
        // NOTE: we cannot simply use the strong reference (the model handle argument to this wire_up fn)
        // because it'll create a reference cycle with the `subscribe_to_model` usage below. Instead,
        // we should use a weak handle and upgrade it lazily.
        let Some(controller) = controller_weak_handle.upgrade(ctx) else {
            return;
        };

        let Some(intent) = event.pty_intent() else {
            return;
        };

        match intent {
            PtyIntent::CtrlD => {
                controller.update(ctx, |controller, ctx| {
                    controller.write_end_of_transmission_char(ctx);
                });
            }
            PtyIntent::ShutdownPty => {
                controller.update(ctx, |controller, ctx| {
                    controller.shutdown_pty(ctx);
                });
            }
            PtyIntent::WriteBytes(bytes) => {
                controller.update(ctx, |controller, ctx| {
                    // TODO: the underlying bytes should be wrapped in an Arc and copied out only when they need to be written to the PTY.
                    controller.write_bytes(bytes, ctx);
                });
            }
            PtyIntent::WriteAgentInput { bytes, mode } => {
                controller.update(ctx, |controller, ctx| {
                    controller.write_agent_bytes(bytes, &mode, ctx);
                });
            }
            PtyIntent::Resize(size_update) => {
                controller.update(ctx, |controller, ctx| {
                    controller.resize_pty(size_update, ctx);
                });
            }
            PtyIntent::ExecuteCommand(event) => {
                let Some(shell_type) = sessions
                    .as_ref(ctx)
                    .get(event.session_id)
                    .map(|s| s.shell().shell_type())
                else {
                    log::warn!("Failed to get shell type for the session associated to the command execution event.");
                    return;
                };

                model_clone.lock().block_list_mut().active_block_mut().set_cloud_workflow_state(event.workflow_id);
                controller.update(ctx, |controller, ctx| {
                    controller.write_command(&event.command, shell_type, event.source.clone(), ctx)
                });

                if event.should_add_command_to_history {
                    update_command_history(
                        &event,
                        &model_clone,
                        model_event_sender.as_ref(),
                        &sessions,
                        ctx,
                    );
                }
            }
            PtyIntent::RunNativeShellCompletions {
                buffer_text,
                results_tx,
            } => {
                controller.update(ctx, |controller, ctx| {
                    controller.run_native_shell_completions(buffer_text, results_tx, ctx);
                });
            }
        }
    });

    ctx.subscribe_to_model(pty_controller, move |_pty_controller, event, ctx| {
        let Some(surface) = surface_weak_handle.upgrade(ctx) else {
            return;
        };

        match event {
            PtyControllerEvent::PtyDisconnected => surface.update(ctx, |_, _| {
                model.lock().exit(ExitReason::PtyDisconnected);
            }),
        }
    });
}

/// Wires up bi-directional communication between the RemoteServerController and the TerminalView.
/// This handles the flow to render the remote server block and forward the user's choice to the controller.
///
/// Note that this interaction can't live in the TerminalView because the view must be manager-agnostic.
///
/// NOTE: we cannot simply use the strong references (the handle arguments to this wire_up fn)
/// in the subscription callbacks because that will create a reference cycle. Instead,
/// we should use weak handles and upgrade them lazily.
#[cfg(not(target_family = "wasm"))]
pub fn wire_up_remote_server_controller_with_view<T: EventLoopSender>(
    remote_server_controller: &ModelHandle<
        super::remote_server_controller::RemoteServerController<T>,
    >,
    terminal_view: &ViewHandle<TerminalView>,
    ctx: &mut AppContext,
) {
    let controller_weak = remote_server_controller.downgrade();
    ctx.subscribe_to_view(terminal_view, move |_view, event, ctx| {
        let Some(controller) = controller_weak.upgrade(ctx) else {
            return;
        };
        match event {
            view::Event::RemoteServerInstallRequested { session_id } => {
                controller.update(ctx, |ctrl, ctx| {
                    ctrl.handle_ssh_remote_server_install(*session_id, ctx);
                });
            }
            view::Event::RemoteServerSkipRequested { session_id } => {
                controller.update(ctx, |ctrl, ctx| {
                    ctrl.handle_ssh_remote_server_skip(*session_id, ctx);
                });
            }
            _ => {}
        }
    });
}

/// Creates a PtyController to broker writes to the PTY and registers it as a model.
pub fn init_pty_controller_model<Sender: EventLoopSender>(
    event_loop_tx: Sender,
    executor_command_rx: Receiver<ExecutorCommandEvent>,
    model_events: ModelHandle<ModelEventDispatcher>,
    sessions: ModelHandle<Sessions>,
    model: Arc<FairMutex<TerminalModel>>,
    ctx: &mut AppContext,
) -> ModelHandle<PtyController<Sender>> {
    let line_editor_status =
        ctx.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));

    ctx.add_model(|ctx| {
        PtyController::new(
            event_loop_tx,
            model_events,
            line_editor_status,
            sessions,
            executor_command_rx,
            model,
            ctx,
        )
    })
}

/// Creates a [`RemoteServerController`] that orchestrates the SSH init flow.
#[cfg(not(target_family = "wasm"))]
pub fn init_remote_server_controller<Sender: EventLoopSender>(
    pty_controller: &ModelHandle<PtyController<Sender>>,
    model_events: &ModelHandle<ModelEventDispatcher>,
    ctx: &mut AppContext,
) -> ModelHandle<super::remote_server_controller::RemoteServerController<Sender>> {
    let pty_weak = pty_controller.downgrade();
    ctx.add_model(|ctx| {
        super::remote_server_controller::RemoteServerController::new(
            pty_weak,
            model_events.clone(),
            ctx,
        )
    })
}
