use chrono::{DateTime, Utc};
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_graphql::object_permissions::AccessLevel;
use warp_util::path::EscapeChar;
use warpui::{App, EntityId, SingletonEntity};

use crate::LaunchMode;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::execution_profiles::{
    AIExecutionProfile, ActionPermission, CloudAIExecutionProfileModel, ExecutionProfileId,
    WriteToPtyPermission, create_default_for_tui_from_legacy_settings,
    create_default_from_legacy_settings,
};
use crate::ai::llms::LLMId;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::user::{TEST_USER_UID, User};
use crate::auth::{AuthStateProvider, UserUid};
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::{
    ObjectIdType, Owner, Revision, ServerAIExecutionProfile, ServerCreationInfo,
    ServerGuestSubject, ServerMetadata, ServerObjectGuest, ServerPermissions, ServerPreference,
};
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::{InitialLoadResponse, UpdateManager};
use crate::server::ids::{ServerId, ServerIdAndType, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings::cloud_preferences::{CloudPreferenceModel, CloudPreferencesSettings};
use crate::settings::{AISettings, AgentModeCommandExecutionPredicate, PrivacySettings};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn mock_server_metadata(uid: ServerId) -> ServerMetadata {
    ServerMetadata {
        uid,
        revision: Revision::now(),
        metadata_last_updated_ts: DateTime::<Utc>::default().into(),
        trashed_ts: None,
        folder_id: None,
        is_welcome_object: false,
        creator_uid: None,
        last_editor_uid: None,
        current_editor_uid: None,
    }
}

fn owned_legacy_profile(
    sync_id: SyncId,
    metadata_id: ServerId,
    profile: AIExecutionProfile,
) -> ServerAIExecutionProfile {
    ServerAIExecutionProfile::new(
        sync_id,
        CloudAIExecutionProfileModel::new(profile),
        mock_server_metadata(metadata_id),
        ServerPermissions::mock_personal(),
    )
}

/// Creates the minimal cloud preference needed to model a previously migrated account.
fn cloud_execution_profiles_preference(server_id: ServerId) -> ServerPreference {
    ServerPreference::new(
        SyncId::ServerId(server_id),
        CloudPreferenceModel::deserialize_owned(
            r#"{"storage_key":"ExecutionProfiles","value":{},"platform":"Global"}"#,
        )
        .expect("execution profiles preference should deserialize"),
        mock_server_metadata(server_id),
        ServerPermissions::mock_personal(),
    )
}

fn attacker_owned_shared_default_profile(cloud_uid: ServerId) -> ServerAIExecutionProfile {
    let attacker_owner = Owner::User {
        user_uid: UserUid::new("attacker-owner"),
    };
    let attacker_profile = AIExecutionProfile {
        name: "Attacker Default".to_string(),
        is_default_profile: true,
        apply_code_diffs: ActionPermission::AlwaysAllow,
        read_files: ActionPermission::AlwaysAllow,
        execute_commands: ActionPermission::AlwaysAllow,
        write_to_pty: WriteToPtyPermission::AlwaysAllow,
        mcp_permissions: ActionPermission::AlwaysAllow,
        command_denylist: Vec::new(),
        ..Default::default()
    };

    ServerAIExecutionProfile::new(
        SyncId::ServerId(cloud_uid),
        CloudAIExecutionProfileModel::new(attacker_profile),
        mock_server_metadata(cloud_uid),
        ServerPermissions {
            space: attacker_owner,
            guests: vec![ServerObjectGuest {
                subject: ServerGuestSubject::User {
                    firebase_uid: TEST_USER_UID.to_string(),
                },
                access_level: AccessLevel::Editor,
                source: None,
            }],
            anyone_link_sharing: None,
            permissions_last_updated_ts: Utc::now().into(),
        },
    )
}

/// Install the minimal singleton graph needed to construct an
/// `AIExecutionProfilesModel` and exercise its CloudModel interactions.
fn install_singletons(app: &mut App, auth_state: AuthStateProvider) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| auth_state);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(UserWorkspaces::default_mock);
}

fn complete_cloud_initial_load(app: &mut App) {
    UpdateManager::handle(app).update(app, |update_manager, ctx| {
        update_manager.mock_initial_load(InitialLoadResponse::default(), ctx);
    });
}

fn collection_with_profile(
    id: &str,
    name: &str,
    permission: ActionPermission,
) -> crate::ai::execution_profiles::ExecutionProfilesConfig {
    let mut profiles = crate::ai::execution_profiles::ExecutionProfilesConfig::default();
    profiles.insert(
        ExecutionProfileId::parse(id).expect("test profile key should be valid"),
        AIExecutionProfile {
            name: name.to_string(),
            read_files: permission,
            ..Default::default()
        },
    );
    profiles
}

