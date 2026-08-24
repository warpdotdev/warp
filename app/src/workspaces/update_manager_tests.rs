use chrono::Utc;
use cloud_object_client::MockObjectClient;
use itertools::Itertools;
use settings::{PrivatePreferences, PublicPreferences};
use warp_graphql::workspace::{
    AvailableLlms as GqlAvailableLlms, LlmContextWindow as GqlLlmContextWindow,
    LlmInfo as GqlLlmInfo, LlmPricing as GqlLlmPricing, LlmProvider as GqlLlmProvider,
    LlmUsageMetadata as GqlLlmUsageMetadata,
};
use warpui::{AddSingletonModel, App};
use warpui_extras::user_preferences;

use super::*;
use crate::ai::credit_availability::{AICreditAvailability, AICreditDenialReason};
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::auth::AuthManager;
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions, ServerWorkflow};
use crate::server::cloud_objects::update_manager::InitialLoadResponse;
use crate::server::ids::SyncId;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::{MockWorkspaceClient, WorkspaceClient};
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::{AISettings, CodeSettings, PrivacySettings};
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, CloudWorkflowModel, WorkflowId};
use crate::workspaces::team::Team;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::workspace::{PurchaseAddOnCreditsPolicy, Workspace, WorkspaceUid};

fn initialize_app(
    team_client: Arc<dyn TeamClient>,
    workspace_client: Arc<dyn WorkspaceClient>,
    workspaces: Vec<Workspace>,
    app: &mut App,
) {
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client.clone(),
            workspace_client.clone(),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ObjectActions::new(vec![]));
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_| UserProfiles::new(vec![]));
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
}

fn mock_workflow(id: WorkflowId, owner: Owner) -> CloudWorkflow {
    CloudWorkflow::new_from_server(mock_server_workflow(id, owner))
}

fn mock_server_workflow(id: WorkflowId, owner: Owner) -> ServerWorkflow {
    ServerWorkflow::new(
        SyncId::ServerId(id.into()),
        CloudWorkflowModel::new(Workflow::new("Test Workflow", "echo hello")),
        ServerMetadata {
            uid: id.into(),
            revision: Revision::now(),
            metadata_last_updated_ts: Utc::now().into(),
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            current_editor_uid: None,
        },
        ServerPermissions {
            space: owner,
            permissions_last_updated_ts: Utc::now().into(),
            anyone_link_sharing: None,
            guests: vec![],
        },
    )
}

