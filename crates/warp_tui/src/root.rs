//! Transcript-only root surface for bare `warp-tui`.

use std::collections::HashMap;
use std::ffi::OsString;

use pathfinder_geometry::vector::Vector2F;
use warp::editor::CodeEditorModel;
use warp::tui_export::{
    ActiveSession, AfterBlockCompletedEvent, BannerState, BlockIndex, BlocklistAIActionModel,
    BlocklistAIContextModel, BlocklistAIController, BlocklistAIInputModel, ConversationSelection,
    GetRelevantFilesController, IsSharedSessionCreator, LocalTtyTerminalManager, PtyIntent,
    PtyIntentEvent, ShellLaunchData, TerminalManagerTrait, TerminalSurface, TerminalSurfaceInit,
    TerminalSurfaceResult,
};
use warpui::platform::{TerminationMode, WindowStyle};
use warpui::{
    AddWindowOptions, AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TuiView,
    TypedActionView, ViewContext, ViewHandle,
};
use warpui_core::elements::tui::{TuiChildView, TuiColumn, TuiElement, TuiParentElement, TuiText};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};

use super::conversation_model::TuiConversationModel;
use super::conversation_selection::TuiConversationSelection;
use super::input::{TuiInputView, TuiInputViewEvent};
use super::transcript_view::TuiTranscriptView;

/// Char-cell width for the TUI prompt editor. Matches the surface's default
/// columns; the editor wraps at this width independent of live terminal resize.
const INPUT_WIDTH: u16 = 120;

/// This surface emits no PTY intents; commands are driven only by the shell
/// the terminal manager spawns, not by the transcript UI.
enum RootEvent {}

impl PtyIntentEvent for RootEvent {
    fn pty_intent(&self) -> Option<PtyIntent> {
        match *self {}
    }
}

struct RootTuiView {
    transcript: ViewHandle<TuiTranscriptView>,
    input_view: ViewHandle<TuiInputView>,
    conversation_model: ModelHandle<TuiConversationModel>,
}

