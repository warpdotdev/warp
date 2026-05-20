use warp_core::ui::appearance::Appearance;
use warp_server_client::cloud_object::ServerPermissions;
use warpui::{
    platform::WindowStyle, AddSingletonModel, App, SingletonEntity, TypedActionView, ViewHandle,
};

use crate::{
    ai::blocklist::BlocklistAIHistoryModel,
    auth::{auth_manager::AuthManager, AuthStateProvider},
    cloud_object::{
        model::{actions::ObjectActions, persistence::CloudModel, view::CloudViewModel},
        CloudObjectSyncStatus, ObjectIdType, ObjectType, Owner, ServerCreationInfo, Space,
    },
    drive::{items::WarpDriveItemId, CloudObjectTypeAndId},
    menu::MenuItem,
    network::NetworkStatus,
    notebooks::{CloudNotebook, CloudNotebookModel},
    server::{
        cloud_objects::update_manager::UpdateManager,
        ids::{ClientId, ServerIdAndType, SyncId},
        server_api::ServerApiProvider,
        sync_queue::{QueueItem, SyncQueue},
        telemetry::context_provider::AppTelemetryContextProvider,
    },
    settings_view::keybindings::KeybindingChangedNotifier,
    terminal::shared_session::permissions_manager::SessionPermissionsManager,
    test_util::settings::initialize_settings_for_tests,
    workflows::{workflow::Workflow, CloudWorkflow, CloudWorkflowModel},
    workspaces::{
        team_tester::TeamTesterStatus, user_profiles::UserProfiles, user_workspaces::UserWorkspaces,
    },
    ASSETS,
};

use super::{DriveIndex, DriveIndexAction};

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudViewModel::mock);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
}

fn create_index(app: &mut App) -> ViewHandle<DriveIndex> {
    let (_, index) = app.add_window(WindowStyle::NotStealFocus, DriveIndex::new);
    index
}

fn create_workflow(app: &mut App) -> SyncId {
    CloudModel::handle(app).update(app, |cloud_model, ctx| {
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        let workflow = Workflow::new("my workflow", "my command");
        cloud_model.create_object(
            sync_id,
            CloudWorkflow::new_local(
                CloudWorkflowModel::new(workflow),
                Owner::mock_current_user(),
                None,
                client_id,
            ),
            ctx,
        );
        sync_id
    })
}

fn create_notebook(app: &mut App) -> SyncId {
    CloudModel::handle(app).update(app, |cloud_model, ctx| {
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        cloud_model.create_object(
            sync_id,
            CloudNotebook::new_local(
                CloudNotebookModel::default(),
                Owner::mock_current_user(),
                None,
                client_id,
            ),
            ctx,
        );
        sync_id
    })
}

fn set_object_in_error(app: &mut App, cloud_object_type_and_id: &CloudObjectTypeAndId) {
    CloudModel::handle(app).update(
        app,
        |cloud_model, _ctx: &mut warpui::ModelContext<'_, CloudModel>| {
            if let Some(object) = cloud_model.get_mut_by_uid(&cloud_object_type_and_id.uid()) {
                object.set_pending_content_changes_status(CloudObjectSyncStatus::Errored);
            }
        },
    );
}

fn label_for_menu_item(item: &MenuItem<DriveIndexAction>) -> &str {
    if let MenuItem::Item(item) = item {
        item.label()
    } else {
        panic!("item provided wasn't of type MenuItem::Item")
    }
}

#[test]
fn test_retry_menu_item_visibility() {
    App::test(ASSETS, |mut app| async move {
        initialize_app(&mut app);
        let index = create_index(&mut app);
        let sync_id = create_workflow(&mut app);
        let cloud_object_type_and_id: CloudObjectTypeAndId =
            CloudObjectTypeAndId::from_id_and_type(sync_id, ObjectType::Workflow);
        let warp_drive_item_id = WarpDriveItemId::Object(cloud_object_type_and_id);

        // by default, it doesn't show up
        index.update(&mut app, |index, ctx| {
            let menu_items = index.menu_items(&Space::Personal, &warp_drive_item_id, ctx);
            assert_eq!(menu_items.len(), 5);
            assert_eq!(label_for_menu_item(&menu_items[0]), "Edit");
            assert_eq!(label_for_menu_item(&menu_items[1]), "Copy workflow text");
            assert_eq!(label_for_menu_item(&menu_items[2]), "Share");
            assert_eq!(label_for_menu_item(&menu_items[3]), "Duplicate");
            assert_eq!(label_for_menu_item(&menu_items[4]), "Export");
        });

        // when the object is in error, it should show up
        set_object_in_error(&mut app, &cloud_object_type_and_id);
        index.update(&mut app, |index, ctx| {
            let menu_items = index.menu_items(&Space::Personal, &warp_drive_item_id, ctx);
            assert_eq!(menu_items.len(), 6);
            assert_eq!(label_for_menu_item(&menu_items[0]), "Retry");
            assert_eq!(label_for_menu_item(&menu_items[1]), "Edit");
            assert_eq!(label_for_menu_item(&menu_items[2]), "Copy workflow text");
            assert_eq!(label_for_menu_item(&menu_items[3]), "Share");
            assert_eq!(label_for_menu_item(&menu_items[4]), "Duplicate");
            assert_eq!(label_for_menu_item(&menu_items[5]), "Export");
        });

        // but if we're offline, it shouldn't show up
        NetworkStatus::handle(&app).update(&mut app, |network_status, ctx| {
            network_status.reachability_changed(false, ctx);
        });
        index.update(&mut app, |index, ctx| {
            let menu_items = index.menu_items(&Space::Personal, &warp_drive_item_id, ctx);
            assert_eq!(menu_items.len(), 5);
            assert_eq!(label_for_menu_item(&menu_items[0]), "Edit");
            assert_eq!(label_for_menu_item(&menu_items[1]), "Copy workflow text");
            assert_eq!(label_for_menu_item(&menu_items[2]), "Share");
            assert_eq!(label_for_menu_item(&menu_items[3]), "Duplicate");
            assert_eq!(label_for_menu_item(&menu_items[4]), "Export");
        });
    })
}

