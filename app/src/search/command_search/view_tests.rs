use std::collections::HashSet;
use std::time::Duration;

use chrono::Utc;
use itertools::Itertools;
use warpui::App;
use warpui::r#async::Timer;
use warpui::platform::WindowStyle;

use super::*;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerNotebook, ServerPermissions};
use crate::network::NetworkStatus;
use crate::notebooks::{CloudNotebook, CloudNotebookModel};
use crate::search::data_source::Query;
use crate::server::cloud_objects::listener::Listener;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::SyncId;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::system::SystemStats;
use crate::terminal::History;
use crate::terminal::model::session::SessionId;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workflows::local_workflows::LocalWorkflows;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(Listener::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(LocalWorkflows::new);
    app.add_singleton_model(|_| History::default());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
}

#[test]
fn test_render_view() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            CommandSearchView::new(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
        });

        app.update(|_| {
            // This will force a redraw of the window, which lays out the
            // window, including the command search view.
        });
    });
}

/// Adds a notebook to `CloudModel` whose title is distinctive enough that a search for it
/// couldn't accidentally match anything else registered in these tests.
fn add_mock_notebook(app: &mut App, title: &str) {
    let metadata_ts = Utc::now().into();
    let server_notebook = ServerNotebook::new(
        SyncId::ServerId(123.into()),
        CloudNotebookModel {
            title: title.to_owned(),
            data: "unique notebook body content for this regression test".to_owned(),
            ai_document_id: None,
            conversation_id: None,
        },
        ServerMetadata {
            uid: 123.into(),
            revision: Revision::now(),
            metadata_last_updated_ts: metadata_ts,
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            current_editor_uid: None,
        },
        ServerPermissions {
            space: Owner::mock_current_user(),
            guests: Vec::new(),
            anyone_link_sharing: None,
            permissions_last_updated_ts: metadata_ts,
        },
    );
    let cloud_notebook = CloudNotebook::new_from_server(server_notebook);
    CloudModel::handle(app).update(app, |cloud_model, _| {
        cloud_model.add_object(cloud_notebook.id, cloud_notebook.clone());
    });
}

#[test]
fn test_command_search_does_not_offer_or_return_notebooks_when_drive_enabled() {
    // Regression test for notebooks removal from Command Search: this drives the real
    // `reset_command_search_mixer` production wiring (not a hand-registered fake source), with
    // Warp Drive enabled and a real matching notebook present, so a later reintroduction of the
    // notebooks source would fail here.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let notebook_title = "zzz-notebook-search-regression-proof";
        add_mock_notebook(&mut app, notebook_title);

        app.read(|app| {
            assert!(
                WarpDriveSettings::is_warp_drive_enabled(app),
                "this test only proves anything with Warp Drive enabled"
            );
        });

        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            CommandSearchView::new(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.reset_command_search_mixer(SessionId::from(0), None, None, None, ctx);
        });

        app.read(|app| {
            let registered_filters = view
                .as_ref(app)
                .mixer
                .as_ref(app)
                .registered_filters()
                .collect_vec();
            assert!(
                !registered_filters.contains(&QueryFilter::Notebooks),
                "Command Search must not register a source for QueryFilter::Notebooks, got: {registered_filters:?}"
            );
        });

        for filters in [HashSet::new(), HashSet::from([QueryFilter::Notebooks])] {
            view.update(&mut app, |view, ctx| {
                view.mixer.update(ctx, |mixer, ctx| {
                    mixer.run_query(
                        Query {
                            text: notebook_title.to_owned(),
                            filters,
                        },
                        ctx,
                    );
                });
            });

            Timer::after(Duration::from_millis(200)).await;

            app.read(|app| {
                let results = view.as_ref(app).mixer.as_ref(app).results();
                assert!(
                    !results
                        .iter()
                        .any(|result| result.accessibility_label().contains(notebook_title)),
                    "a query for the notebook's title must not return it from Command Search"
                );
            });
        }
    });
}
