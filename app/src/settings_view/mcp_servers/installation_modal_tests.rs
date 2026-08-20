use uuid::Uuid;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{App, AppContext, Element, Entity, TypedActionView, View};

use super::*;
use crate::ai::mcp::{JsonTemplate, TemplatableMCPServer, TemplateVariable};
use crate::appearance::Appearance;

#[derive(Default)]
struct TestRoot;

impl Entity for TestRoot {
    type Event = ();
}

impl View for TestRoot {
    fn ui_name() -> &'static str {
        "TestRoot"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TestRoot {
    type Action = ();
}

#[test]
fn child_view_ids_covers_text_input_and_dropdown_variables() {
    // Regression test mirroring APP-5314: `InstallationModalBody` creates its
    // free-text `TextInput` editors via plain `ctx.add_view` (no structural
    // parent edge), and only renders them while an install is pending (see
    // `MCPServersSettingsPageView::get_modal_content`). On Cancel, the
    // pending server/inputs are left populated (only cleared on a completed
    // Install), so without `child_view_ids` a cross-window tab drag could
    // orphan these editors in the source window exactly like `AboutPageView`
    // did for `SettingsView`.
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| crate::server::server_api::ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| crate::auth::AuthStateProvider::new_for_test());
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(crate::cloud_object::model::persistence::CloudModel::mock);
        app.add_singleton_model(crate::workspaces::user_workspaces::UserWorkspaces::default_mock);
        app.add_singleton_model(crate::settings::PrivacySettings::mock);
        app.add_singleton_model(|_| crate::network::NetworkStatus::new());
        app.add_singleton_model(crate::workspaces::team_tester::TeamTesterStatus::mock);
        app.add_singleton_model(crate::server::sync_queue::SyncQueue::mock);
        app.add_singleton_model(crate::server::cloud_objects::update_manager::UpdateManager::mock);
        app.add_singleton_model(|_| {
            crate::settings_view::keybindings::KeybindingChangedNotifier::new()
        });
        app.add_singleton_model(|_| {
            crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier::new()
        });
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestRoot);

        let body = app.add_typed_action_view(window_id, InstallationModalBody::new);

        // One freetext variable (produces a `TextInput`) and one with
        // allowed values (produces a `Dropdown`), so the test covers both
        // `VariableInput` arms.
        let server = TemplatableMCPServer {
            uuid: Uuid::new_v4(),
            template: JsonTemplate {
                json: "{}".to_string(),
                variables: vec![
                    TemplateVariable {
                        key: "api_key".to_string(),
                        allowed_values: None,
                    },
                    TemplateVariable {
                        key: "region".to_string(),
                        allowed_values: Some(vec!["us".to_string(), "eu".to_string()]),
                    },
                ],
            },
            ..Default::default()
        };

        body.update(&mut app, |body, ctx| {
            body.set_templatable_mcp_server(Some(server), None, ctx);
        });

        let variable_input_ids: Vec<_> = body.read(&app, |body, _| {
            body.variable_inputs
                .values()
                .map(|input| match input {
                    VariableInput::TextInput(handle) => handle.id(),
                    VariableInput::Dropdown { handle, .. } => handle.id(),
                })
                .collect()
        });
        assert_eq!(
            variable_input_ids.len(),
            2,
            "sanity check: both variables should have produced an input widget"
        );

        let child_view_ids = body.read(&app, |body, ctx| body.child_view_ids(ctx));

        assert_eq!(
            child_view_ids.len(),
            variable_input_ids.len(),
            "child_view_ids must cover every variable input widget"
        );
        for id in variable_input_ids {
            assert!(
                child_view_ids.contains(&id),
                "child_view_ids is missing variable input {id:?}"
            );
        }
    });
}
