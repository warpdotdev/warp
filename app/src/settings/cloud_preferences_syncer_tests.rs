use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use warp_core::settings::macros::define_settings_group;
use warp_core::settings::{RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud};
use warp_core::user_preferences::GetUserPreferences;
use warpui::{App, SingletonEntity};

use super::{
    initialize_cloud_preferences_syncer, ClientIdProvider, CloudPreferencesSyncer,
    ForceCloudToMatchLocal, SETTINGS_FILE_LAST_SYNCED_HASH_KEY,
};
use crate::auth::auth_state::AuthState;
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::{
    BulkCreateCloudObjectResult, CreatedCloudObject, GenericStringObjectFormat,
    GenericStringObjectUniqueKey, JsonObjectType, ObjectDeleteResult, ObjectIdType, Owner,
    Revision, RevisionAndLastEditor, ServerMetadata, ServerObject, ServerPermissions,
    ServerPreference, UniquePer, UpdateCloudObjectResult,
};
use crate::server::cloud_objects::fake_object_client::FakeObjectClient;
use crate::server::cloud_objects::test_utils::{
    create_update_manager_struct, initialize_app, UpdateManagerStruct,
};
use crate::server::cloud_objects::update_manager::{InitialLoadResponse, UpdateManager};
use crate::server::ids::{ClientId, ServerId, ServerIdAndType, SyncId};
use crate::server::server_api::object::MockObjectClient;
use crate::server::sync_queue::SyncQueue;
use crate::settings::cloud_preferences::{
    CloudPreferenceModel, CloudPreferencesSettings, Platform,
};
use crate::ASSETS;

define_settings_group!(TestSettings, settings: [
    all_platforms_cloud_setting: AllPlatforms {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    all_platforms_always_sync_cloud_setting: AllPlatformsAlwaysSync {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: true,
    },
    mac_only_cloud_setting: MacOnly {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes),
        private: true,
    },
    linux_only_cloud_setting: LinuxOnly {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::LINUX,
        sync_to_cloud: SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes),
        private: true,
    },
    platform_specific_cloud_setting: PlatformSpecific {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes),
        private: true,
    },
    non_value_syncable_setting: NonValueSyncable {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    non_cloud_setting: NonCloud {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    },
    hashset_cloud_setting: HashSetSetting {
        type: HashSet<String>,
        default: HashSet::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
]);

impl NonValueSyncable {
    fn current_value_is_syncable(&self) -> bool {
        false
    }
}

struct SettingToLoad {
    id: GenericStringObjectId,
    serialized_preference: String,
}

struct TestClientIdProvider {
    client_ids: Mutex<VecDeque<ClientId>>,
}

impl TestClientIdProvider {
    fn new(client_ids: Vec<ClientId>) -> Self {
        Self {
            client_ids: Mutex::new(client_ids.into()),
        }
    }
}

impl ClientIdProvider for TestClientIdProvider {
    fn next_client_id(&self) -> ClientId {
        self.client_ids
            .lock()
            .unwrap()
            .pop_front()
            .expect("no client ids left")
    }
}

fn initialize_settings(app: &mut App) {
    initialize_app(app);
    TestSettings::register(app);
    CloudPreferencesSettings::register(app);
}

fn enable_settings_sync(app: &mut App) {
    app.update(|ctx| {
        CloudPreferencesSettings::handle(ctx).update(ctx, |prefs_settings, ctx| {
            let _ = prefs_settings.settings_sync_enabled.set_value(true, ctx);
        });
    });
}

fn initial_load_response_with_cloud_settings(
    settings_to_load: Vec<SettingToLoad>,
) -> InitialLoadResponse {
    let mut initial_load_response = InitialLoadResponse::default();
    let settings = settings_to_load
        .iter()
        .map(|setting| {
            let id = setting.id;
            let metadata = ServerMetadata {
                uid: ServerId::default(),
                revision: Revision::now(),
                metadata_last_updated_ts: Utc::now().into(),
                trashed_ts: None,
                folder_id: None,
                is_welcome_object: false,
                creator_uid: None,
                last_editor_uid: None,
                current_editor_uid: None,
            };

            let cloud_setting = ServerPreference::new(
                SyncId::ServerId(id.into()),
                CloudPreferenceModel::deserialize_owned(&setting.serialized_preference)
                    .expect("error creating preference"),
                metadata,
                ServerPermissions {
                    space: Owner::mock_current_user(),
                    guests: Vec::new(),
                    anyone_link_sharing: None,
                    permissions_last_updated_ts: Utc::now().into(),
                },
            );
            Box::new(cloud_setting) as Box<dyn ServerObject>
        })
        .collect::<Vec<Box<dyn ServerObject>>>();
    initial_load_response.updated_generic_string_objects.insert(
        GenericStringObjectFormat::Json(JsonObjectType::Preference),
        settings,
    );
    initial_load_response
}

fn expect_sync_preferences_setting(server_api: &mut MockObjectClient) -> Vec<ClientId> {
    expect_bulk_create_generic_string_objects(server_api, 1)
}

fn expect_sync_server_stored_privacy_settings(server_api: &mut MockObjectClient) -> Vec<ClientId> {
    expect_bulk_create_generic_string_objects(server_api, 4)
}

async fn spawned_sync_queue_future_at_index(app: &mut App, index: usize) {
    SyncQueue::handle(app)
        .update(app, |sync_queue, ctx| {
            ctx.await_spawned_future(sync_queue.spawned_futures()[index])
        })
        .await
}
async fn wait_for_num_spawned_futures(app: &mut App, expected_num: usize, message: &str) {
    for _ in 0..50 {
        let num_spawned_futures =
            SyncQueue::handle(app).read(app, |sync_queue, _ctx| sync_queue.spawned_futures().len());
        if num_spawned_futures == expected_num {
            return;
        }
        warpui::r#async::Timer::after(Duration::from_millis(100)).await;
    }

    assert_num_spawned_futures(app, expected_num, message);
}

