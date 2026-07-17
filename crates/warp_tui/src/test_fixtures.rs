//! Shared fixtures for `warp_tui` unit tests.
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    ActiveSession, BlocklistAIActionModel, BlocklistAIHistoryModel, GetRelevantFilesController,
    ModelEventDispatcher, Sessions, TerminalModel,
};
use warp_core::semantic_selection::SemanticSelection;
use warpui::{AddSingletonModel, App, EntityId, ModelHandle};
use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::{AppContext, Entity, TuiView, TypedActionView};

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
    (action_model, dispatcher)
}