#[test]
fn test_leaving_team_removes_objects() {
    App::test((), |mut app| async move {
        let workspace_uid: WorkspaceUid = WorkspaceUid::from(ServerId::from(987));
        let team_uid: ServerId = ServerId::from(123);
        let team_workflow_id = WorkflowId::from(1);
        let personal_workflow_id = WorkflowId::from(2);
        let shared_workflow_id = WorkflowId::from(3);
        let shared_workflow = mock_server_workflow(shared_workflow_id, Owner::Team { team_uid });

        let mut team_client = MockTeamClient::new();
        team_client.expect_workspaces_metadata().returning(|| {
            Ok(WorkspacesMetadataWithPricing {
                metadata: WorkspacesMetadataResponse {
                    workspaces: vec![],
                    joinable_teams: vec![],
                    experiments: None,
                    feature_model_choices: None,
                    team_feature_model_choices: HashMap::new(),
                    ai_credit_availability: None,
                    user_purchase_policy: None,
                },
                pricing_info: None,
            })
        });

        let workspace_client = MockWorkspaceClient::new();
        let team_client = Arc::new(team_client);
        let workspace_client = Arc::new(workspace_client);
        initialize_app(
            team_client.clone(),
            workspace_client.clone(),
            vec![Workspace::from_local_cache(
                workspace_uid,
                "Test Workspace".to_owned(),
                Some(vec![Team::from_local_cache(
                    team_uid,
                    "Test Team".to_owned(),
                    None,
                    None,
                    None,
                )]),
            )],
            &mut app,
        );

        // Add the initial Warp Drive objects.
        CloudModel::handle(&app).update(&mut app, |cloud_model, _| {
            cloud_model.add_object(
                SyncId::ServerId(team_workflow_id.into()),
                mock_workflow(team_workflow_id, Owner::Team { team_uid }),
            );

            cloud_model.add_object(
                SyncId::ServerId(shared_workflow_id.into()),
                CloudWorkflow::new_from_server(shared_workflow.clone()),
            );

            cloud_model.add_object(
                SyncId::ServerId(personal_workflow_id.into()),
                mock_workflow(personal_workflow_id, Owner::mock_current_user()),
            );
        });

        let mut cloud_server_api = MockObjectClient::new();
        cloud_server_api
            .expect_fetch_changed_objects()
            .returning(move |_, _| {
                Ok(InitialLoadResponse {
                    updated_workflows: vec![shared_workflow.clone()],
                    ..Default::default()
                })
            });

        let team_update_manager =
            app.add_singleton_model(|ctx| TeamUpdateManager::new(team_client, None, ctx));

        let cloud_update_manager = app
            .add_singleton_model(|ctx| UpdateManager::new(None, Arc::new(cloud_server_api), ctx));

        // Simulate leaving the team.
        team_update_manager.update(&mut app, |team_manager, ctx| {
            team_manager.on_team_left(
                team_uid,
                Ok(WorkspacesMetadataWithPricing {
                    metadata: WorkspacesMetadataResponse {
                        workspaces: vec![],
                        joinable_teams: vec![],
                        experiments: None,
                        feature_model_choices: None,
                        team_feature_model_choices: HashMap::new(),
                        ai_credit_availability: None,
                        user_purchase_policy: None,
                    },
                    pricing_info: None,
                }),
                ctx,
            );
        });

        // Both team-owned objects should be removed.
        CloudModel::handle(&app).read(&app, |cloud_model, _| {
            assert_eq!(
                cloud_model
                    .cloud_objects()
                    .map(|obj| obj.uid())
                    .collect_vec(),
                vec![personal_workflow_id.to_string()]
            );
        });

        // This should also trigger a refresh.
        cloud_update_manager
            .update(&mut app, |update_manager, ctx| {
                ctx.await_spawned_future(update_manager.spawned_futures()[0])
            })
            .await;

        // The refresh will then re-add the shared workflow.
        CloudModel::handle(&app).read(&app, |cloud_model, _| {
            let mut objects = cloud_model
                .cloud_objects()
                .map(|obj| obj.uid())
                .collect_vec();
            objects.sort();
            assert_eq!(
                objects,
                vec![
                    personal_workflow_id.to_string(),
                    shared_workflow_id.to_string()
                ]
            );
        });
    });
}

#[test]
fn test_workspace_metadata_piggyback_feeds_ai_credit_availability() {
    App::test((), |mut app| async move {
        let team_client = Arc::new(MockTeamClient::new());
        initialize_app(
            team_client.clone(),
            Arc::new(MockWorkspaceClient::new()),
            vec![],
            &mut app,
        );
        if app
            .models_of_type::<settings::PrivatePreferences>()
            .is_empty()
        {
            app.update(crate::settings::init_and_register_user_preferences);
        }
        app.add_singleton_model(|ctx| {
            AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
        });
        let team_update_manager =
            app.add_singleton_model(|ctx| TeamUpdateManager::new(team_client, None, ctx));

        let availability = AICreditAvailability::unavailable(AICreditDenialReason::OutOfCredits);
        team_update_manager.update(&mut app, |manager, ctx| {
            manager.on_workspaces_updated(
                Ok(WorkspacesMetadataResponse {
                    workspaces: vec![],
                    joinable_teams: vec![],
                    experiments: None,
                    feature_model_choices: None,
                    team_feature_model_choices: HashMap::new(),
                    ai_credit_availability: Some(availability),
                    user_purchase_policy: None,
                }),
                ctx,
            );
        });

        AIRequestUsageModel::handle(&app).read(&app, |model, _| {
            assert_eq!(model.server_availability(), Some(availability));
        });
    });
}