async fn await_spawned_futures(app: &mut App, num_futures: usize, message: &str) {
    wait_for_num_spawned_futures(app, num_futures, message).await;
    for _ in 0..num_futures {
        spawned_sync_queue_future_at_index(app, 0).await;
    }
}

fn assert_num_spawned_futures(app: &mut App, expected_num: usize, message: &str) {
    SyncQueue::handle(app).read(app, |sync_queue, _ctx| {
        assert_eq!(
            expected_num,
            sync_queue.spawned_futures().len(),
            "{message}"
        );
    });
}

fn random_server_id() -> ServerId {
    rand::random::<i64>().into()
}

/// Creates a MockObjectClient with base expectations needed for tests that call mock_initial_load.
fn mock_object_client_with_base_expectations() -> MockObjectClient {
    let mut server_api = MockObjectClient::new();
    // Mock environment timestamps fetch - called during mock_initial_load
    server_api
        .expect_fetch_environment_last_task_run_timestamps()
        .returning(|| Ok(HashMap::new()));
    server_api
}

/// Expects a bulk create request for the given number of settings and returns the client ids
/// used to create those objects. The  reason we need to return the client ids is so the
/// cloud prefs sycner can use them to create objects, and the mock server responses match
/// those created objects. If the client ids don't match, then object updates for those objects will fail
/// because the sync queue is blocked waiting for dependent requests to complete.
fn expect_bulk_create_generic_string_objects(
    server_api: &mut MockObjectClient,
    num_objects: usize,
) -> Vec<ClientId> {
    let client_ids: Vec<ClientId> = (0..num_objects).map(|_| ClientId::new()).collect();
    let client_ids_clone = client_ids.clone();
    server_api
        .expect_bulk_create_generic_string_objects()
        .times(1)
        .return_once(move |_, _| {
            let res = client_ids_clone
                .into_iter()
                .map(|client_id| CreatedCloudObject {
                    // Note that the client id is set here in the mock response.
                    client_id,
                    revision_and_editor: RevisionAndLastEditor {
                        revision: Revision::now(),
                        last_editor_uid: Some("34jkaosdfss".to_string()),
                    },
                    metadata_ts: DateTime::<Utc>::default().into(),
                    server_id_and_type: ServerIdAndType {
                        id: random_server_id(),
                        id_type: ObjectIdType::GenericStringObject,
                    },
                    creator_uid: None,
                    permissions: ServerPermissions::mock_personal(),
                })
                .collect();
            Ok(BulkCreateCloudObjectResult::Success {
                created_cloud_objects: res,
            })
        });
    client_ids
}

#[test]
fn test_sync_local_pref_to_cloud_after_initial_sync_creates_prefs_setting() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();

        // Expect an initial create request for CloudPreferencesSetting
        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        enable_settings_sync(&mut app);
        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new_for_test(
                ctx,
                Arc::new(TestClientIdProvider::new(all_client_ids)),
            );
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        app.update(|ctx| {
            // Force initial load
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(InitialLoadResponse::default(), ctx);
            });
        });

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // Wait for the syncer to create the preferences and privacy settings.
        await_spawned_futures(
            &mut app,
            3,
            "expect the syncer to create the preferences and privacy settings",
        )
        .await;
    })
}

#[test]
fn test_sync_local_pref_to_cloud_after_initial_sync() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let is_mac = cfg!(all(not(target_family = "wasm"), target_os = "macos"));
        let is_linux = cfg!(all(
            not(target_family = "wasm"),
            any(target_os = "linux", target_os = "freebsd")
        ));

        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        // Expect the creation of one or two cloud settings in separate requests depending on the platform
        all_client_ids.append(&mut expect_bulk_create_generic_string_objects(
            &mut server_api,
            1,
        ));

        if is_mac || is_linux {
            let per_platform_client_id = ClientId::new();
            all_client_ids.push(per_platform_client_id);
            server_api
                .expect_bulk_create_generic_string_objects()
                .times(1)
                .return_once(move |_, objects| {
                    assert_eq!(1, objects.len());
                    if is_mac {
                        assert_eq!(
                            &objects[0].uniqueness_key,
                            &Some(GenericStringObjectUniqueKey {
                                key: format!(
                                    "{}_{}",
                                    Platform::Mac,
                                    MacOnly::storage_key().to_owned()
                                ),
                                unique_per: UniquePer::User
                            })
                        );
                    } else if is_linux {
                        assert_eq!(
                            &objects[0].uniqueness_key,
                            &Some(GenericStringObjectUniqueKey {
                                key: format!(
                                    "{}_{}",
                                    Platform::Linux,
                                    LinuxOnly::storage_key().to_owned()
                                ),
                                unique_per: UniquePer::User
                            })
                        );
                    }

                    Ok(BulkCreateCloudObjectResult::Success {
                        created_cloud_objects: vec![CreatedCloudObject {
                            client_id: per_platform_client_id,
                            revision_and_editor: RevisionAndLastEditor {
                                revision: Revision::now(),
                                last_editor_uid: Some("34jkaosdfk".to_string()),
                            },
                            metadata_ts: DateTime::<Utc>::default().into(),
                            server_id_and_type: ServerIdAndType {
                                id: random_server_id(),
                                id_type: ObjectIdType::GenericStringObject,
                            },
                            creator_uid: None,
                            permissions: ServerPermissions::mock_personal(),
                        }],
                    })
                });
        }

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        enable_settings_sync(&mut app);
        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new_for_test(
                ctx,
                Arc::new(TestClientIdProvider::new(all_client_ids)),
            );
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        app.update(|ctx| {
            // Force initial load
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(InitialLoadResponse::default(), ctx);
            });
        });

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        app.update(|ctx| {
            // And then update all settings forcing the create requests
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .all_platforms_cloud_setting
                    .set_value(true, ctx);
                if cfg!(all(not(target_family = "wasm"), target_os = "macos")) {
                    let _ = test_settings.mac_only_cloud_setting.set_value(true, ctx);
                } else if cfg!(all(
                    not(target_family = "wasm"),
                    any(target_os = "linux", target_os = "freebsd")
                )) {
                    let _ = test_settings.linux_only_cloud_setting.set_value(true, ctx);
                }
                let _ = test_settings.non_cloud_setting.set_value(true, ctx);
                let _ = test_settings
                    .non_value_syncable_setting
                    .set_value(true, ctx);
            })
        });

        let expected_num_create_requests = if is_mac || is_linux { 5 } else { 4 };
        await_spawned_futures(
            &mut app,
            expected_num_create_requests,
            "expect the syncer to create the settings",
        )
        .await;
    })
}

