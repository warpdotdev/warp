use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, sync_channel};

use cloud_object_client::{MockObjectClient, ObjectClient};
use settings::manager::SettingsManager;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::{App, ModelHandle, SingletonEntity};

use super::update_manager::UpdateManager;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::network::NetworkStatus;
use crate::persistence::ModelEvent;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::{PrivacySettings, WarpDrivePrivacySettings};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// The size of the bounded channel that we use to queue persistence/sqlite-related events.
const CHANNEL_SIZE: usize = 128;

pub struct UpdateManagerStruct {
    pub update_manager: ModelHandle<UpdateManager>,
    pub receiver: Receiver<ModelEvent>,
    pub cloud_model_events: async_channel::Receiver<CloudModelEvent>,
}

pub fn initialize_app(app: &mut App) {
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));

    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SettingsManager::default());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.update(crate::settings::init_and_register_user_preferences);
    // This ServerApiProvider is used for the PrivacySettings model, but not the UpdateManager
    // under test.
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    WarpDrivePrivacySettings::register(app);
    app.update(PrivacySettings::register_singleton);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
}

pub fn create_update_manager_struct(
    app: &mut App,
    server_api: Arc<dyn ObjectClient>,
) -> UpdateManagerStruct {
    let (sender, receiver) = sync_channel(CHANNEL_SIZE);

    // the sync queue can't be mocked; needs to use the same server_api as the update_manager
    app.add_singleton_model(|ctx| SyncQueue::new(Default::default(), server_api.clone(), ctx));
    let update_manager =
        app.add_singleton_model(|ctx| UpdateManager::new(Some(sender.clone()), server_api, ctx));

    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));

    // set up the sync queue in a dequeueing state
    SyncQueue::handle(app).update(app, |sync_queue, ctx| {
        sync_queue.start_dequeueing(ctx);
    });

    let cloud_model_events = app.update(|ctx| {
        let (tx, rx) = async_channel::unbounded();
        ctx.subscribe_to_model(&CloudModel::handle(ctx), move |_, event, _| {
            let _ = tx.try_send(event.clone());
        });
        rx
    });

    // The start of polling is normally triggered by authentication completion, but
    // we need to do it manually for tests. We do this AFTER UpdateManager is created
    // so the polling uses the correct mock.
    TeamTesterStatus::handle(app).update(app, |team_tester, ctx| {
        team_tester.initiate_data_pollers(false, ctx);
    });

    UpdateManagerStruct {
        update_manager,
        receiver,
        cloud_model_events,
    }
}

/// Creates a baseline [`MockObjectClient`] with common mocks like:
/// * The logged-in user
/// * Background polling for updated objects
pub fn mock_server_api() -> MockObjectClient {
    let mut mock_object_client = MockObjectClient::new();
    // Mock *failures* for background fetches. This prevents `UpdateManager` clearing out any
    // objects that tests manually add to `CloudModel`.
    mock_object_client
        .expect_fetch_changed_objects()
        .returning(|_, _| Err(anyhow::anyhow!("Ignoring background refresh in tests")));
    // Mock environment timestamps fetch - return empty by default.
    mock_object_client
        .expect_fetch_environment_last_task_run_timestamps()
        .returning(|| Ok(HashMap::new()));
    mock_object_client
}