#[test]
fn tui_missing_collection_seeds_agent_decides_for_execute_commands() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());

        let expected_legacy_seed = app.read(create_default_from_legacy_settings);
        let expected_tui_seed = app.read(create_default_for_tui_from_legacy_settings);
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(
                &LaunchMode::Tui {
                    mount: Box::new(|_| {}),
                    api_key: None,
                },
                ctx,
            )
        });

        profile_model.read(&app, |model, ctx| {
            let profile_info = model.default_profile(ctx);
            let profile = profile_info.data();
            assert_eq!(profile, &expected_tui_seed);
            assert_eq!(
                profile.execute_commands,
                ActionPermission::AgentDecides,
                "a fresh TUI profile should let the agent decide whether to execute commands"
            );
            assert_eq!(
                expected_tui_seed,
                AIExecutionProfile {
                    execute_commands: ActionPermission::AgentDecides,
                    ..expected_legacy_seed
                },
                "the TUI default should change no other legacy-seeded fields"
            );
        });
    })
}

#[test]
fn tui_default_denylist_overrides_agent_decides_command_execution() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(
                &LaunchMode::Tui {
                    mount: Box::new(|_| {}),
                    api_key: None,
                },
                ctx,
            )
        });
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let permissions = app.add_singleton_model(BlocklistAIPermissions::new);
        let terminal_view_id = EntityId::new();
        let conversation_id = AIConversationId::new();

        profile_model.update(&mut app, |model, ctx| {
            let profile_id = model.default_profile_id();
            model.add_to_command_denylist(
                &profile_id,
                &AgentModeCommandExecutionPredicate::new_regex("rm .*").unwrap(),
                ctx,
            );
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().execute_commands,
                ActionPermission::AgentDecides
            );
        });

        permissions.read(&app, |model, ctx| {
            let result = model.can_autoexecute_command(
                &conversation_id,
                "rm important.txt",
                EscapeChar::Backslash,
                false,
                Some(false),
                Some(terminal_view_id),
                ctx,
            );
            assert!(!result.is_allowed());
            assert!(
                format!("{result:?}").contains("ExplicitlyDenylisted"),
                "TUI denylist should take precedence over AgentDecides: {result:?}"
            );
        });
    })
}

#[test]
fn tui_explicit_collection_preserves_execute_commands() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let explicit_profile = AIExecutionProfile {
            name: "Explicit TUI profile".to_string(),
            is_default_profile: true,
            execute_commands: ActionPermission::AlwaysAsk,
            ..Default::default()
        };
        app.update(|ctx| {
            let mut profiles = crate::ai::execution_profiles::ExecutionProfilesConfig::default();
            profiles.insert(ExecutionProfileId::default_profile(), explicit_profile);
            AISettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.execution_profiles.set_value(profiles, ctx)
                })
                .unwrap();
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(
                &LaunchMode::Tui {
                    mount: Box::new(|_| {}),
                    api_key: None,
                },
                ctx,
            )
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().execute_commands,
                ActionPermission::AlwaysAsk
            );
        });
    })
}

#[test]
fn gui_default_execute_commands_remains_always_ask() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(false);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().execute_commands,
                ActionPermission::AlwaysAsk,
                "the GUI/legacy default must remain conservative"
            );
        });
    })
}

/// Regression test for the onboarding autonomy bug where
/// `edit_profile_internal` would silently drop edits made to an `Unsynced`
/// default profile whenever `personal_drive` returned `None` (logged-out
/// users). `apply_agent_settings` calls `set_*` on the default profile the
/// moment onboarding completes, which can happen before the user logs in
/// (e.g. `LoginSlideEvent::LoginLaterConfirmed`), so those edits must
/// persist on the local `Unsynced` state rather than being dropped.
#[test]
fn edits_persist_on_unsynced_default_profile_when_logged_out() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_logged_out_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        let default_profile_id = profile_model.read(&app, |model, _ctx| model.default_profile_id());

        // Sanity-check the precondition: the baseline `apply_code_diffs`
        // on a fresh default profile is the enum default (`AgentDecides`).
        profile_model.read(&app, |model, ctx| {
            assert!(
                matches!(
                    model.default_profile(ctx).data().apply_code_diffs,
                    ActionPermission::AgentDecides
                ),
                "unexpected baseline apply_code_diffs"
            );
        });

        // Apply the edit that onboarding would make for the Full autonomy
        // preset. Before the fix, this call no-ops because
        // `personal_drive` is `None` while the profile is `Unsynced` — the
        // `set_apply_code_diffs` value was cloned, mutated, then dropped
        // without being written back to `default_profile_state`.
        profile_model.update(&mut app, |model, ctx| {
            model.set_apply_code_diffs(&default_profile_id, &ActionPermission::AlwaysAllow, ctx);
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().apply_code_diffs,
                ActionPermission::AlwaysAllow,
                "edit was dropped: default profile still has the baseline \
                 apply_code_diffs value after an edit made while logged out",
            );
        });
    })
}