fn run_initial_sync_test(is_onboarded: bool) {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let is_mac = cfg!(all(not(target_family = "wasm"), target_os = "macos"));
        let is_linux = cfg!(all(
            not(target_family = "wasm"),
            any(target_os = "linux", target_os = "freebsd")
        ));

        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);

        if !is_onboarded {
            // Only sync other settings if the user isn't onboarded (e.g. is a first time user)
            let expected_num_other_settings_client_ids = if is_mac || is_linux { 6 } else { 5 };
            all_client_ids.append(&mut expect_bulk_create_generic_string_objects(
                &mut server_api,
                expected_num_other_settings_client_ids,
            ));
        };

        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        if !is_onboarded {
            server_api
                .expect_update_generic_string_object()
                .times(1)
                .return_once(move |_, _, _| {
                    Ok(UpdateCloudObjectResult::<Box<dyn ServerObject>>::Success {
                        revision_and_editor: RevisionAndLastEditor {
                            revision: Revision::now(),
                            last_editor_uid: None,
                        },
                    })
                });
        }

        if !is_onboarded {
            server_api
                .expect_fetch_changed_objects()
                .times(1)
                .return_once(move |_, _| Ok(InitialLoadResponse::default()));
        }

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        app.update(|ctx| {
            // Force initial load with no cloud objects
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(InitialLoadResponse::default(), ctx);
            });

            // And then update both settings
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                // Make sure to sync at least one setting
                let _ = test_settings
                    .all_platforms_always_sync_cloud_setting
                    .set_value(true, ctx);
                let _ = test_settings
                    .all_platforms_cloud_setting
                    .set_value(true, ctx);
                if is_mac {
                    let _ = test_settings.mac_only_cloud_setting.set_value(true, ctx);
                } else if is_linux {
                    let _ = test_settings.linux_only_cloud_setting.set_value(true, ctx);
                }
                let _ = test_settings.non_cloud_setting.set_value(true, ctx);
            });
        });

        // Do the initial load after the settings have been updated locally
        app.add_singleton_model(|ctx| {
            let mut syncer = CloudPreferencesSyncer::new_for_test(
                ctx,
                Arc::new(TestClientIdProvider::new(all_client_ids)),
            );
            let auth_state = AuthState::new_for_test();
            auth_state.set_is_onboarded(is_onboarded);
            syncer.handle_user_fetched(Arc::new(auth_state), ctx);
            syncer
        });

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        let expected_num_spawned_futures = if !is_onboarded { 4 } else { 3 };
        await_spawned_futures(
            &mut app,
            expected_num_spawned_futures,
            "expect the syncer to create the settings",
        )
        .await;

        app.update(|ctx| {
            // Now update the settings again to ensure the syncer only syncs for non-onboarded users.
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                // Sync a cloud only setting by changing the value we had set earlier.
                let _ = test_settings
                    .all_platforms_cloud_setting
                    .set_value(false, ctx);
            });
        });

        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        assert_eq!(
            is_onboarded,
            !app.read(|ctx| {
                *CloudPreferencesSettings::handle(ctx)
                    .as_ref(ctx)
                    .settings_sync_enabled
            }),
            "settings sync enabled should be opposite of onboarded"
        );

        if !is_onboarded {
            // If the user isn't onboarded, they should have been opted into sync and sent another request.
            assert_num_spawned_futures(
                &mut app,
                expected_num_spawned_futures + 1,
                "expect an additional update request",
            );
            spawned_sync_queue_future_at_index(&mut app, expected_num_spawned_futures).await;
        }
    })
}

#[test]
fn test_sync_local_pref_to_cloud_on_initial_sync_for_first_time_user() {
    run_initial_sync_test(false);
}

#[test]
fn test_sync_local_pref_to_cloud_on_initial_sync_for_returning_user() {
    run_initial_sync_test(true);
}

#[test]
fn test_sync_local_pref_to_cloud_updates_existing_pref() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        // Add missing fetch_changed_objects expectation
        server_api
            .expect_fetch_changed_objects()
            .times(1)
            .return_once(move |_, _| Ok(InitialLoadResponse::default()));

        // Expect updating a single cloud setting
        server_api
            .expect_update_generic_string_object()
            .times(1)
            .return_once(move |_, _, _| {
                Ok(UpdateCloudObjectResult::<Box<dyn ServerObject>>::Success {
                    revision_and_editor: RevisionAndLastEditor {
                        revision: Revision::now(),
                        last_editor_uid: None,
                    },
                })
            });

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        enable_settings_sync(&mut app);

        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new_for_test(
                ctx,
                Arc::new(TestClientIdProvider::new(all_client_ids)),
            );
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        let generic_object_id: GenericStringObjectId = 123.into();
        app.update(|ctx| {
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(
                    initial_load_response_with_cloud_settings(vec![SettingToLoad {
                        id: generic_object_id,
                        serialized_preference: "{\"storage_key\":\"AllPlatforms\",\"value\":true,\"platform\":\"Global\"}".to_owned(),
                    }]),
                    ctx,
                );
            });
        });

        // Give the initial load time to complete
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // complete the create request for the cloud settings and the telemetry/crash reporting settings
        await_spawned_futures(
            &mut app,
            3,
            "expect the syncer to create the initial settings",
        )
        .await;

        app.update(|ctx| {
            // And then update settings - should trigger an update server call since
            // the value has changed (just for the cloud synced setting)
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .all_platforms_cloud_setting
                    .set_value(false, ctx);
                let _ = test_settings.non_cloud_setting.set_value(true, ctx);
            });
        });

        // Give the update time to spawn futures
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        assert_num_spawned_futures(&mut app, 4, "expect the syncer to send an update request");
        spawned_sync_queue_future_at_index(&mut app, 3).await;
    })
}