#[test]
fn test_poll_path_apply_refreshes_user_purchase_policy() {
    App::test((), |mut app| async move {
        let team_client = Arc::new(MockTeamClient::new());
        let workspace_client = Arc::new(MockWorkspaceClient::new());
        initialize_app(team_client.clone(), workspace_client, vec![], &mut app);

        let team_update_manager =
            app.add_singleton_model(|ctx| TeamUpdateManager::new(team_client, None, ctx));

        // The periodic poll applies metadata through TeamUpdateManager's
        // own on_workspaces_updated; it must refresh the stored user-level
        // policy.
        let response_with_policy = WorkspacesMetadataResponse {
            workspaces: vec![],
            joinable_teams: vec![],
            experiments: None,
            feature_model_choices: None,
            team_feature_model_choices: HashMap::new(),
            ai_credit_availability: None,
            user_purchase_policy: Some(PurchaseAddOnCreditsPolicy {
                enabled: false,
                premium_enabled: true,
                price_premium_bps: 1000,
            }),
        };
        team_update_manager.update(&mut app, |manager, ctx| {
            manager.on_workspaces_updated(Ok(response_with_policy), ctx);
        });
        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx)
                    .purchase_policy()
                    .is_some_and(|policy| policy.allows_purchases()),
                "a poll-path apply should store the user-level policy"
            );
        });

        // A later poll without the policy must clear the stored fallback so
        // it can't go stale.
        let response_without_policy = WorkspacesMetadataResponse {
            workspaces: vec![],
            joinable_teams: vec![],
            experiments: None,
            feature_model_choices: None,
            team_feature_model_choices: HashMap::new(),
            ai_credit_availability: None,
            user_purchase_policy: None,
        };
        team_update_manager.update(&mut app, |manager, ctx| {
            manager.on_workspaces_updated(Ok(response_without_policy), ctx);
        });
        app.read(|ctx| {
            assert!(
                UserWorkspaces::as_ref(ctx).purchase_policy().is_none(),
                "a poll-path apply without the policy should clear the stored fallback"
            );
        });
    });
}

fn gql_llm_info(id: &str) -> GqlLlmInfo {
    GqlLlmInfo {
        display_name: id.to_string(),
        base_model_name: id.to_string(),
        id: id.to_string(),
        reasoning_level: None,
        usage_metadata: GqlLlmUsageMetadata {
            credit_multiplier: None,
            request_multiplier: 1,
        },
        description: None,
        disable_reason: None,
        vision_supported: false,
        spec: None,
        provider: GqlLlmProvider::Unknown,
        host_configs: vec![],
        pricing: GqlLlmPricing {
            discount_percentage: None,
        },
        context_window: GqlLlmContextWindow {
            is_configurable: false,
            min: 0.into(),
            max: 0.into(),
            default: 0.into(),
        },
    }
}

fn gql_available_llms(model_id: &str) -> GqlAvailableLlms {
    GqlAvailableLlms {
        default_id: model_id.to_string(),
        choices: vec![gql_llm_info(model_id)],
        preferred_codex_model_id: None,
    }
}

/// A `FeatureModelChoice` whose every feature offers exactly one model, `model_id`, so a
/// test can tell two teams' catalogs apart by that single id.
fn feature_model_choice_with_model(model_id: &str) -> FeatureModelChoice {
    FeatureModelChoice {
        agent_mode: gql_available_llms(model_id),
        planning: gql_available_llms(model_id),
        coding: gql_available_llms(model_id),
        cli_agent: gql_available_llms(model_id),
        computer_use_agent: gql_available_llms(model_id),
    }
}