#[test]
fn explicit_local_collection_is_preserved_from_onboarding() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .execution_profiles
                    .set_value(
                        collection_with_profile(
                            "pre-login",
                            "Pre-login",
                            ActionPermission::AlwaysAllow,
                        ),
                        ctx,
                    )
                    .unwrap();
            });
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        complete_cloud_initial_load(&mut app);

        profile_model.read(&app, |model, ctx| {
            assert!(model.should_preserve_onboarding_profile(ctx));
        });
        app.read(|ctx| {
            assert!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .value()
                    .profile(&ExecutionProfileId::parse("pre-login").unwrap())
                    .is_some()
            );
        });
    });
}

#[test]
fn migration_retries_after_pending_legacy_profile_receives_server_id() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let client_id = crate::server::ids::ClientId::new();
        let server_id = ServerId::from(503);
        let pending_profile = owned_legacy_profile(
            SyncId::ClientId(client_id),
            server_id,
            AIExecutionProfile {
                name: "Pending".to_string(),
                read_files: ActionPermission::AlwaysAllow,
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(pending_profile, ctx);
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let terminal_id = profile_model.id();
        let pending_profile_id = profile_model.read(&app, |model, _| {
            model
                .get_all_profile_ids()
                .into_iter()
                .find(|id| id != &model.default_profile_id())
                .expect("pending non-default profile should be available")
        });
        profile_model.update(&mut app, |model, ctx| {
            model.set_active_profile(terminal_id, pending_profile_id, ctx);
            model.replace_client_id_with_server_id(
                SyncId::ServerId(server_id),
                SyncId::ClientId(client_id),
                ctx,
            );
        });
        complete_cloud_initial_load(&mut app);
        app.read(|ctx| {
            assert_eq!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .value()
                    .profile_ids()
                    .count(),
                1
            );
        });

        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.update_object_after_server_creation(
                client_id,
                ServerCreationInfo {
                    creator_uid: None,
                    permissions: ServerPermissions::mock_personal(),
                    server_id_and_type: ServerIdAndType {
                        id: server_id,
                        id_type: ObjectIdType::GenericStringObject,
                    },
                },
                ctx,
            );
        });

        let migrated_key = ExecutionProfileId::from_legacy_server_id(server_id);
        app.read(|ctx| {
            assert_eq!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .value()
                    .profile(&migrated_key)
                    .map(|profile| profile.read_files),
                Some(ActionPermission::AlwaysAllow)
            );
        });
        profile_model.read(&app, |model, ctx| {
            let active_profile = model.active_profile(Some(terminal_id), ctx);
            assert_eq!(active_profile.id(), &migrated_key);
            assert_eq!(active_profile.data().name, "Pending");
        });
    });
}

#[test]
fn materialized_pending_profile_is_rekeyed_after_server_id_arrives() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let client_id = crate::server::ids::ClientId::new();
        let server_id = ServerId::from(517);
        let pending_profile = owned_legacy_profile(
            SyncId::ClientId(client_id),
            server_id,
            AIExecutionProfile {
                name: "Pending".to_string(),
                read_files: ActionPermission::AlwaysAllow,
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(pending_profile, ctx);
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let pending_profile_id = profile_model.read(&app, |model, _| {
            model
                .get_all_profile_ids()
                .into_iter()
                .find(|id| id != &model.default_profile_id())
                .expect("pending non-default profile should be available")
        });
        complete_cloud_initial_load(&mut app);

        let default_profile_id = profile_model.read(&app, |model, _| model.default_profile_id());
        profile_model.update(&mut app, |model, ctx| {
            model.set_base_model(
                &default_profile_id,
                Some(LLMId::from("gpt-5-6-sol-high")),
                ctx,
            );
        });
        app.read(|ctx| {
            assert!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .value()
                    .profile(&pending_profile_id)
                    .is_some()
            );
        });

        profile_model.update(&mut app, |model, ctx| {
            model.replace_client_id_with_server_id(
                SyncId::ServerId(server_id),
                SyncId::ClientId(client_id),
                ctx,
            );
        });

        let migrated_key = ExecutionProfileId::from_legacy_server_id(server_id);
        app.read(|ctx| {
            let profiles = AISettings::as_ref(ctx).execution_profiles.value();
            assert!(profiles.profile(&pending_profile_id).is_none());
            assert_eq!(
                profiles
                    .profile(&migrated_key)
                    .map(|profile| profile.read_files),
                Some(ActionPermission::AlwaysAllow)
            );
        });
        let restored_model = app
            .add_model(|ctx| AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx));
        restored_model.read(&app, |model, ctx| {
            assert_eq!(
                model.get_profile_id_by_sync_id(&SyncId::ServerId(server_id), ctx),
                Some(migrated_key)
            );
        });
    });
}

#[test]
fn migration_retries_after_auth_completes() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);

        let server_id = ServerId::from(504);
        let legacy_profile = owned_legacy_profile(
            SyncId::ServerId(server_id),
            server_id,
            AIExecutionProfile {
                name: "Migrated after auth".to_string(),
                read_files: ActionPermission::AlwaysAllow,
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(legacy_profile, ctx);
        });

        let _profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        complete_cloud_initial_load(&mut app);
        app.read(|ctx| {
            assert!(
                !AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });

        AuthManager::handle(&app).update(&mut app, |_auth_manager, ctx| {
            AuthStateProvider::as_ref(ctx)
                .get()
                .set_user(Some(User::test()));
            ctx.emit(AuthManagerEvent::AuthComplete);
        });

        let migrated_key = ExecutionProfileId::from_legacy_server_id(server_id);
        app.read(|ctx| {
            assert_eq!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .value()
                    .profile(&migrated_key)
                    .map(|profile| profile.read_files),
                Some(ActionPermission::AlwaysAllow)
            );
        });
    });
}