#[test]
fn test_sync_cloud_pref_to_local_on_initial_load_or_collab_update() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        let generic_object_id_1: GenericStringObjectId = 123.into();
        let generic_object_id_2: GenericStringObjectId = 345.into();
        let generic_object_id_3: GenericStringObjectId = 456.into();
        let generic_object_id_4: GenericStringObjectId = 567.into();

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        app.update(|ctx| {
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(
                    initial_load_response_with_cloud_settings(vec![
                        SettingToLoad {
                            id: generic_object_id_1,
                            serialized_preference: "{\"storage_key\":\"AllPlatforms\",\"value\":true,\"platform\":\"Global\"}".to_owned(),
                        },
                        SettingToLoad {
                            id: generic_object_id_2,
                            serialized_preference: "{\"storage_key\":\"MacOnly\",\"value\":true,\"platform\":\"Mac\"}".to_owned(),
                        },
                        SettingToLoad {
                            id: generic_object_id_3,
                            serialized_preference: "{\"storage_key\":\"LinuxOnly\",\"value\":true,\"platform\":\"Linux\"}".to_owned(),
                        },
                        SettingToLoad {
                            id: generic_object_id_4,
                            serialized_preference: "{\"storage_key\":\"PlatformSpecific\",\"value\":true,\"platform\":\"Linux\"}".to_owned(),
                        },
                    ]),
                    ctx,
                );
            });
        });

        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                !settings.all_platforms_cloud_setting.inner,
                "setting not set locally"
            );
        });

        enable_settings_sync(&mut app);

        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new(false, std::path::PathBuf::new(), ctx);
            // This should sync the cloud preferences at this point
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // complete the create request for the cloud settings and the telemetry/crash reporting settings
        await_spawned_futures(
            &mut app,
            3,
            "expect the syncer to create the initial settings",
        )
        .await;

        let is_mac = cfg!(all(not(target_family = "wasm"), target_os = "macos"));
        let is_linux = cfg!(all(
            not(target_family = "wasm"),
            any(target_os = "linux", target_os = "freebsd")
        ));
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                settings.all_platforms_cloud_setting.inner,
                "setting set locally"
            );
            if is_mac {
                assert!(
                    settings.mac_only_cloud_setting.inner,
                    "mac setting set locally"
                );
                assert!(
                    !settings.platform_specific_cloud_setting.inner,
                    "platform specific setting should not be set locally on mac"
                );
                assert!(
                    !settings.linux_only_cloud_setting.inner,
                    "linux setting not set locally"
                );
            } else if is_linux {
                assert!(
                    settings.linux_only_cloud_setting.inner,
                    "linux setting set locally"
                );
                assert!(
                    settings.platform_specific_cloud_setting.inner,
                    "platform specific setting set locally"
                );
                assert!(
                    !settings.mac_only_cloud_setting.inner,
                    "mac setting not set locally"
                );
            }
        });
    })
}

#[test]
fn test_cloud_preferences_setting_initial_load_skipped_when_setting_is_off() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let settings_object_id: GenericStringObjectId = 123.into();
        let generic_object_id_1: GenericStringObjectId = 123.into();
        let generic_object_id_2: GenericStringObjectId = 345.into();
        let generic_object_id_3: GenericStringObjectId = 456.into();
        let generic_object_id_4: GenericStringObjectId = 567.into();

        // Expect creating the cloud settings after the setting is enabled
        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        app.update(|ctx| {
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(
                    initial_load_response_with_cloud_settings(vec![
                        SettingToLoad {
                            id: settings_object_id,
                            serialized_preference: "{\"storage_key\":\"IsSettingsSyncEnabled\",\"value\":false,\"platform\":\"Global\"}".to_owned(),
                        },
                        SettingToLoad {
                            id: generic_object_id_1,
                            serialized_preference: "{\"storage_key\":\"AllPlatforms\",\"value\":true,\"platform\":\"Global\"}".to_owned(),
                        },
                        SettingToLoad {
                            id: generic_object_id_2,
                            serialized_preference: "{\"storage_key\":\"MacOnly\",\"value\":true,\"platform\":\"Mac\"}".to_owned(),
                        },
                        SettingToLoad {
                            id: generic_object_id_3,
                            serialized_preference: "{\"storage_key\":\"LinuxOnly\",\"value\":true,\"platform\":\"Linux\"}".to_owned(),
                        },
                        SettingToLoad {
                            id: generic_object_id_4,
                            serialized_preference: "{\"storage_key\":\"PlatformSpecific\",\"value\":true,\"platform\":\"Linux\"}".to_owned(),
                        },
                    ]),
                    ctx,
                );
            });
        });

        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                !settings.all_platforms_cloud_setting.inner,
                "setting not set locally"
            );
        });

        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new_for_test(
                ctx,
                Arc::new(TestClientIdProvider::new(all_client_ids)),
            );
            // No syncing should happen since the setting is disabled.
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        let is_mac = cfg!(all(not(target_family = "wasm"), target_os = "macos"));
        let is_linux = cfg!(all(
            not(target_family = "wasm"),
            any(target_os = "linux", target_os = "freebsd")
        ));
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                !settings.all_platforms_cloud_setting.inner,
                "setting set locally"
            );
            if is_mac {
                assert!(
                    !settings.mac_only_cloud_setting.inner,
                    "mac setting set locally"
                );
                assert!(
                    !settings.platform_specific_cloud_setting.inner,
                    "platform specific setting should not be set locally on mac"
                );
                assert!(
                    !settings.linux_only_cloud_setting.inner,
                    "linux setting not set locally"
                );
            } else if is_linux {
                assert!(
                    !settings.linux_only_cloud_setting.inner,
                    "linux setting set locally"
                );
                assert!(
                    !settings.platform_specific_cloud_setting.inner,
                    "platform specific setting set locally"
                );
                assert!(
                    !settings.mac_only_cloud_setting.inner,
                    "mac setting not set locally"
                );
            }
        });

        // Now enable the setting and make sure everything syncs
        enable_settings_sync(&mut app);

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // complete the create request for the cloud settings and the telemetry/crash reporting settings
        await_spawned_futures(
            &mut app,
            3,
            "expect the syncer to create the initial settings",
        )
        .await;
    })
}

