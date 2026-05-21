use std::sync::Arc;
use warp_core::ui::appearance::Appearance;
use warpui::{
    elements::Empty, platform::WindowStyle, App, AppContext, Element, Entity, TypedActionView,
    View, ViewContext,
};

use crate::{
    ai::blocklist::BlocklistAIHistoryModel,
    auth::AuthStateProvider,
    cloud_object::model::persistence::CloudModel,
    menu::MenuItemFields,
    pane_group::{focus_state::PaneFocusHandle, BackingView, PaneConfiguration, PaneId, PaneView},
    server::server_api::{object::MockObjectClient, ServerApiProvider},
    settings_view::keybindings::KeybindingChangedNotifier,
    terminal::shared_session::permissions_manager::SessionPermissionsManager,
    test_util::settings::initialize_settings_for_tests,
    NetworkStatus, SyncQueue, TeamTesterStatus, UpdateManager, UserProfiles, UserWorkspaces,
};

use super::{Event, OpenOverlay};

#[cfg(test)]
use crate::server::server_api::workspace::MockWorkspaceClient;

#[cfg(test)]
use crate::server::server_api::team::MockTeamClient;

/// A dummy view that is also a backing pane view for testing purposes.
struct TestView {
    counter: usize,
    close_invoked: bool,
}

#[derive(Clone, Debug)]
enum TestViewAction {
    IncrementCounter,
}

impl TestView {
    fn new() -> Self {
        Self {
            counter: 0,
            close_invoked: false,
        }
    }
}

impl Entity for TestView {
    type Event = ();
}

impl View for TestView {
    fn ui_name() -> &'static str {
        "TestView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TestView {
    type Action = TestViewAction;
    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TestViewAction::IncrementCounter => {
                self.counter += 1;
                ctx.notify();
            }
        }
    }
}

impl BackingView for TestView {
    type PaneHeaderOverflowMenuAction = TestViewAction;
    type CustomAction = TestViewAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn handle_custom_action(&mut self, action: &Self::CustomAction, ctx: &mut ViewContext<Self>) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.close_invoked = true;
        ctx.notify();
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render_header_content(
        &self,
        _ctx: &super::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> super::HeaderContent {
        super::HeaderContent::simple("Test")
    }

    fn set_focus_handle(&mut self, _focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {}
}

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| NetworkStatus::new());
    let mock_team_client = Arc::new(MockTeamClient::new());
    let mock_workspace_client = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            mock_team_client.clone(),
            mock_workspace_client.clone(),
            vec![],
            ctx,
        )
    });
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|ctx| UpdateManager::new(None, Arc::new(MockObjectClient::new()), ctx));
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
}

#[test]
fn test_overflow_menu_items() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, pane_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let test_view = ctx.add_typed_action_view(|_| TestView::new());
            let pane_config = ctx.add_model(|_| PaneConfiguration::new("Test"));

            PaneView::new(PaneId::dummy_pane_id(), test_view, (), pane_config, ctx)
        });

        let header = pane_view.read(&app, |pane, _ctx| pane.header().to_owned());
        let overflow_menu = header.read(&app, |header, _ctx| header.overflow_menu.to_owned());

        let menu_item_label = "Increment counter";
        let menu_items = vec![MenuItemFields::new(menu_item_label)
            .with_on_select_action(TestViewAction::IncrementCounter)
            .into_item()];

        // Set the menu items and open the menu.
        header.update(&mut app, |header, ctx| {
            header.set_overflow_menu_items(menu_items, ctx);
            header.open_overlay = OpenOverlay::OverflowMenu;
            ctx.notify();
        });

        // Mimic what happens when clicking the item.
        overflow_menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_name(menu_item_label, ctx);
            menu.mimic_confirm(ctx);
        });

        pane_view.read(&app, |view, ctx| {
            assert_eq!(view.child(ctx).as_ref(ctx).counter, 1);
        });
    })
}

#[test]
fn test_handle_close() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, pane_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let test_view = ctx.add_view(|_| TestView::new());
            let pane_config = ctx.add_model(|_| PaneConfiguration::new("Test"));
            PaneView::new(PaneId::dummy_pane_id(), test_view, (), pane_config, ctx)
        });

        pane_view.update(&mut app, |pane_view, ctx| {
            pane_view.header().update(ctx, |_header, ctx| {
                // Mimic clicking the close button.
                ctx.emit(Event::Close);
            })
        });

        pane_view.read(&app, |view, ctx| {
            assert!(view.child(ctx).as_ref(ctx).close_invoked);
        });
    })
}