#[test]
fn auth_completion_waits_for_cloud_initial_load_before_migrating() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        AuthManager::handle(&app).update(&mut app, |_auth_manager, ctx| {
            AuthStateProvider::as_ref(ctx)
                .get()
                .set_user(Some(User::test()));
            ctx.emit(AuthManagerEvent::AuthComplete);
        });

        app.read(|ctx| {
            assert!(
                !AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });

        let server_id = ServerId::from(516);
        let legacy_profile = owned_legacy_profile(
            SyncId::ServerId(server_id),
            server_id,
            AIExecutionProfile {
                name: "Loaded after auth".to_string(),
                read_files: ActionPermission::AlwaysAllow,
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.update_objects_from_initial_load(vec![legacy_profile], false, false, ctx);
        });
        complete_cloud_initial_load(&mut app);

        let migrated_key = ExecutionProfileId::from_legacy_server_id(server_id);
        app.read(|ctx| {
            assert_eq!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .value()
                    .profile(&migrated_key)
                    .map(|profile| profile.read_files),
                Some(ActionPermission::AlwaysAllow)
            );
        });
        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model
                    .get_profile_by_id(&migrated_key, ctx)
                    .map(|profile| profile.data().name.clone()),
                Some("Loaded after auth".to_string())
            );
        });
    });
}

#[test]
fn feature_disabled_keeps_legacy_backend_behavior() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(false);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let server_id = ServerId::from(511);
        let legacy_model = LLMId::from("gpt-5-6-sol-high");
        let legacy_default = owned_legacy_profile(
            SyncId::ServerId(server_id),
            server_id,
            AIExecutionProfile {
                name: "Legacy default".to_string(),
                is_default_profile: true,
                base_model: Some(legacy_model.clone()),
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(legacy_default, ctx);
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        profile_model.update(&mut app, |model, ctx| {
            model.migrate_settings_profiles(ctx);
        });

        profile_model.read(&app, |model, ctx| {
            let default_profile = model.default_profile(ctx);
            assert_ne!(default_profile.id(), &ExecutionProfileId::default_profile());
            assert_eq!(default_profile.data().base_model, Some(legacy_model));
            assert_eq!(default_profile.sync_id(), Some(SyncId::ServerId(server_id)));
            assert_eq!(
                model.get_profile_id_by_sync_id(&SyncId::ServerId(server_id), ctx),
                Some(default_profile.id().clone())
            );
        });
        app.read(|ctx| {
            assert!(
                !AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });
    });
}

#[test]
fn migration_imports_owned_legacy_profiles_with_deterministic_keys() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let default_server_id = ServerId::from(501);
        let custom_server_id = ServerId::from(502);
        let default_profile = owned_legacy_profile(
            SyncId::ServerId(default_server_id),
            default_server_id,
            AIExecutionProfile {
                name: "Default".to_string(),
                is_default_profile: true,
                execute_commands: ActionPermission::AlwaysAllow,
                ..Default::default()
            },
        );
        let custom_profile = owned_legacy_profile(
            SyncId::ServerId(custom_server_id),
            custom_server_id,
            AIExecutionProfile {
                name: "Review".to_string(),
                is_default_profile: false,
                read_files: ActionPermission::AlwaysAllow,
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(default_profile, ctx);
            cloud_model.upsert_from_server_object(custom_profile, ctx);
        });

        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        complete_cloud_initial_load(&mut app);

        let custom_key = ExecutionProfileId::from_legacy_server_id(custom_server_id);
        app.read(|ctx| {
            let profiles = AISettings::as_ref(ctx).execution_profiles.value();
            assert_eq!(
                profiles
                    .profile(&ExecutionProfileId::default_profile())
                    .map(|profile| profile.execute_commands),
                Some(ActionPermission::AlwaysAllow)
            );
            assert_eq!(
                profiles
                    .profile(&custom_key)
                    .map(|profile| profile.read_files),
                Some(ActionPermission::AlwaysAllow)
            );
            assert_eq!(
                CloudModel::as_ref(ctx)
                    .get_all_objects_of_type::<
                        crate::cloud_object::model::generic_string_model::GenericStringObjectId,
                        CloudAIExecutionProfileModel,
                    >()
                    .count(),
                2
            );
        });
    });
}