#[test]
fn test_retry_menu_item_logic() {
    App::test(ASSETS, |mut app| async move {
        initialize_app(&mut app);
        let index = create_index(&mut app);
        let sync_id = create_workflow(&mut app);
        let cloud_object_type_and_id: CloudObjectTypeAndId =
            CloudObjectTypeAndId::from_id_and_type(sync_id, ObjectType::Workflow);

        SyncQueue::handle(&app).update(&mut app, |sync_queue, _ctx| {
            sync_queue.stop_dequeueing();
            assert_eq!(sync_queue.queue().len(), 0);
        });

        index.update(&mut app, |index, ctx| {
            index.retry_failed_object(&cloud_object_type_and_id, ctx);
        });

        // the item is now in flight
        CloudModel::handle(&app).update(&mut app, |cloud_model, _ctx| {
            if let Some(object) = cloud_model.get_mut_by_uid(&cloud_object_type_and_id.uid()) {
                assert!(object.metadata().has_pending_content_changes());
            }
        });

        // with an object not known to the server, we enqueue a CreateWorkflow item
        SyncQueue::handle(&app).read(&app, |sync_queue, _ctx| {
            assert_eq!(sync_queue.queue().len(), 1);
            assert!(matches!(
                sync_queue.queue()[0].1,
                QueueItem::CreateWorkflow { .. }
            ))
        });

        let new_sync_id: SyncId = SyncId::ServerId(1.into());

        // make the object known to the server (by giving it a server id instead)
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            if let CloudObjectTypeAndId::Workflow(SyncId::ClientId(client_id)) =
                cloud_object_type_and_id
            {
                if let SyncId::ServerId(server_id) = new_sync_id {
                    let server_creation_info = ServerCreationInfo {
                        server_id_and_type: ServerIdAndType {
                            id: server_id,
                            id_type: ObjectIdType::Workflow,
                        },
                        creator_uid: None,
                        permissions: ServerPermissions::mock_personal(),
                    };
                    cloud_model.update_object_after_server_creation(
                        client_id,
                        server_creation_info,
                        ctx,
                    );
                }
            }
        });

        index.update(&mut app, |index, ctx| {
            let new_cloud_object_type_and_id: CloudObjectTypeAndId =
                CloudObjectTypeAndId::from_id_and_type(new_sync_id, ObjectType::Workflow);
            index.retry_failed_object(&new_cloud_object_type_and_id, ctx);
        });

        // with an object known to the server, we enqueue an UpdateWorkflow item
        SyncQueue::handle(&app).read(&app, |sync_queue, _ctx| {
            assert_eq!(sync_queue.queue().len(), 2);
            assert!(matches!(
                sync_queue.queue()[1].1,
                QueueItem::UpdateWorkflow { .. }
            ))
        });
    })
}

#[test]
fn test_warp_drive_navigation_states() {
    use crate::drive::index::DriveIndexAction;
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let index = create_index(&mut app);
        let sync_id = create_notebook(&mut app);
        let cloud_object_type_and_id: CloudObjectTypeAndId =
            CloudObjectTypeAndId::from_id_and_type(sync_id, ObjectType::Notebook);

        index.read(&app, |index, _| {
            assert_eq!(index.selected, None, "Expect selected to be None");
            assert_eq!(
                index.focused_index,
                Some(0),
                "Expect focused_index to be initialized"
            );
        });

        index.update(&mut app, |index, ctx| {
            index.handle_action(&DriveIndexAction::OpenObject(cloud_object_type_and_id), ctx);
        });

        index.read(&app, |index, _| {
            assert_eq!(
                index.selected,
                Some(WarpDriveItemId::Object(cloud_object_type_and_id)),
                "Expect selected to have correct value"
            );
        });
    });
}