/// Registers the minimal settings/preferences singletons `LLMPreferences::new` needs, on top
/// of this module's own `initialize_app`. Kept separate (rather than folded into
/// `initialize_app`) because most tests in this file never touch the model catalog.
fn initialize_llm_preferences_dependencies(app: &mut App) {
    app.add_singleton_model(|_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    app.add_singleton_model(|_| {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    app.add_singleton_model(CodeSettings::new_with_defaults);
    app.add_singleton_model(AISettings::new_with_defaults);
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
    });
    app.add_singleton_model(ai::api_keys::ApiKeyManager::new);
    app.add_singleton_model(|_| crate::ai::mcp::TemplatableMCPServerManager::default());
    app.add_singleton_model(|ctx| {
        crate::ai::execution_profiles::profiles::AIExecutionProfilesModel::new(
            &crate::LaunchMode::new_for_unit_test(),
            ctx,
        )
    });
}

/// Regression for two review findings on the multi-team catalog rebuild:
///
/// 1. The retained blocklist-controller refresh triggers now go through
///    `TeamUpdateManager::refresh_workspace_metadata` -- i.e. this same `on_workspaces_updated`
///    apply path -- instead of the legacy unscoped `get_feature_model_choices` fetch. Team A
///    and team B below have genuinely different catalogs, so a "same payload in every bucket"
///    defect in that path would fail the first assertion block.
/// 2. Pruning used to be skipped whenever a response's *own* folded catalog was empty
///    (`on_workspaces_updated` guarded the whole apply on `!feature_models_by_team.is_empty()`),
///    which is a real state -- every team the user was on responding with no catalog entry, or
///    the user leaving every team -- not just "no teams named at all" (naming only team B, as
///    the middle step below does, is already non-empty and would not have exercised the old
///    guard). A later authoritative response with an empty catalog must still evict every
///    stale bucket it doesn't name, rather than leaving them live indefinitely.
#[test]
fn on_workspaces_updated_keeps_teams_distinct_and_prunes_a_team_the_response_omits() {
    App::test((), |mut app| async move {
        let team_client = Arc::new(MockTeamClient::new());
        initialize_app(
            team_client.clone(),
            Arc::new(MockWorkspaceClient::new()),
            vec![],
            &mut app,
        );
        initialize_llm_preferences_dependencies(&mut app);
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);
        let team_update_manager =
            app.add_singleton_model(|ctx| TeamUpdateManager::new(team_client, None, ctx));

        let team_a = ServerId::from(1);
        let team_b = ServerId::from(2);
        let model_a = LLMId::from("team-a-only");
        let model_b = LLMId::from("team-b-only");

        team_update_manager.update(&mut app, |manager, ctx| {
            manager.on_workspaces_updated(
                Ok(WorkspacesMetadataResponse {
                    workspaces: vec![],
                    joinable_teams: vec![],
                    experiments: None,
                    feature_model_choices: None,
                    team_feature_model_choices: HashMap::from([
                        (team_a, feature_model_choice_with_model(model_a.as_str())),
                        (team_b, feature_model_choice_with_model(model_b.as_str())),
                    ]),
                    ai_credit_availability: None,
                    user_purchase_policy: None,
                }),
                ctx,
            );
        });

        llm_preferences.read(&app, |preferences, _| {
            assert!(
                preferences
                    .get_llm_info_for_team_uid(Some(team_a), &model_a)
                    .is_some(),
                "team A's own model should be visible in its own bucket"
            );
            assert!(
                preferences
                    .get_llm_info_for_team_uid(Some(team_b), &model_b)
                    .is_some(),
                "team B's own model should be visible in its own bucket"
            );
            assert!(
                preferences
                    .get_llm_info_for_team_uid(Some(team_a), &model_b)
                    .is_none(),
                "team A's bucket must not contain team B's model"
            );
            assert!(
                preferences
                    .get_llm_info_for_team_uid(Some(team_b), &model_a)
                    .is_none(),
                "team B's bucket must not contain team A's model"
            );
        });

        // Team A left the account; the next authoritative response names only team B.
        team_update_manager.update(&mut app, |manager, ctx| {
            manager.on_workspaces_updated(
                Ok(WorkspacesMetadataResponse {
                    workspaces: vec![],
                    joinable_teams: vec![],
                    experiments: None,
                    feature_model_choices: None,
                    team_feature_model_choices: HashMap::from([(
                        team_b,
                        feature_model_choice_with_model(model_b.as_str()),
                    )]),
                    ai_credit_availability: None,
                    user_purchase_policy: None,
                }),
                ctx,
            );
        });

        llm_preferences.read(&app, |preferences, _| {
            assert!(
                preferences
                    .get_llm_info_for_team_uid(Some(team_a), &model_a)
                    .is_none(),
                "team A's catalog bucket should have been evicted once the response stopped \
                 naming it, not left stale"
            );
            assert!(
                preferences
                    .get_llm_info_for_team_uid(Some(team_b), &model_b)
                    .is_some(),
                "team B's catalog should remain"
            );
        });

        // The user then leaves team B too: the next authoritative response's folded catalog is
        // genuinely empty (no workspace-level entry, no team entries at all). This is exactly
        // the case the old `!feature_models_by_team.is_empty()` guard skipped applying
        // entirely, so team B's bucket would previously have survived this update unpruned.
        team_update_manager.update(&mut app, |manager, ctx| {
            manager.on_workspaces_updated(
                Ok(WorkspacesMetadataResponse {
                    workspaces: vec![],
                    joinable_teams: vec![],
                    experiments: None,
                    feature_model_choices: None,
                    team_feature_model_choices: HashMap::new(),
                    ai_credit_availability: None,
                    user_purchase_policy: None,
                }),
                ctx,
            );
        });

        llm_preferences.read(&app, |preferences, app| {
            assert!(
                preferences
                    .get_llm_info_for_team_uid(Some(team_b), &model_b)
                    .is_none(),
                "an authoritative empty catalog must prune every remaining bucket, not be \
                 skipped as a no-op"
            );
            // The resolved-teamless fallback must still resolve to the built-in default rather
            // than panicking or resolving through a leftover team bucket.
            preferences.get_default_base_model_for_team_uid(None, app);
        });
    });
}