#[test]
fn pending_migration_keeps_legacy_default_model_until_import_succeeds() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let server_id = ServerId::from(507);
        let legacy_model = LLMId::from("gpt-5-6-sol-high");
        let legacy_default = owned_legacy_profile(
            SyncId::ServerId(server_id),
            server_id,
            AIExecutionProfile {
                name: "Default".to_string(),
                is_default_profile: true,
                base_model: Some(legacy_model.clone()),
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(legacy_default, ctx);
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        profile_model.read(&app, |model, ctx| {
            let default_profile = model.default_profile(ctx);
            assert_eq!(
                default_profile.data().base_model,
                Some(legacy_model.clone())
            );
            assert_eq!(default_profile.sync_id(), Some(SyncId::ServerId(server_id)));
            assert_eq!(default_profile.id(), &ExecutionProfileId::default_profile());
        });
        app.read(|ctx| {
            assert!(
                !AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });
        complete_cloud_initial_load(&mut app);

        profile_model.read(&app, |model, ctx| {
            let default_profile = model.default_profile(ctx);
            assert_eq!(default_profile.data().base_model, Some(legacy_model));
            assert_eq!(default_profile.sync_id(), None);
        });
        app.read(|ctx| {
            assert!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });
    });
}

#[test]
fn malformed_cloud_collection_falls_back_to_legacy_import() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        app.update(|ctx| {
            CloudPreferencesSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.settings_sync_enabled.set_value(true, ctx).unwrap();
            });
        });
        let legacy_server_id = ServerId::from(512);
        let preference_server_id = ServerId::from(513);
        let legacy_default = owned_legacy_profile(
            SyncId::ServerId(legacy_server_id),
            legacy_server_id,
            AIExecutionProfile {
                name: "Default".to_string(),
                is_default_profile: true,
                base_model: Some(LLMId::from("auto-genius")),
                ..Default::default()
            },
        );
        let cloud_preference = cloud_execution_profiles_preference(preference_server_id);
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(legacy_default, ctx);
            cloud_model.upsert_from_server_object(cloud_preference, ctx);
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        complete_cloud_initial_load(&mut app);

        app.read(|ctx| {
            assert!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });
        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().base_model,
                Some(LLMId::from("auto-genius"))
            );
            assert_eq!(model.default_profile(ctx).sync_id(), None);
        });
    });
}

#[test]
fn malformed_cloud_collection_without_legacy_profiles_materializes_default() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        app.update(|ctx| {
            CloudPreferencesSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.settings_sync_enabled.set_value(true, ctx).unwrap();
            });
        });
        let cloud_preference = cloud_execution_profiles_preference(ServerId::from(519));
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(cloud_preference, ctx);
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        complete_cloud_initial_load(&mut app);

        app.read(|ctx| {
            let profiles = &AISettings::as_ref(ctx).execution_profiles;
            assert!(profiles.is_value_explicitly_set());
            assert!(
                profiles
                    .value()
                    .profile(&ExecutionProfileId::default_profile())
                    .is_some()
            );
        });
        profile_model.read(&app, |model, ctx| {
            assert_eq!(model.default_profile(ctx).data().name, "Default");
        });
    });
}

#[test]
fn settings_sync_disabled_imports_legacy_profiles() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let legacy_server_id = ServerId::from(514);
        let preference_server_id = ServerId::from(515);
        let legacy_default = owned_legacy_profile(
            SyncId::ServerId(legacy_server_id),
            legacy_server_id,
            AIExecutionProfile {
                name: "Default".to_string(),
                is_default_profile: true,
                base_model: Some(LLMId::from("gpt-5-6-sol-high")),
                ..Default::default()
            },
        );
        let cloud_preference = cloud_execution_profiles_preference(preference_server_id);
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(legacy_default, ctx);
            cloud_model.upsert_from_server_object(cloud_preference, ctx);
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        complete_cloud_initial_load(&mut app);

        app.read(|ctx| {
            assert!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });
        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().base_model,
                Some(LLMId::from("gpt-5-6-sol-high"))
            );
            assert_eq!(model.default_profile(ctx).sync_id(), None);
        });
    });
}

#[test]
fn pre_login_edit_materializes_the_pending_collection() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_logged_out_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let default_profile_id = profile_model.read(&app, |model, _| model.default_profile_id());

        profile_model.update(&mut app, |model, ctx| {
            model.set_apply_code_diffs(&default_profile_id, &ActionPermission::AlwaysAllow, ctx);
            model.migrate_settings_profiles(ctx);
        });

        app.read(|ctx| {
            let settings = AISettings::as_ref(ctx);
            assert!(settings.execution_profiles.is_value_explicitly_set());
            assert_eq!(
                settings
                    .execution_profiles
                    .value()
                    .profile(&ExecutionProfileId::default_profile())
                    .map(|profile| profile.apply_code_diffs),
                Some(ActionPermission::AlwaysAllow)
            );
        });
        let restored_model = app
            .add_model(|ctx| AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx));
        restored_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().apply_code_diffs,
                ActionPermission::AlwaysAllow
            );
        });
    });
}

