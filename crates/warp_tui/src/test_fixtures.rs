//! Shared fixtures for `warp_tui` unit tests.
#![cfg_attr(not(test), allow(dead_code))]
use std::any::Any;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::settings::SettingsFileError;
use warp::tui_export::{
    AIConversationAutoexecuteMode, ActiveSession, Appearance, BlocklistAIActionModel,
    BlocklistAIHistoryModel, ConversationSelection, ConversationSelectionHandle,
    GetRelevantFilesController, ModelEventDispatcher, Sessions, TerminalManagerTrait,
    TerminalModel, TerminalSurfaceInit, TranscriptScope, TuiOnboardingMarkers, UserWorkspaces,
};
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warp_core::semantic_selection::SemanticSelection;
use warpui::{AddSingletonModel, App, EntityId, ModelHandle};
use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::{AppContext, Entity, TuiView, TypedActionView, ViewHandle, WindowId};

use crate::conversation_selection::TuiConversationSelection;
use crate::resume::TuiExitSummaryHandle;
use crate::terminal_session_view::TuiTerminalSessionView;
use crate::zero_state_animation::ZeroStateAnimationConfig;

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

pub(crate) fn add_test_conversation_selection(ctx: &mut AppContext) -> ConversationSelectionHandle {
    if !ctx.has_singleton_model::<AppExecutionMode>() {
        ctx.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    }
    if !ctx.has_singleton_model::<BlocklistAIHistoryModel>() {
        ctx.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    }
    let terminal_surface_id = EntityId::new();
    let mut terminal_model = TerminalModel::mock(None, None);
    terminal_model
        .block_list_mut()
        .set_transcript_scope(TranscriptScope::Unfiltered);
    let terminal_model = Arc::new(FairMutex::new(terminal_model));
    ctx.add_model(|ctx| {
        Box::new(TuiConversationSelection::new(
            terminal_surface_id,
            terminal_model,
            AIConversationAutoexecuteMode::RespectUserSettings,
            ctx,
        )) as Box<dyn ConversationSelection>
    })
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
    if !app.read(|ctx| ctx.has_singleton_model::<TuiOnboardingMarkers>()) {
        app.add_singleton_model(|_| TuiOnboardingMarkers::new_ready_for_test(false, false));
    }
    if !app.read(|ctx| ctx.has_singleton_model::<Appearance>()) {
        app.add_singleton_model(|_| Appearance::mock());
    }
    if !app.read(|ctx| ctx.has_singleton_model::<warp_core::telemetry::TelemetryContextModel>()) {
        app.update(warp_core::telemetry::testing::MockTelemetryContextProvider::register);
    }
    add_test_semantic_selection(app);
    // Read as a singleton by the action model's executors.
    if !app.read(|ctx| ctx.has_singleton_model::<BlocklistAIHistoryModel>()) {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    }
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
            UserWorkspaces::teamless_context_resolver_for_test(),
            ctx,
        )
    });
    (action_model, dispatcher)
}

/// Builds a full session view against mock terminal plumbing.
pub(crate) fn add_test_terminal_session(
    app: &mut App,
    window_id: WindowId,
) -> (
    ViewHandle<TuiTerminalSessionView>,
    ModelHandle<Box<dyn TerminalManagerTrait>>,
) {
    add_test_terminal_session_with_options(app, window_id, false, None)
}

pub(crate) fn add_test_terminal_session_with_first_run_onboarding(
    app: &mut App,
    window_id: WindowId,
) -> (
    ViewHandle<TuiTerminalSessionView>,
    ModelHandle<Box<dyn TerminalManagerTrait>>,
) {
    add_test_terminal_session_with_options(app, window_id, true, None)
}

pub(crate) fn add_test_terminal_session_with_settings_file_error(
    app: &mut App,
    window_id: WindowId,
    initial_settings_file_error: Option<SettingsFileError>,
) -> (
    ViewHandle<TuiTerminalSessionView>,
    ModelHandle<Box<dyn TerminalManagerTrait>>,
) {
    add_test_terminal_session_with_options(app, window_id, false, initial_settings_file_error)
}

fn add_test_terminal_session_with_options(
    app: &mut App,
    window_id: WindowId,
    handles_first_run_onboarding: bool,
    initial_settings_file_error: Option<SettingsFileError>,
) -> (
    ViewHandle<TuiTerminalSessionView>,
    ModelHandle<Box<dyn TerminalManagerTrait>>,
) {
    app.update(|ctx| {
        if !ctx.has_singleton_model::<ZeroStateAnimationConfig>() {
            ctx.add_singleton_model(|_| ZeroStateAnimationConfig::default());
        }
        let surface_init = TerminalSurfaceInit::new_for_test(ctx);
        let terminal_model = surface_init.model.clone();
        let view = ctx.add_typed_action_tui_view(window_id, |ctx| {
            TuiTerminalSessionView::new(
                surface_init,
                TuiExitSummaryHandle::default(),
                false,
                AIConversationAutoexecuteMode::RespectUserSettings,
                handles_first_run_onboarding,
                initial_settings_file_error,
                ctx,
            )
        });
        let manager = ctx.add_model(|_| {
            Box::new(TestTerminalManager(terminal_model)) as Box<dyn TerminalManagerTrait>
        });
        (view, manager)
    })
}
