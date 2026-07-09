//! Shared fixtures for `warp_tui` unit tests.
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    ActiveSession, BlocklistAIActionModel, BlocklistAIHistoryModel, GetRelevantFilesController,
    ModelEventDispatcher, Sessions, TerminalModel,
};
use warpui::{App, EntityId, ModelHandle};
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

/// Builds the action model and terminal-event dispatcher injected into TUI agent blocks.
pub(crate) fn add_test_action_model_and_events(
    app: &mut App,
) -> (
    ModelHandle<BlocklistAIActionModel>,
    ModelHandle<ModelEventDispatcher>,
) {
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
    let action_model = app.add_model(|ctx| {
        BlocklistAIActionModel::new(
            terminal_model,
            active_session,
            &dispatcher,
            get_relevant_files,
            EntityId::new(),
            ctx,
        )
    });
    (action_model, dispatcher)
}