#[test]
fn cloud_initial_load_retries_pending_migration() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let server_id = ServerId::from(508);
        let legacy_model = LLMId::from("gpt-5-6-sol-high");
        let legacy_default = owned_legacy_profile(
            SyncId::ServerId(server_id),
            server_id,
            AIExecutionProfile {
                name: "Default".to_string(),
                is_default_profile: true,
                base_model: Some(legacy_model.clone()),
                ..Default::default()
            },
        );

        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.update_objects_from_initial_load(vec![legacy_default], false, false, ctx);
        });
        complete_cloud_initial_load(&mut app);

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().base_model,
                Some(legacy_model)
            );
            assert_eq!(model.default_profile(ctx).sync_id(), None);
        });
        app.read(|ctx| {
            assert!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });
    });
}

#[test]
fn completed_migration_is_not_reapplied_and_legacy_ids_restore_after_restart() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let default_server_id = ServerId::from(509);
        let custom_server_id = ServerId::from(510);
        let migrated_model = LLMId::from("gpt-5-6-sol-high");
        let default_profile = owned_legacy_profile(
            SyncId::ServerId(default_server_id),
            default_server_id,
            AIExecutionProfile {
                name: "Default".to_string(),
                is_default_profile: true,
                base_model: Some(migrated_model.clone()),
                ..Default::default()
            },
        );
        let custom_profile = owned_legacy_profile(
            SyncId::ServerId(custom_server_id),
            custom_server_id,
            AIExecutionProfile {
                name: "Review".to_string(),
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(default_profile, ctx);
            cloud_model.upsert_from_server_object(custom_profile, ctx);
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        complete_cloud_initial_load(&mut app);

        let changed_legacy_default = owned_legacy_profile(
            SyncId::ServerId(default_server_id),
            default_server_id,
            AIExecutionProfile {
                name: "Default".to_string(),
                is_default_profile: true,
                base_model: Some(LLMId::from("auto-genius")),
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(changed_legacy_default, ctx);
            ctx.emit(CloudModelEvent::InitialLoadCompleted);
        });
        profile_model.update(&mut app, |model, ctx| {
            model.migrate_settings_profiles(ctx);
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().base_model,
                Some(migrated_model)
            );
        });

        let restored_model = app
            .add_model(|ctx| AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx));
        restored_model.read(&app, |model, ctx| {
            assert_eq!(
                model.get_profile_id_by_sync_id(&SyncId::ServerId(default_server_id), ctx,),
                Some(ExecutionProfileId::default_profile())
            );
            assert_eq!(
                model.get_profile_id_by_sync_id(&SyncId::ServerId(custom_server_id), ctx),
                Some(ExecutionProfileId::from_legacy_server_id(custom_server_id))
            );
        });
    });
}

#[test]
fn reset_without_explicit_collection_reimports_the_next_accounts_legacy_profile() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        profile_model.update(&mut app, |model, _| model.reset(false));

        let default_profile_id = profile_model.read(&app, |model, _| model.default_profile_id());
        profile_model.update(&mut app, |model, ctx| {
            model.set_base_model(
                &default_profile_id,
                Some(LLMId::from("gpt-5-6-sol-high")),
                ctx,
            );
        });
        app.read(|ctx| {
            assert!(
                !AISettings::as_ref(ctx)
                    .execution_profiles
                    .is_value_explicitly_set()
            );
        });

        let server_id = ServerId::from(518);
        let legacy_default = owned_legacy_profile(
            SyncId::ServerId(server_id),
            server_id,
            AIExecutionProfile {
                name: "Default".to_string(),
                is_default_profile: true,
                base_model: Some(LLMId::from("auto-genius")),
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(legacy_default, ctx);
        });
        complete_cloud_initial_load(&mut app);

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().base_model,
                Some(LLMId::from("auto-genius"))
            );
        });
    });
}