#[test]
fn test_sync_local_pref_to_cloud_doesnt_update_equal_pref() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let settings_object_id: GenericStringObjectId = 123.into();

        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        // Expect updating a single cloud setting when we extend the hashset
        server_api
            .expect_update_generic_string_object()
            .times(1)
            .return_once(move |_, _, _| {
                Ok(UpdateCloudObjectResult::<Box<dyn ServerObject>>::Success {
                    revision_and_editor: RevisionAndLastEditor {
                        revision: Revision::now(),
                        last_editor_uid: None,
                    },
                })
            });

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        app.update(|ctx| {
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(
                    initial_load_response_with_cloud_settings(vec![
                        SettingToLoad {
                            id: settings_object_id,
                            // use a hashset setting that deserializes as a json array in a non-deterministic order
                            serialized_preference: "{\"storage_key\":\"HashSetSetting\",\"value\":[\"foo\", \"bar\"],\"platform\":\"Global\"}".to_owned(),
                        },
                    ]),
                    ctx,
                );
            });
        });

        enable_settings_sync(&mut app);
        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new(false, std::path::PathBuf::new(), ctx);
            // This should sync the cloud preferences at this point
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // Complete the create request for cloud prefs syncing
        await_spawned_futures(
            &mut app,
            3,
            "expect the syncer to create the initial settings",
        )
        .await;

        // After the initial load the settings should be set.
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert_eq!(2, settings.hashset_cloud_setting.value().len());
            assert!(settings.hashset_cloud_setting.value().contains("foo"));
            assert!(settings.hashset_cloud_setting.value().contains("bar"));
        });

        // This should not trigger an update request since the values are the same.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings.hashset_cloud_setting.set_value(
                    // Reverse the order and make sure the settings are still considered equal
                    HashSet::from_iter(vec!["bar".to_string(), "foo".to_string()]),
                    ctx,
                );
            });
        });

        assert_num_spawned_futures(&mut app, 3, "should not be any additional requests");

        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings.hashset_cloud_setting.set_value(
                    // Add an element and make sure the settings are considered different
                    HashSet::from_iter(vec![
                        "bar".to_string(),
                        "foo".to_string(),
                        "zed".to_string(),
                    ]),
                    ctx,
                );
            });
        });

        // Complete the second update request (fourth total request)
        assert_num_spawned_futures(&mut app, 4, "should be an update request");
        spawned_sync_queue_future_at_index(&mut app, 3).await;
    })
}

#[test]
fn test_cloud_preferences_setting_enabling_setting_syncs_prefs() {
    App::test(ASSETS, |mut app| async move {
        // Start with cloud prefs disabled
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));
        // Expect creating a single generic string object as an additional synced setting.
        all_client_ids.append(&mut expect_bulk_create_generic_string_objects(
            &mut server_api,
            1,
        ));

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        // Now enable settings sync
        enable_settings_sync(&mut app);
        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new(false, std::path::PathBuf::new(), ctx);
            // This should sync the cloud preferences at this point
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        app.update(|ctx| {
            // Force initial load
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(InitialLoadResponse::default(), ctx);
            });
        });

        // Spend time waiting for the initial load to finish etc.
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        await_spawned_futures(
            &mut app,
            3,
            "expect the syncer to create the initial settings",
        )
        .await;

        // update a different setting
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .all_platforms_cloud_setting
                    .set_value(true, ctx);
            });
        });

        // complete the create request for the new synced setting
        assert_num_spawned_futures(&mut app, 4, "should be an additional create request");
        spawned_sync_queue_future_at_index(&mut app, 3).await;
    })
}

#[test]
fn test_cloud_pref_not_synced_when_current_value_not_syncable() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        // Create a cloud preference for non_value_syncable_setting with true
        let settings_object_id: GenericStringObjectId = 123.into();
        let initial_load = initial_load_response_with_cloud_settings(vec![SettingToLoad {
            id: settings_object_id,
            serialized_preference: "{\"storage_key\":\"non_value_syncable_setting\",\"value\":true,\"platform\":\"Global\"}".to_owned(),
        }]);

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        // Set local value to false
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .non_value_syncable_setting
                    .set_value(false, ctx);
            });
        });

        // Enable settings sync
        enable_settings_sync(&mut app);

        app.update(|ctx| {
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(initial_load, ctx);
            });
        });

        // Add the syncer and trigger sync
        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new(false, std::path::PathBuf::new(), ctx);
            // This should sync the cloud preferences at this point
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        // Run any spawned futures
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;
        await_spawned_futures(&mut app, 3, "initial load").await;

        // Verify that the local value remains false and wasn't synced from cloud's true
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                !settings.non_value_syncable_setting.value(),
                "Local value should remain false since current value is not syncable"
            );
        });
    })
}

