//! Shared fixtures for `warp_tui` unit tests.
use std::any::Any;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    ActiveSession, BlocklistAIActionModel, BlocklistAIHistoryModel, GetRelevantFilesController,
    ModelEventDispatcher, Sessions, TerminalManagerTrait, TerminalModel, TerminalSurfaceInit,
};
use warp_core::semantic_selection::SemanticSelection;
use warpui::{AddSingletonModel, App, EntityId, ModelHandle, SingletonEntity as _};
use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::{AppContext, Entity, TuiView, TypedActionView, ViewHandle, WindowId};

use crate::resume::TuiExitSummaryHandle;
use crate::terminal_session_view::TuiTerminalSessionView;

struct TestTerminalManager(Arc<FairMutex<TerminalModel>>);

impl TerminalManagerTrait for TestTerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.0.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A trivial typed-action root view for tests that need a TUI window whose
/// real subject is a non-root child view.
pub(crate) struct TestHostView;

impl Entity for TestHostView {
    type Event = ();
}

impl TuiView for TestHostView {
    fn ui_name() -> &'static str {
        "TestHostView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TuiText::new(""))
    }
}

impl TypedActionView for TestHostView {
    type Action = ();
}
/// Registers semantic-selection settings shared by selectable TUI test views.
pub(crate) fn add_test_semantic_selection(ctx: &mut impl AddSingletonModel) {
    ctx.add_singleton_model(|_| SemanticSelection::mock(true, ""));
}

/// Builds the action model injected into stateful TUI tool-call views.
pub(crate) fn add_test_action_model(app: &mut App) -> ModelHandle<BlocklistAIActionModel> {
    add_test_action_model_and_events(app).0
}

/// Builds the action model and terminal-event dispatcher injected into TUI agent blocks.
pub(crate) fn add_test_action_model_and_events(
    app: &mut App,
) -> (
    ModelHandle<BlocklistAIActionModel>,
    ModelHandle<ModelEventDispatcher>,
) {
    let (action_model, dispatcher, _) = add_test_action_model_with_surface(app);
    (action_model, dispatcher)
}

/// [`add_test_action_model_and_events`] plus the terminal-surface id the
/// action model was built with, so tests can register an active conversation
/// for that surface in the history model (required by
/// `BlocklistAIActionModel::get_pending_action`).
pub(crate) fn add_test_action_model_with_surface(
    app: &mut App,
) -> (
    ModelHandle<BlocklistAIActionModel>,
    ModelHandle<ModelEventDispatcher>,
    EntityId,
) {
    add_test_semantic_selection(app);
    // Read as a singleton by the action model's executors.
    app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_tx, model_events_rx) = async_channel::unbounded();
    let dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session =
        app.add_model(|ctx| ActiveSession::new(sessions.clone(), dispatcher.clone(), ctx));
    // `GetRelevantFilesController::new` subscribes to the `CodebaseIndexManager`
    // singleton, which these tests don't register; `default` skips it.
    let get_relevant_files = app.add_model(|_| GetRelevantFilesController::default());
    let terminal_surface_id = EntityId::new();
    let action_model = app.add_model(|ctx| {
        BlocklistAIActionModel::new(
            terminal_model,
            active_session,
            &dispatcher,
            get_relevant_files,
            terminal_surface_id,
            ctx,
        )
    });
    (action_model, dispatcher, terminal_surface_id)
}

/// Builds a full session view against mock terminal plumbing.
pub(crate) fn add_test_terminal_session(
    app: &mut App,
    window_id: WindowId,
) -> (
    ViewHandle<TuiTerminalSessionView>,
    ModelHandle<Box<dyn TerminalManagerTrait>>,
) {
    app.update(|ctx| {
        let surface_init = TerminalSurfaceInit::new_for_test(ctx);
        let terminal_model = surface_init.model.clone();
        let view = ctx.add_typed_action_tui_view(window_id, |ctx| {
            TuiTerminalSessionView::new(surface_init, TuiExitSummaryHandle::default(), false, ctx)
        });
        let manager = ctx.add_model(|_| {
            Box::new(TestTerminalManager(terminal_model)) as Box<dyn TerminalManagerTrait>
        });
        (view, manager)
    })
}

/// Creates a live, active conversation for `terminal_surface_id` in the
/// history model, returning its id. Combined with
/// `BlocklistAIActionModel::queue_pending_action_for_test`, this drives an
/// action into `Blocked` status for confirmation-flow tests.
#[allow(dead_code)]
pub(crate) fn add_active_test_conversation(
    app: &mut App,
    terminal_surface_id: EntityId,
) -> warp::tui_export::AIConversationId {
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(terminal_surface_id, false, false, false, ctx);
            history.set_active_conversation_id(conversation_id, terminal_surface_id, ctx);
            conversation_id
        })
    })
}