impl RootTuiView {
    fn new(surface_init: TerminalSurfaceInit, ctx: &mut ViewContext<Self>) -> Self {
        let TerminalSurfaceInit {
            model,
            sessions,
            model_events,
            wakeups_rx,
            ..
        } = surface_init;
        let terminal_surface_id: EntityId = ctx.view_id();
        let active_session =
            ctx.add_model(|ctx| ActiveSession::new(sessions.clone(), model_events.clone(), ctx));
        let conversation_selection = ctx.add_model(|ctx| {
            Box::new(TuiConversationSelection::new(terminal_surface_id, ctx))
                as Box<dyn ConversationSelection>
        });
        let context_model = ctx.add_model(|ctx| {
            BlocklistAIContextModel::new(
                sessions,
                &model_events,
                model.clone(),
                terminal_surface_id,
                conversation_selection.clone(),
                ctx,
            )
        });
        let input_model = ctx.add_model(|ctx| {
            BlocklistAIInputModel::new(
                model.clone(),
                conversation_selection.clone(),
                context_model.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        let get_relevant_files_controller = ctx.add_model(GetRelevantFilesController::new);
        let action_model = ctx.add_model(|ctx| {
            BlocklistAIActionModel::new(
                model.clone(),
                active_session.clone(),
                &model_events,
                get_relevant_files_controller,
                terminal_surface_id,
                ctx,
            )
        });
        let ai_controller = ctx.add_model(|ctx| {
            BlocklistAIController::new(
                input_model,
                context_model,
                conversation_selection.clone(),
                action_model,
                active_session,
                model.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        let conversation_model = ctx.add_model(|ctx| {
            TuiConversationModel::new(
                terminal_surface_id,
                conversation_selection,
                ai_controller,
                ctx,
            )
        });
        let transcript = ctx.add_typed_action_tui_view(|ctx| {
            TuiTranscriptView::new(terminal_surface_id, model.clone(), ctx)
        });
        let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(INPUT_WIDTH, ctx));
        let input_view =
            ctx.add_typed_action_tui_view(move |ctx| TuiInputView::new(input_model, ctx));
        ctx.subscribe_to_view(&input_view, Self::handle_submitted_prompt);

        ctx.subscribe_to_model(&model_events, |_, _, _, ctx| ctx.notify());
        ctx.spawn_stream_local(wakeups_rx, |_, _, ctx| ctx.notify(), |_, _| {});
        ctx.focus_self();

        Self {
            transcript,
            input_view,
            conversation_model,
        }
    }

    /// Routes a prompt submitted from the input editor to this surface's
    /// conversation, streaming the response into the transcript as an agent block.
    fn handle_submitted_prompt(
        &mut self,
        _input_view: ViewHandle<TuiInputView>,
        event: &TuiInputViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiInputViewEvent::Submitted(prompt) => {
                let prompt = prompt.trim().to_owned();
                if prompt.is_empty() {
                    return;
                }
                self.conversation_model
                    .update(ctx, |model, ctx| model.send_prompt(prompt, ctx));
                ctx.notify();
            }
        }
    }
}

impl Entity for RootTuiView {
    type Event = RootEvent;
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        vec![self.input_view.id(), self.transcript.id()]
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        // The input is laid out first at its content height; the transcript is
        // last so it greedily fills the remaining rows (it owns scrolling).
        Box::new(
            TuiColumn::new()
                .with_child(Box::new(TuiChildView::new(&self.input_view)))
                .with_child(Box::new(
                    TuiText::new("──── transcript · wheel to scroll · Ctrl-C quits ────")
                        .truncate(),
                ))
                .with_child(Box::new(TuiChildView::new(&self.transcript))),
        )
    }
}

impl TypedActionView for RootTuiView {
    type Action = ();
}

impl TerminalSurface for RootTuiView {
    #[cfg(unix)]
    fn should_start_password_prompt_polling(&self, _command: &str, _ctx: &AppContext) -> bool {
        false
    }

    #[cfg(unix)]
    fn should_stop_password_prompt_polling(&self, _completed: &AfterBlockCompletedEvent) -> bool {
        false
    }

    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn on_active_shell_launch_data_updated(
        &mut self,
        _shell_launch_data: Option<ShellLaunchData>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>) {
        log::error!("TUI PTY spawn failed: {error:#}");
        ctx.notify();
    }

    #[cfg(unix)]
    fn on_possible_password_prompt(
        &mut self,
        _block_index: Option<BlockIndex>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(unix)]
    fn on_polled_block_completed(
        &mut self,
        _completed: &AfterBlockCompletedEvent,
        _ctx: &mut ViewContext<Self>,
    ) {
    }
}

struct TuiSession {
    _driver: TuiDriverHandle,
    _manager: ModelHandle<Box<dyn TerminalManagerTrait>>,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Starts the transcript-only root TUI session.
pub(super) fn start(ctx: &mut AppContext) {
    let banner = ctx.add_model(|_| BannerState::default());
    let manager = LocalTtyTerminalManager::<RootTuiView>::create_tui_model(
        std::env::current_dir().ok(),
        HashMap::<OsString, OsString>::from_iter(std::env::vars_os()),
        IsSharedSessionCreator::No,
        None,
        banner,
        Vector2F::new(120., 24.),
        None,
        None,
        ctx,
        |surface_init, ctx| {
            let (_, surface) = ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| RootTuiView::new(surface_init, ctx),
            );
            TerminalSurfaceResult {
                surface,
                post_wire: |_manager: &mut LocalTtyTerminalManager<RootTuiView>,
                            _surface: &ViewHandle<RootTuiView>,
                            _ctx: &mut AppContext| {},
            }
        },
    );
    let window_id = manager.surface.window_id(ctx);
    match spawn_tui_driver(ctx, window_id, manager.surface) {
        Ok(driver) => {
            ctx.add_singleton_model(|_| TuiSession {
                _driver: driver,
                _manager: manager.manager,
            });
        }
        Err(error) => {
            log::error!("failed to start transcript TUI: {error}");
            ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error.into())));
        }
    }
}