#[test]
fn test_ensure_no_duplicate_cloud_prefs() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        let mut server_api = mock_object_client_with_base_expectations();
        let mut all_client_ids = expect_sync_preferences_setting(&mut server_api);
        all_client_ids.append(&mut expect_sync_server_stored_privacy_settings(
            &mut server_api,
        ));

        // Set up delete expectations for the duplicate preferences
        server_api.expect_delete_object().times(3).returning(|_| {
            Ok(ObjectDeleteResult::Success {
                deleted_ids: Vec::new(),
            })
        });

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(server_api));

        // Create three duplicate preferences for the same setting with different IDs
        // One matching the local value (true) and two with different values
        let duplicate_prefs = vec![
            SettingToLoad {
                id: 123.into(),
                serialized_preference:
                    "{\"storage_key\":\"AllPlatforms\",\"value\":false,\"platform\":\"Global\"}"
                        .to_owned(),
            },
            SettingToLoad {
                id: 456.into(),
                serialized_preference:
                    "{\"storage_key\":\"AllPlatforms\",\"value\":true,\"platform\":\"Global\"}"
                        .to_owned(),
            },
            SettingToLoad {
                id: 789.into(),
                serialized_preference:
                    "{\"storage_key\":\"AllPlatforms\",\"value\":false,\"platform\":\"Global\"}"
                        .to_owned(),
            },
        ];

        enable_settings_sync(&mut app);

        app.update(|ctx| {
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.mock_initial_load(
                    initial_load_response_with_cloud_settings(duplicate_prefs),
                    ctx,
                );
            });
        });

        app.add_singleton_model(|ctx| {
            let syncer = CloudPreferencesSyncer::new_for_test(
                ctx,
                Arc::new(TestClientIdProvider::new(all_client_ids)),
            );
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        // Give time for the initial load and deduplication to complete
        warpui::r#async::Timer::after(Duration::from_secs(1)).await;

        // Wait for the initial creation of cloud settings and telemetry settings
        await_spawned_futures(
            &mut app,
            3,
            "expect the syncer to create the initial settings",
        )
        .await;

        // After the initial operations complete, expect 3 delete operations for duplicates
        // plus 1 for the timestamps fetch = 4 total on UpdateManager
        UpdateManager::handle(&app).read(&app, |update_manager, _ctx| {
            assert_eq!(
                4,
                update_manager.spawned_futures().len(),
                "expect three delete operations for duplicate preferences plus timestamps fetch"
            );
        });

        for i in 0..4 {
            // Complete the delete operations and timestamps fetch
            UpdateManager::handle(&app)
                .update(&mut app, |update_manager, ctx| {
                    ctx.await_spawned_future(update_manager.spawned_futures()[i])
                })
                .await;
        }
    })
}

// ============================================================================
// End-to-end tests for the force-local-wins-on-startup hash check
// (QUALITY-474)
//
// These tests exercise the full path from hash comparison through syncer
// construction to cloud upload, using a real temporary TOML file and the
// stateful `FakeObjectClient`. They use the production entry point
// `initialize_cloud_preferences_syncer` so the hash-check logic is
// exercised end-to-end rather than hard-coded.
//
// Because the existing test harness installs an in-memory
// `PublicPreferences` backend (not a real `TomlBackedUserPreferences`),
// the temp settings file is only consulted for its hash; local setting
// values still live in the in-memory store. Tests manually set the local
// value via `TestSettings::set_value` to simulate "this is what the file
// says".
// ============================================================================

/// Drains all currently-pending `SyncQueue` futures to completion, then
/// waits a short timer to give the syncer time to spawn any follow-up
/// work before the test asserts on cloud state.
async fn drain_sync_queue(app: &mut App) {
    // The syncer spawns futures for bulk creates, updates, and deletes
    // through the sync queue. We drain them in a loop because draining
    // one can cause others to be spawned (e.g. an update in response
    // to a cloud change).
    for _ in 0..5 {
        warpui::r#async::Timer::after(Duration::from_millis(200)).await;
        let num = SyncQueue::handle(app).read(app, |sq, _| sq.spawned_futures().len());
        if num == 0 {
            return;
        }
        for _ in 0..num {
            spawned_sync_queue_future_at_index(app, 0).await;
        }
    }
}

/// Writes a fresh `settings.toml` file at the given path and returns
/// its `file_content_hash` so tests can reason about what value the
/// syncer will compare against.
fn write_settings_file_with_content(path: &std::path::Path, content: &str) -> String {
    std::fs::write(path, content).expect("write temp settings file");
    warpui_extras::user_preferences::toml_backed::TomlBackedUserPreferences::file_content_hash(path)
        .expect("hash should be Some for non-empty file")
}

fn read_stored_hash(app: &App) -> Option<String> {
    app.read(|ctx| {
        ctx.private_user_preferences()
            .read_value(SETTINGS_FILE_LAST_SYNCED_HASH_KEY)
            .unwrap_or_default()
    })
}

fn write_stored_hash(app: &App, value: &str) {
    app.read(|ctx| {
        ctx.private_user_preferences()
            .write_value(SETTINGS_FILE_LAST_SYNCED_HASH_KEY, value.to_string())
            .expect("write stored hash to in-memory private prefs");
    });
}

