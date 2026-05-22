use std::sync::mpsc;

use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{AddSingletonModel, App, EntityId, ModelHandle, ViewContext, ViewHandle};

use super::NotebookManager;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::cloud_object::Owner;
use crate::network::NetworkStatus;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::notebook::NotebookView;
use crate::pane_group::NotebookPane;
use crate::persistence::ModelEvent;
use crate::search::files::model::FileSearchModel;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::shared_session::permissions_manager::SessionPermissionsManager;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::ActiveSession;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

struct TestState {
    manager: ModelHandle<NotebookManager>,
    model_events: mpsc::Receiver<ModelEvent>,
}

impl TestState {
    /// Add a notebook view, configured by `init`, and register it with the [`NotebookManager`].
    fn add_notebook<F>(&self, app: &mut App, init: F) -> ViewHandle<NotebookView>
    where
        F: FnOnce(&mut NotebookView, &mut ViewContext<NotebookView>),
    {
        let (window, notebook) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut view = NotebookView::new(ctx);
            init(&mut view, ctx);
            view
        });

        self.manager.update(app, |manager, ctx| {
            let pane = NotebookPane::new(notebook.clone(), ctx);
            manager.register_pane(&pane, EntityId::new(), window, ctx)
        });

        notebook
    }

    /// All model events not yet received.
    fn model_events(&self) -> Vec<ModelEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.model_events.try_recv() {
            events.push(event);
        }
        events
    }
}

fn initialize_app(app: &mut App) -> TestState {
    initialize_settings_for_tests(app);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(|_| UserProfiles::new(vec![]));
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_| repo_metadata::repositories::DetectedRepositories::default());
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(repo_metadata::RepoMetadataModel::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);

    let (sender, receiver) = mpsc::sync_channel(10);
    let objects_client = ServerApiProvider::new_for_test().get_cloud_objects_client();
    let sync_queue = app
        .add_singleton_model(|ctx| SyncQueue::new(Default::default(), objects_client.clone(), ctx));
    app.add_singleton_model(|ctx| UpdateManager::new(Some(sender), objects_client.clone(), ctx));
    sync_queue.update(app, |queue, ctx| queue.start_dequeueing(ctx));

    app.add_singleton_model(CloudViewModel::mock);
    let manager = app.add_singleton_model(NotebookManager::mock);
    TestState {
        manager,
        model_events: receiver,
    }
}

#[test]
fn test_save_on_close() {
    App::test((), |mut app| async move {
        let state = initialize_app(&mut app);
        let notebook = state.add_notebook(&mut app, |view, ctx| {
            view.open_new_notebook(
                Some("Test Notebook".to_string()),
                Owner::mock_current_user(),
                None,
                ctx,
            );
        });

        // Ensure the notebook has a pending edit.
        notebook.update(&mut app, |notebook, ctx| {
            notebook.input_editor().update(ctx, |editor, ctx| {
                editor.user_typed("Hello", ctx);
            });
        });

        // There will be an initial model event to save the notebook.
        let events = state.model_events();
        assert_eq!(events.len(), 1, "Expected 1 event, got {events:?}");

        // Closing the notebook manager should trigger a save.
        state
            .manager
            .update(&mut app, |manager, ctx| manager.close_notebooks(ctx));

        // There should now be a pending model event to save the notebook.
        let events = state.model_events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ModelEvent::UpsertNotebook { notebook } => {
                assert_eq!(notebook.model().title, "Test Notebook");
                assert_eq!(notebook.model().data, "Hello");
            }
            other => panic!("Expected an UpsertNotebook event, got {other:?}"),
        }
    });
}