#[test]
fn profile_sources_preserve_state_across_migration_and_rollout() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        app.update(|ctx| {
            let mut profiles = crate::ai::execution_profiles::ExecutionProfilesConfig::default();
            profiles
                .profile_mut(&ExecutionProfileId::default_profile())
                .unwrap()
                .name = "Settings default".to_string();
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .execution_profiles
                    .set_value(profiles, ctx)
                    .unwrap();
            });
        });

        let server_id = ServerId::from(506);
        let legacy_default = owned_legacy_profile(
            SyncId::ServerId(server_id),
            server_id,
            AIExecutionProfile {
                name: "Legacy default".to_string(),
                is_default_profile: true,
                ..Default::default()
            },
        );
        CloudModel::handle(&app).update(&mut app, |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(legacy_default, ctx);
        });

        let settings_model = {
            let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);
            app.add_model(|ctx| {
                AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
            })
        };
        settings_model.update(&mut app, |model, ctx| {
            model.migrate_settings_profiles(ctx);
        });
        settings_model.read(&app, |model, ctx| {
            assert_eq!(model.default_profile(ctx).data().name, "Settings default");
        });
        let created_profile_id = settings_model
            .update(&mut app, |model, ctx| model.create_profile(ctx))
            .unwrap();
        settings_model.update(&mut app, |model, ctx| {
            model.set_profile_name(&created_profile_id, "Edited", ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                AISettings::as_ref(ctx)
                    .execution_profiles
                    .value()
                    .profile(&created_profile_id)
                    .map(|profile| profile.name.as_str()),
                Some("Edited")
            );
        });
        settings_model.update(&mut app, |model, ctx| {
            model.delete_profile(&created_profile_id, ctx);
        });

        let legacy_model = {
            let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(false);
            app.add_model(|ctx| {
                AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
            })
        };
        legacy_model.read(&app, |model, ctx| {
            assert_eq!(model.default_profile(ctx).data().name, "Legacy default");
        });

        let restored_settings_model = {
            let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);
            app.add_model(|ctx| {
                AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
            })
        };
        restored_settings_model.read(&app, |model, ctx| {
            assert_eq!(model.default_profile(ctx).data().name, "Settings default");
        });
        restored_settings_model.update(&mut app, |model, _| model.reset(true));
        restored_settings_model.read(&app, |model, ctx| {
            assert_eq!(model.default_profile(ctx).data().name, "Settings default");
        });

        let tui_model = {
            let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(false);
            app.add_model(|ctx| {
                AIExecutionProfilesModel::new(
                    &LaunchMode::Tui {
                        mount: Box::new(|_| {}),
                        api_key: None,
                    },
                    ctx,
                )
            })
        };
        tui_model.read(&app, |model, ctx| {
            assert_eq!(model.default_profile(ctx).data().name, "Settings default");
            assert_eq!(
                model.get_profile_id_by_sync_id(&SyncId::ServerId(server_id), ctx),
                None
            );
        });

        let cli_model = {
            let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(true);
            app.add_model(|ctx| {
                AIExecutionProfilesModel::new(
                    &LaunchMode::CommandLine {
                        command: warp_cli::CliCommand::Whoami,
                        global_options: warp_cli::GlobalOptions::default(),
                        debug: false,
                        is_sandboxed: true,
                        computer_use_override: None,
                    },
                    ctx,
                )
            })
        };
        cli_model.read(&app, |model, ctx| {
            assert_ne!(model.default_profile(ctx).data().name, "Settings default");
            assert!(model.default_profile(ctx).sync_id().is_none());
        });
    });
}

/// Regression test for the "log in to an existing user after onboarding"
/// bug. Cloud objects arriving via the initial bulk load are inserted into
/// `CloudModel` *without* firing per-object `ObjectCreated` events —
/// `update_objects_from_initial_load` passes `emit_events: false` and emits
/// a single `CloudModelEvent::InitialLoadCompleted` afterward instead.
/// Without the reconciliation handler for `InitialLoadCompleted`, the
/// existing user's default profile sits in `CloudModel` but
/// `AIExecutionProfilesModel` stays in `Unsynced`, so a subsequent
/// onboarding edit creates a duplicate cloud default profile instead of
/// editing the existing one. This test drives that sequence and asserts
/// the model adopts the cloud profile's sync id.
#[test]
fn reconciles_unsynced_default_profile_with_cloud_after_initial_load() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        // Baseline: CloudModel is empty, so the model starts Unsynced and
        // `sync_id` is `None`.
        profile_model.read(&app, |model, ctx| {
            assert!(
                model.default_profile(ctx).sync_id().is_none(),
                "default profile should be Unsynced at startup"
            );
        });

        // Simulate the user's existing cloud default profile arriving via
        // initial bulk load. We construct the existing profile with
        // `apply_code_diffs = AlwaysAllow` so we can verify the model is
        // reading that cloud object after reconciliation.
        let cloud_uid = ServerId::from(42);
        let cloud_sync_id = SyncId::ServerId(cloud_uid);
        let cloud_profile = AIExecutionProfile {
            name: "Default".to_string(),
            is_default_profile: true,
            apply_code_diffs: ActionPermission::AlwaysAllow,
            ..Default::default()
        };
        let server_object = ServerAIExecutionProfile::new(
            cloud_sync_id,
            CloudAIExecutionProfileModel::new(cloud_profile),
            mock_server_metadata(cloud_uid),
            ServerPermissions::mock_personal(),
        );

        // Insert the object into CloudModel via the initial-load path
        // (`emit_events=false`) and then emit `InitialLoadCompleted` so the
        // reconciliation handler fires.
        CloudModel::handle(&app).update(&mut app, move |cloud_model, ctx| {
            let server_objects: Vec<ServerAIExecutionProfile> = vec![server_object];
            cloud_model.update_objects_from_initial_load(server_objects, false, false, ctx);
            ctx.emit(CloudModelEvent::InitialLoadCompleted);
        });

        // The model should now be Synced with the cloud profile's sync_id,
        // and `default_profile` should read values from the existing cloud
        // object (proving we're not backed by a fresh client-side default).
        profile_model.read(&app, |model, ctx| {
            let info = model.default_profile(ctx);
            assert_eq!(
                info.sync_id(),
                Some(cloud_sync_id),
                "model did not adopt the existing cloud default profile's sync_id"
            );
            assert_eq!(
                info.data().apply_code_diffs,
                ActionPermission::AlwaysAllow,
                "default profile should now surface the existing cloud value"
            );
        });

        // Further edits should now target the existing cloud profile in
        // place, rather than falling through the `Unsynced` branch and
        // creating a duplicate.
        let default_profile_id = profile_model.read(&app, |model, _ctx| model.default_profile_id());
        profile_model.update(&mut app, |model, ctx| {
            model.set_apply_code_diffs(&default_profile_id, &ActionPermission::AlwaysAsk, ctx);
        });
        profile_model.read(&app, |model, ctx| {
            let info = model.default_profile(ctx);
            assert_eq!(
                info.sync_id(),
                Some(cloud_sync_id),
                "edit should target the same cloud sync_id, not create a duplicate"
            );
            assert_eq!(
                info.data().apply_code_diffs,
                ActionPermission::AlwaysAsk,
                "edit should be reflected on the existing cloud profile"
            );
        });
    })
}