#[test]
fn test_force_local_wins_on_startup_uploads_local_to_cloud() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        // Step 1: create a real temp settings.toml. The file's hash is
        // what the syncer will compare against the stored hash.
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("settings.toml");
        let file_hash = write_settings_file_with_content(
            &toml_path,
            "# user edited this offline\nsome_key = \"new_value\"\n",
        );

        // Step 2: seed the stored hash with a DIFFERENT value so the
        // comparison detects divergence. "0" is guaranteed not to
        // match a real SHA-256 hex digest.
        assert_ne!("0", file_hash);
        write_stored_hash(&app, "0");

        // Step 3: simulate the user's in-memory local values matching
        // "what the file says". The file contents aren't actually read
        // by the in-memory prefs backend, so tests set the local value
        // explicitly here.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .all_platforms_cloud_setting
                    .set_value(true, ctx);
            });
        });

        // Step 4: seed the fake cloud with a stale value. If the
        // syncer runs in normal "cloud wins" mode, it would overwrite
        // the local true with this false; the force-local path should
        // instead upload the local true over this false.
        let fake = FakeObjectClient::default();
        fake.seed_preference("AllPlatforms", "false", Platform::Global);

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(fake.clone()));
        enable_settings_sync(&mut app);

        // Step 5: construct the syncer via the production entry point
        // with the real file path. This is the seam where the hash
        // comparison runs.
        let toml_path_for_closure = toml_path.clone();
        app.add_singleton_model(move |ctx| {
            let syncer = initialize_cloud_preferences_syncer(
                toml_path_for_closure,
                None, // no parse error
                ctx,
            );
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        // Step 6: trigger the initial load with the fake's current
        // snapshot. This kicks off handle_initial_load, which sees the
        // force_local_wins_on_startup flag and uploads local to cloud.
        let initial_load = fake.snapshot_as_initial_load_response();
        app.update(|ctx| {
            update_manager.update(ctx, |um, ctx| {
                um.mock_initial_load(initial_load, ctx);
            });
        });

        drain_sync_queue(&mut app).await;

        // Assertion 1: the cloud now reflects the local value. If the
        // hash check or the override path were broken, the cloud would
        // still hold "false".
        assert_eq!(
            Some("true".to_string()),
            fake.cloud_value("AllPlatforms", Platform::Global),
            "local value should have been uploaded to cloud"
        );

        // Assertion 2: the stored hash is no longer "0" and now
        // matches the current file's hash. This covers the
        // `update_stored_settings_hash` call at the end of
        // `handle_initial_load`.
        let new_stored = read_stored_hash(&app);
        assert_eq!(
            Some(file_hash),
            new_stored,
            "stored hash should match current file hash after sync",
        );
    })
}

#[test]
fn test_no_force_local_when_hashes_match() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        // Create a file and seed the stored hash with its exact value
        // so the comparison yields "no divergence".
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("settings.toml");
        let file_hash =
            write_settings_file_with_content(&toml_path, "# in-sync with cloud\nkey = \"value\"\n");
        write_stored_hash(&app, &file_hash);

        // Start with local = false (the default for the test setting).
        // Seed cloud with true. Normal "cloud wins" semantics should
        // apply the cloud true to local.
        let fake = FakeObjectClient::default();
        fake.seed_preference("AllPlatforms", "true", Platform::Global);

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(fake.clone()));
        enable_settings_sync(&mut app);

        let toml_path_for_closure = toml_path.clone();
        app.add_singleton_model(move |ctx| {
            let syncer = initialize_cloud_preferences_syncer(toml_path_for_closure, None, ctx);
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        let initial_load = fake.snapshot_as_initial_load_response();
        app.update(|ctx| {
            update_manager.update(ctx, |um, ctx| {
                um.mock_initial_load(initial_load, ctx);
            });
        });

        drain_sync_queue(&mut app).await;

        // With hashes matching, force-local-wins should NOT trigger.
        // The syncer runs in normal ForceCloudToMatchLocal::No mode,
        // and the seeded cloud value (true) is applied to local.
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                settings.all_platforms_cloud_setting.inner,
                "cloud value should have been applied to local"
            );
        });

        // The cloud value should be unchanged.
        assert_eq!(
            Some("true".to_string()),
            fake.cloud_value("AllPlatforms", Platform::Global),
        );
    })
}

#[test]
fn test_force_local_suppressed_when_file_is_broken() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        // Create a file whose contents happen to hash to something,
        // seed a different stored hash so the comparison would
        // normally detect divergence, but also pass a non-None
        // `startup_toml_parse_error` to simulate "file is broken".
        // The broken-file guard should suppress force-local-wins.
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("settings.toml");
        write_settings_file_with_content(&toml_path, "broken [toml");
        write_stored_hash(&app, "0");

        // Local value set to true, cloud seeded with false. If the
        // broken-file guard fails and force-local triggers anyway,
        // the cloud would be overwritten with true.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .all_platforms_cloud_setting
                    .set_value(true, ctx);
            });
        });

        let fake = FakeObjectClient::default();
        fake.seed_preference("AllPlatforms", "false", Platform::Global);

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(fake.clone()));
        enable_settings_sync(&mut app);

        let toml_path_for_closure = toml_path.clone();
        app.add_singleton_model(move |ctx| {
            let syncer = initialize_cloud_preferences_syncer(
                toml_path_for_closure,
                Some("simulated TOML parse error"),
                ctx,
            );
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        let initial_load = fake.snapshot_as_initial_load_response();
        app.update(|ctx| {
            update_manager.update(ctx, |um, ctx| {
                um.mock_initial_load(initial_load, ctx);
            });
        });

        drain_sync_queue(&mut app).await;

        // The broken-file guard should have suppressed force-local.
        // Normal cloud-wins semantics restore the seeded cloud value
        // (false) into local state.
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                !settings.all_platforms_cloud_setting.inner,
                "cloud value should have been applied to local despite divergence",
            );
        });
        assert_eq!(
            Some("false".to_string()),
            fake.cloud_value("AllPlatforms", Platform::Global),
            "cloud value should not have been overwritten with local value",
        );
    })
}

#[test]
fn test_file_missing_with_stored_hash_lets_cloud_win() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        // Use a path that doesn't exist: the user has deleted their
        // settings.toml entirely. A stored hash is present (from a
        // previous session). The `(None, Some(_))` arm of the match
        // should choose cloud-wins to avoid wiping the cloud with
        // defaults.
        let tmp = tempfile::tempdir().unwrap();
        let missing_path = tmp.path().join("settings_does_not_exist.toml");
        assert!(!missing_path.exists());
        write_stored_hash(&app, "12345");

        let fake = FakeObjectClient::default();
        fake.seed_preference("AllPlatforms", "true", Platform::Global);

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(fake.clone()));
        enable_settings_sync(&mut app);

        app.add_singleton_model(move |ctx| {
            let syncer = initialize_cloud_preferences_syncer(missing_path, None, ctx);
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        let initial_load = fake.snapshot_as_initial_load_response();
        app.update(|ctx| {
            update_manager.update(ctx, |um, ctx| {
                um.mock_initial_load(initial_load, ctx);
            });
        });

        drain_sync_queue(&mut app).await;

        // Cloud should win, so local is updated to true.
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                settings.all_platforms_cloud_setting.inner,
                "cloud should win when file is missing"
            );
        });
        // Cloud state is preserved (not wiped by local defaults).
        assert_eq!(
            Some("true".to_string()),
            fake.cloud_value("AllPlatforms", Platform::Global),
        );
    })
}

