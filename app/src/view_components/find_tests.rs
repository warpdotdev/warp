use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warpui::App;
use warpui::platform::WindowStyle;

use super::{Find, FindDirection, FindEvent, FindModel};
use crate::auth::AuthStateProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

struct MockFindModel;

impl warpui::Entity for MockFindModel {
    type Event = FindEvent;
}

impl FindModel for MockFindModel {
    fn focused_match_index(&self) -> Option<usize> {
        None
    }
    fn match_count(&self) -> usize {
        0
    }
    fn default_find_direction(&self, _app: &warpui::AppContext) -> FindDirection {
        FindDirection::Down
    }
}

fn initialize_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![],
            ctx,
        )
    });
}

#[test]
fn test_set_query_text_replaces_existing_text() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let model = app.add_model(|_| MockFindModel);
        let (_, find_view) =
            app.add_window(WindowStyle::NotStealFocus, |ctx| Find::new(model, ctx));

        find_view.update(&mut app, |view, ctx| {
            view.set_query_text("first", ctx);
        });
        let text = find_view.read(&app, |view, ctx| view.editor_text(ctx));
        assert_eq!(text, "first");

        find_view.update(&mut app, |view, ctx| {
            view.set_query_text("second", ctx);
        });
        let text = find_view.read(&app, |view, ctx| view.editor_text(ctx));
        assert_eq!(text, "second");
    })
}