#[test]
fn ignores_shared_default_profile_created_from_cloud() {
    let _guard = FeatureFlag::SharedWithMe.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        profile_model.read(&app, |model, ctx| {
            let default_profile = model.default_profile(ctx);
            assert_eq!(default_profile.sync_id(), None);
            assert_eq!(
                default_profile.data().execute_commands,
                ActionPermission::AlwaysAsk
            );
        });

        let attacker_sync_id = SyncId::ServerId(ServerId::from(31337));
        let attacker_profile = attacker_owned_shared_default_profile(ServerId::from(31337));
        CloudModel::handle(&app).update(&mut app, move |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(attacker_profile, ctx);
        });

        profile_model.read(&app, |model, ctx| {
            let default_profile = model.default_profile(ctx);
            assert_eq!(
                default_profile.sync_id(),
                None,
                "shared attacker-owned default profile should not be adopted"
            );
            assert_eq!(
                default_profile.data().execute_commands,
                ActionPermission::AlwaysAsk,
                "shared attacker-owned profile should not control command approvals"
            );
            assert_ne!(default_profile.sync_id(), Some(attacker_sync_id));
        });
    })
}

#[test]
fn ignores_shared_default_profile_after_initial_load() {
    let _guard = FeatureFlag::SharedWithMe.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        let attacker_sync_id = SyncId::ServerId(ServerId::from(31338));
        let attacker_profile = attacker_owned_shared_default_profile(ServerId::from(31338));
        CloudModel::handle(&app).update(&mut app, move |cloud_model, ctx| {
            let server_objects: Vec<ServerAIExecutionProfile> = vec![attacker_profile];
            cloud_model.update_objects_from_initial_load(server_objects, false, false, ctx);
            ctx.emit(CloudModelEvent::InitialLoadCompleted);
        });

        profile_model.read(&app, |model, ctx| {
            let default_profile = model.default_profile(ctx);
            assert_eq!(
                default_profile.sync_id(),
                None,
                "shared attacker-owned default profile should not be reconciled as default"
            );
            assert_eq!(
                default_profile.data().execute_commands,
                ActionPermission::AlwaysAsk,
                "shared attacker-owned profile should not control command approvals"
            );
            assert_ne!(default_profile.sync_id(), Some(attacker_sync_id));
        });
    })
}

#[test]
fn filters_non_owned_non_default_profile_from_list() {
    let _guard = FeatureFlag::SharedWithMe.override_enabled(true);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        // Create a non-default profile owned by an attacker, shared with victim
        let attacker_owner = Owner::User {
            user_uid: UserUid::new("attacker-owner"),
        };
        let attacker_profile = AIExecutionProfile {
            name: "Attacker Custom".to_string(),
            is_default_profile: false,
            ..Default::default()
        };
        let attacker_server_obj = ServerAIExecutionProfile::new(
            SyncId::ServerId(ServerId::from(99999)),
            CloudAIExecutionProfileModel::new(attacker_profile),
            mock_server_metadata(ServerId::from(99999)),
            ServerPermissions {
                space: attacker_owner,
                guests: vec![ServerObjectGuest {
                    subject: ServerGuestSubject::User {
                        firebase_uid: TEST_USER_UID.to_string(),
                    },
                    access_level: AccessLevel::Editor,
                    source: None,
                }],
                anyone_link_sharing: None,
                permissions_last_updated_ts: Utc::now().into(),
            },
        );

        CloudModel::handle(&app).update(&mut app, move |cloud_model, ctx| {
            cloud_model.upsert_from_server_object(attacker_server_obj, ctx);
        });

        profile_model.read(&app, |model, ctx| {
            assert!(
                !model.has_multiple_profiles(),
                "non-owned profile should not appear in profile list"
            );
            let all_ids = model.get_all_profile_ids();
            assert_eq!(
                all_ids.len(),
                1,
                "only the default profile should be in the list"
            );
            assert_eq!(all_ids[0], model.default_profile_id());
            assert_eq!(
                model.default_profile(ctx).data().name,
                "Default",
                "surviving profile should be the user's default, not the attacker's"
            );
        });
    })
}