#[test]
fn test_first_launch_with_no_stored_hash_lets_cloud_win() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        // Fresh install: a settings.toml exists but there is no
        // stored hash yet. The `(Some(_), None)` arm should choose
        // cloud-wins.
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("settings.toml");
        write_settings_file_with_content(&toml_path, "# fresh install\n");
        // Deliberately NO call to write_stored_hash.
        assert_eq!(None, read_stored_hash(&app));

        let fake = FakeObjectClient::default();
        fake.seed_preference("AllPlatforms", "true", Platform::Global);

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(fake.clone()));
        enable_settings_sync(&mut app);

        let toml_path_for_closure = toml_path.clone();
        app.add_singleton_model(move |ctx| {
            let syncer = initialize_cloud_preferences_syncer(toml_path_for_closure, None, ctx);
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        let initial_load = fake.snapshot_as_initial_load_response();
        app.update(|ctx| {
            update_manager.update(ctx, |um, ctx| {
                um.mock_initial_load(initial_load, ctx);
            });
        });

        drain_sync_queue(&mut app).await;

        // Cloud wins: local setting is updated to cloud's true.
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(
                settings.all_platforms_cloud_setting.inner,
                "cloud should win on first launch with no stored hash"
            );
        });

        // After the sync completes, the stored hash is populated so
        // future startups can detect divergence from this baseline.
        let stored_after_sync = read_stored_hash(&app);
        assert!(
            stored_after_sync.is_some(),
            "stored hash should be populated after first sync",
        );
    })
}

#[test]
fn test_offline_ui_change_does_not_update_hash_until_sync_succeeds() {
    App::test(ASSETS, |mut app| async move {
        initialize_settings(&mut app);

        // Phase 1: normal startup. File and stored hash match, cloud
        // has a value, initial load completes. After reconciliation the
        // stored hash reflects the file.
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("settings.toml");
        let initial_hash = write_settings_file_with_content(&toml_path, "# in sync with cloud\n");
        write_stored_hash(&app, &initial_hash);

        let fake = FakeObjectClient::default();
        fake.seed_preference("AllPlatforms", "false", Platform::Global);

        let UpdateManagerStruct { update_manager, .. } =
            create_update_manager_struct(&mut app, Arc::new(fake.clone()));
        enable_settings_sync(&mut app);

        let toml_path_for_closure = toml_path.clone();
        app.add_singleton_model(move |ctx| {
            let syncer = initialize_cloud_preferences_syncer(toml_path_for_closure, None, ctx);
            syncer.sync(ForceCloudToMatchLocal::No, ctx);
            syncer
        });

        let initial_load = fake.snapshot_as_initial_load_response();
        app.update(|ctx| {
            update_manager.update(ctx, |um, ctx| {
                um.mock_initial_load(initial_load, ctx);
            });
        });
        drain_sync_queue(&mut app).await;

        // Sanity: stored hash matches the initial file after the
        // initial load completes.
        let hash_after_initial_load = read_stored_hash(&app);
        assert_eq!(
            Some(initial_hash.clone()),
            hash_after_initial_load,
            "stored hash should match file after initial load",
        );

        // Phase 2: simulate an "offline UI change".
        //
        // Stop the SyncQueue from processing new items. This models
        // the device being offline: the syncer will enqueue uploads
        // but the queue won't actually send them to the server.
        SyncQueue::handle(&app).update(&mut app, |sq, _| sq.stop_dequeueing());

        // Write new file content (simulates what the TOML backend
        // does when the setting model calls write_value).
        let new_hash = write_settings_file_with_content(
            &toml_path,
            "# user changed a setting offline\nall_platforms = true\n",
        );
        assert_ne!(initial_hash, new_hash);

        // Change the in-memory setting. In the #[cfg(test)] path this
        // synchronously calls maybe_sync_local_prefs_to_cloud, which
        // enqueues an upload to the SyncQueue — but the queue is
        // stopped, so the upload won't be processed.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .all_platforms_cloud_setting
                    .set_value(true, ctx);
            });
        });

        // Give the syncer a moment to finish handling the setting
        // change event.
        warpui::r#async::Timer::after(Duration::from_millis(200)).await;

        // CRITICAL ASSERTION: the stored hash must NOT have been
        // updated. The upload is enqueued but the SyncQueue is
        // stopped, so no ObjectUpdateSuccessful event fires and the
        // hash stays at the pre-change value. This is the bug that
        // was fixed — previously the hash was updated synchronously
        // in maybe_sync_local_prefs_to_cloud, which caused the next
        // startup to think the file was in sync with cloud.
        assert_eq!(
            Some(initial_hash),
            read_stored_hash(&app),
            "stored hash must NOT update before the upload succeeds",
        );

        // Verify that a hypothetical restart right now would detect
        // divergence: current file hash != stored hash.
        assert_ne!(
            Some(new_hash.clone()),
            read_stored_hash(&app),
            "file hash should differ from stored hash while upload is pending",
        );

        // Phase 3: "come back online" — restart the sync queue so
        // enqueued items get processed.
        SyncQueue::handle(&app).update(&mut app, |sq, ctx| sq.start_dequeueing(ctx));
        drain_sync_queue(&mut app).await;

        // Now the stored hash should be updated to match the new file,
        // because the SyncQueue success event fired and the syncer's
        // subscription updated the hash.
        assert_eq!(
            Some(new_hash),
            read_stored_hash(&app),
            "stored hash should match new file after successful upload",
        );
    })
}
