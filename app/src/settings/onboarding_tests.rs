use ai::LLMId;
use chrono::{DateTime, Utc};
use onboarding::SelectedSettings;
use onboarding::slides::{AgentAutonomy, AgentDevelopmentSettings, ProjectOnboardingSettings};
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity};

use crate::LaunchMode;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::execution_profiles::{
    AIExecutionProfile, ActionPermission, CloudAIExecutionProfileModel,
};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::{Revision, ServerAIExecutionProfile, ServerMetadata, ServerPermissions};
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ServerId, SyncId};
use crate::server::sync_queue::SyncQueue;
use crate::settings::{AISettings, PrivacySettings, apply_onboarding_settings};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
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

/// Regression test for: "Logging in to an existing user at the end of
/// onboarding should preserve the user's cloud-stored default execution
/// profile rather than overwriting it with the onboarding-selected
/// base_model and autonomy."
///
/// Simulates the full post-login flow:
///   1. User starts unauthenticated; `AIExecutionProfilesModel` begins in
///      `Unsynced`.
///   2. User logs into an existing account; their cloud default profile
///      arrives via initial bulk load and `InitialLoadCompleted` fires.
///   3. The reconciliation handler promotes the local state to `Synced`
///      with the cloud profile's `sync_id`.
///   4. `apply_onboarding_settings` runs (as it would from
///      `handle_cloud_preferences_syncer_event`) with onboarding-selected
///      values that differ from what's on the cloud profile.
///   5. The cloud profile's stored values must be preserved.
#[test]
fn apply_onboarding_settings_preserves_existing_cloud_profile_on_existing_user_login() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());
        app.add_singleton_model(PrivacySettings::mock);
        app.add_singleton_model(UserWorkspaces::default_mock);
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        // The existing user's stored cloud default profile. Values are
        // deliberately chosen to differ from both `AIExecutionProfile`'s
        // defaults and from the onboarding values we'll pass below, so any
        // accidental overwrite is detectable.
        let cloud_uid = ServerId::from(7);
        let cloud_sync_id = SyncId::ServerId(cloud_uid);
        let cloud_stored_model = LLMId::from("claude-existing-cloud-model");
        let cloud_profile = AIExecutionProfile {
            name: "Default".to_string(),
            is_default_profile: true,
            base_model: Some(cloud_stored_model.clone()),
            apply_code_diffs: ActionPermission::AlwaysAllow,
            read_files: ActionPermission::AlwaysAllow,
            execute_commands: ActionPermission::AlwaysAllow,
            mcp_permissions: ActionPermission::AlwaysAllow,
            ..Default::default()
        };
        let server_object = ServerAIExecutionProfile::new(
            cloud_sync_id,
            CloudAIExecutionProfileModel::new(cloud_profile),
            mock_server_metadata(cloud_uid),
            ServerPermissions::mock_personal(),
        );

        // Insert the existing user's cloud profile via the initial-load
        // path (no per-object events) and emit `InitialLoadCompleted` so
        // `AIExecutionProfilesModel` reconciles to `Synced`.
        CloudModel::handle(&app).update(&mut app, move |cloud_model, ctx| {
            let server_objects: Vec<ServerAIExecutionProfile> = vec![server_object];
            cloud_model.update_objects_from_initial_load(server_objects, false, false, ctx);
            ctx.emit(CloudModelEvent::InitialLoadCompleted);
        });

        // Sanity: reconciliation occurred and the model now reads the
        // cloud profile.
        profile_model.read(&app, |model, ctx| {
            let info = model.default_profile(ctx);
            assert_eq!(info.sync_id(), Some(cloud_sync_id));
            assert_eq!(info.data().base_model, Some(cloud_stored_model.clone()));
        });

        // Simulate the onboarding handoff: the user picked a different
        // base_model and "None" autonomy on the agent slide, which would
        // map to every `ActionPermission` being `AlwaysAsk`.
        let onboarding_settings = SelectedSettings::AgentDrivenDevelopment {
            agent_settings: AgentDevelopmentSettings {
                selected_model_id: LLMId::from("onboarding-chosen-model"),
                autonomy: Some(AgentAutonomy::None),
                cli_agent_toolbar_enabled: true,
                session_default: onboarding::SessionDefault::Agent,
                disable_oz: false,
                show_agent_notifications: true,
            },
            project_settings: ProjectOnboardingSettings::default(),
            ui_customization: None,
        };

        app.update(|ctx| {
            apply_onboarding_settings(&onboarding_settings, true, ctx);
        });

        // Post-condition: the cloud profile retains its stored values.
        // Every field touched by `apply_agent_settings` should be
        // unchanged.
        profile_model.read(&app, |model, ctx| {
            let info = model.default_profile(ctx);
            assert_eq!(
                info.sync_id(),
                Some(cloud_sync_id),
                "still pointing at the existing cloud profile"
            );
            assert_eq!(
                info.data().base_model,
                Some(cloud_stored_model.clone()),
                "base_model should not be overwritten by onboarding for existing users"
            );
            assert_eq!(
                info.data().apply_code_diffs,
                ActionPermission::AlwaysAllow,
                "apply_code_diffs should not be overwritten by onboarding for existing users"
            );
            assert_eq!(
                info.data().read_files,
                ActionPermission::AlwaysAllow,
                "read_files should not be overwritten by onboarding for existing users"
            );
            assert_eq!(
                info.data().execute_commands,
                ActionPermission::AlwaysAllow,
                "execute_commands should not be overwritten by onboarding for existing users"
            );
            assert_eq!(
                info.data().mcp_permissions,
                ActionPermission::AlwaysAllow,
                "mcp_permissions should not be overwritten by onboarding for existing users"
            );
        });
    })
}

/// Warp's AI features run on a Warp account. For third-party agent intent
/// (`disable_oz = true`), AI is therefore off when the user skips creating an
/// account and on once they have one.
#[test]
fn apply_onboarding_settings_gates_third_party_ai_on_account() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());
        app.add_singleton_model(PrivacySettings::mock);
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        let onboarding_settings = SelectedSettings::AgentDrivenDevelopment {
            agent_settings: AgentDevelopmentSettings {
                selected_model_id: LLMId::from("auto"),
                autonomy: None,
                cli_agent_toolbar_enabled: true,
                session_default: onboarding::SessionDefault::Agent,
                disable_oz: true,
                show_agent_notifications: true,
            },
            project_settings: ProjectOnboardingSettings::default(),
            ui_customization: None,
        };

        // Skipping login (no account) leaves AI off, even for agent intent.
        app.update(|ctx| {
            apply_onboarding_settings(&onboarding_settings, false, ctx);
        });
        let ai_disabled = app.read(|ctx| !*AISettings::as_ref(ctx).is_any_ai_enabled);
        assert!(
            ai_disabled,
            "skipping login must disable AI even for agent intent"
        );

        // Creating an account turns AI on, including for third-party agents.
        app.update(|ctx| {
            apply_onboarding_settings(&onboarding_settings, true, ctx);
        });
        let ai_enabled = app.read(|ctx| *AISettings::as_ref(ctx).is_any_ai_enabled);
        assert!(
            ai_enabled,
            "creating an account must enable AI for third-party agent intent"
        );
    })
}
