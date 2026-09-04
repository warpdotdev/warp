use warp_core::ui::appearance::Appearance;
use warp_server_client::auth::AgentIdentity;
use warpui::App;
use warpui::TypedActionView;
use warpui::platform::WindowStyle;

use super::CreateApiKeyModal;
use crate::auth::AuthStateProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn agent(uid: &str, name: &str, available: bool) -> AgentIdentity {
    AgentIdentity {
        uid: uid.to_string(),
        name: name.to_string(),
        available,
    }
}

/// Regression test for the searchable Agent picker in the New API key modal:
/// the agent dropdown is a `FilterableDropdown`, lists only available agents,
/// and filtering by a query narrows the visible list case-insensitively.
#[test]
fn test_agent_dropdown_is_searchable() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SyncedInputState::mock());
        app.add_singleton_model(|_| VimRegisters::new());
        app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
        app.add_singleton_model(UserWorkspaces::default_mock);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, CreateApiKeyModal::new);

        // Populate the picker with several agents; the unavailable one should be
        // excluded from the list entirely.
        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(
                vec![
                    agent("1", "Default Service Account", true),
                    agent("2", "Ben's Agent", true),
                    agent("3", "Server Migration Agent", true),
                    agent("4", "Unavailable Agent", false),
                ],
                ctx,
            );
        });

        // Only the 3 available agents are listed, and all are visible with no filter.
        let total = view.read(&app, |modal, ctx| modal.agent_dropdown.as_ref(ctx).len());
        assert_eq!(total, 3, "only available agents should be listed");
        let all_visible = view.read(&app, |modal, ctx| {
            modal
                .agent_dropdown
                .as_ref(ctx)
                .visible_items_len_for_test(ctx)
        });
        assert_eq!(all_visible, 3);

        // Typing a query filters the list case-insensitively.
        view.update(&mut app, |modal, ctx| {
            modal.agent_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_filter_query_for_test("BEN", ctx)
            });
        });
        let filtered = view.read(&app, |modal, ctx| {
            modal
                .agent_dropdown
                .as_ref(ctx)
                .visible_items_len_for_test(ctx)
        });
        assert_eq!(filtered, 1, "query should match only \"Ben's Agent\"");

        // A non-matching query yields no matches.
        view.update(&mut app, |modal, ctx| {
            modal.agent_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_filter_query_for_test("zzz", ctx)
            });
        });
        let none = view.read(&app, |modal, ctx| {
            modal
                .agent_dropdown
                .as_ref(ctx)
                .visible_items_len_for_test(ctx)
        });
        assert_eq!(none, 0);

        // Clearing the query restores the full list.
        view.update(&mut app, |modal, ctx| {
            modal.agent_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_filter_query_for_test("", ctx)
            });
        });
        let restored = view.read(&app, |modal, ctx| {
            modal
                .agent_dropdown
                .as_ref(ctx)
                .visible_items_len_for_test(ctx)
        });
        assert_eq!(restored, 3);
    })
}

/// Regression test for APP-5819: once agents load, the default shown agent
/// (Default Service Account, or the first available agent otherwise) must be
/// stored in `selected_agent_uid` without requiring the user to click the
/// dropdown, since that's what gates the Create key button for Type=Agent.
#[test]
fn test_agent_dropdown_auto_selects_default_agent() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SyncedInputState::mock());
        app.add_singleton_model(|_| VimRegisters::new());
        app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
        app.add_singleton_model(UserWorkspaces::default_mock);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, CreateApiKeyModal::new);

        assert!(
            view.read(&app, |modal, _| modal.selected_agent_uid.clone())
                .is_none(),
            "no agent should be selected before agents load"
        );

        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(
                vec![
                    agent("1", "Default Service Account", true),
                    agent("2", "Ben's Agent", true),
                    agent("3", "Unavailable Agent", false),
                ],
                ctx,
            );
        });

        let selected = view.read(&app, |modal, _| modal.selected_agent_uid.clone());
        assert_eq!(
            selected,
            Some("1".to_string()),
            "Default Service Account should be auto-selected once agents load"
        );

        // Re-selecting the default (as if the user picked it again) should keep it selected.
        view.update(&mut app, |modal, ctx| {
            modal.handle_action(
                &super::CreateApiKeyModalAction::SelectAgent("1".to_string()),
                ctx,
            );
        });
        let selected_again = view.read(&app, |modal, _| modal.selected_agent_uid.clone());
        assert_eq!(selected_again, Some("1".to_string()));
    })
}

/// When no agent is named "Default Service Account", the first available
/// agent should be auto-selected instead.
#[test]
fn test_agent_dropdown_auto_selects_first_agent_without_default_service_account() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SyncedInputState::mock());
        app.add_singleton_model(|_| VimRegisters::new());
        app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
        app.add_singleton_model(UserWorkspaces::default_mock);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, CreateApiKeyModal::new);

        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(
                vec![
                    agent("2", "Ben's Agent", true),
                    agent("3", "Server Migration Agent", true),
                ],
                ctx,
            );
        });

        let selected = view.read(&app, |modal, _| modal.selected_agent_uid.clone());
        assert_eq!(selected, Some("2".to_string()));
    })
}
