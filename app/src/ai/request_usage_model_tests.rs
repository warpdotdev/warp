use std::sync::Arc;

use chrono::Duration;
use warpui::{App, ModelHandle};

use crate::auth::AuthStateProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::server_api::ServerApiProvider;
use crate::workspaces::{
    user_workspaces::UserWorkspaces,
    workspace::{
        AiOverages, ByoApiKeyPolicy, CustomerType, EnterpriseCreditsAutoReloadPolicy,
        EnterprisePayAsYouGoPolicy, PurchaseAddOnCreditsPolicy, Workspace, WorkspaceUid,
    },
};

use ai::api_keys::ApiKeyManager;
use warp_core::features::FeatureFlag;

use super::*;

fn create_test_workspace() -> (WorkspaceUid, Workspace) {
    let server_id: crate::server::ids::ServerId = 1_i64.into();
    let uid = WorkspaceUid::from(server_id);
    let workspace = Workspace::from_local_cache(uid, "Test Workspace".to_string(), None);
    (uid, workspace)
}

fn add_user_workspaces_with_workspace(app: &mut App, workspace: Workspace) {
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![workspace],
            ctx,
        )
    });
}

fn add_request_usage_model(app: &mut App) -> ModelHandle<AIRequestUsageModel> {
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    add_request_usage_model_without_auth(app)
}

fn add_request_usage_model_for_anonymous_users(app: &mut App) -> ModelHandle<AIRequestUsageModel> {
    app.add_singleton_model(|_| AuthStateProvider::new_anonymous_for_test());
    add_request_usage_model_without_auth(app)
}

fn add_request_usage_model_without_auth(app: &mut App) -> ModelHandle<AIRequestUsageModel> {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
        ctx.add_singleton_model(ApiKeyManager::new);
    });
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    })
}

#[test]
fn test_request_limit_info() {
    App::test((), |mut app| async move {
        let request_usage_model = add_request_usage_model(&mut app);
        request_usage_model.update(&mut app, |request_usage_model, _ctx| {
            request_usage_model.request_limit_info = RequestLimitInfo {
                limit: 200,
                num_requests_used_since_refresh: 39,
                next_refresh_time: ServerTimestamp::new(Utc::now() + Duration::days(1)),
                is_unlimited: false,
                request_limit_refresh_duration: RequestLimitRefreshDuration::Monthly,
                is_unlimited_voice: false,
                voice_request_limit: 100,
                voice_requests_used_since_last_refresh: 0,
                is_unlimited_codebase_indices: false,
                max_codebase_indices: 3,
                max_files_per_repo: 5000,
                embedding_generation_batch_size: 100,
            };
            assert_eq!(200, request_usage_model.request_limit());
            assert_eq!(39, request_usage_model.requests_used());
            assert_eq!(161, request_usage_model.requests_remaining());
        })
    });
}

#[test]
fn test_request_limit_info_with_limit() {
    App::test((), |mut app| async move {
        let request_usage_model = add_request_usage_model(&mut app);
        request_usage_model.update(&mut app, |request_usage_model, _ctx| {
            request_usage_model.request_limit_info = RequestLimitInfo {
                limit: 999999999,
                num_requests_used_since_refresh: 39,
                next_refresh_time: ServerTimestamp::new(Utc::now() + Duration::minutes(1)),
                is_unlimited: false,
                request_limit_refresh_duration: RequestLimitRefreshDuration::Monthly,
                is_unlimited_voice: false,
                voice_request_limit: 100,
                voice_requests_used_since_last_refresh: 0,
                is_unlimited_codebase_indices: false,
                max_codebase_indices: 3,
                max_files_per_repo: 5000,
                embedding_generation_batch_size: 100,
            };
            assert_eq!(999999999, request_usage_model.request_limit());
            assert_eq!(39, request_usage_model.requests_used());
            assert_eq!(999999960, request_usage_model.requests_remaining());
        })
    });
}

#[test]
fn test_request_limit_info_past_refresh_time() {
    App::test((), |mut app| async move {
        let request_usage_model = add_request_usage_model(&mut app);
        request_usage_model.update(&mut app, |request_usage_model, _ctx| {
            request_usage_model.request_limit_info = RequestLimitInfo {
                limit: 200,
                num_requests_used_since_refresh: 39,
                next_refresh_time: ServerTimestamp::new(Utc::now() - Duration::seconds(1)),
                is_unlimited: false,
                request_limit_refresh_duration: RequestLimitRefreshDuration::Monthly,
                is_unlimited_voice: false,
                voice_request_limit: 100,
                voice_requests_used_since_last_refresh: 0,
                is_unlimited_codebase_indices: false,
                max_codebase_indices: 3,
                max_files_per_repo: 5000,
                embedding_generation_batch_size: 100,
            };
            assert_eq!(200, request_usage_model.request_limit());
            assert_eq!(0, request_usage_model.requests_used());
            assert_eq!(200, request_usage_model.requests_remaining());
        })
    });
}

#[test]
fn test_request_limit_info_is_unlimited_true() {
    App::test((), |mut app| async move {
        let request_usage_model = add_request_usage_model(&mut app);
        request_usage_model.update(&mut app, |request_usage_model, _ctx| {
            request_usage_model.request_limit_info = RequestLimitInfo {
                limit: 999999999,
                num_requests_used_since_refresh: 39,
                next_refresh_time: ServerTimestamp::new(Utc::now() + Duration::minutes(1)),
                is_unlimited: true,
                request_limit_refresh_duration: RequestLimitRefreshDuration::Monthly,
                is_unlimited_voice: false,
                voice_request_limit: 100,
                voice_requests_used_since_last_refresh: 0,
                is_unlimited_codebase_indices: false,
                max_codebase_indices: 3,
                max_files_per_repo: 5000,
                embedding_generation_batch_size: 100,
            };
            assert_eq!(999999999, request_usage_model.request_limit());
            assert_eq!(39, request_usage_model.requests_used());
            assert_eq!(999999999, request_usage_model.requests_remaining());
        })
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_remaining_requests() {
    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // Some requests remaining, no bonus or overages needed.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 5);
            assert!(model.has_any_ai_remaining(ctx));
        });
    });
}

#[test]
fn test_buy_credits_banner_shows_with_only_ambient_bonus_credits() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(PurchaseAddOnCreditsPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants = vec![BonusGrant {
                created_at: Utc::now(),
                cost_cents: 0,
                expiration: Some(Utc::now() + chrono::Duration::days(7)),
                grant_type: BonusGrantType::AmbientOnly,
                reason: "ambient trial credits".to_string(),
                user_facing_message: None,
                request_credits_granted: 1000,
                request_credits_remaining: 1000,
                scope: BonusGrantScope::User,
            }];

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::OutOfCredits,
            );
        });
    });
}

#[test]
fn test_buy_credits_banner_hidden_with_non_ambient_bonus_credits() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(PurchaseAddOnCreditsPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants = vec![BonusGrant {
                created_at: Utc::now(),
                cost_cents: 0,
                expiration: Some(Utc::now() + chrono::Duration::days(7)),
                grant_type: BonusGrantType::Any,
                reason: "standard bonus credits".to_string(),
                user_facing_message: None,
                request_credits_granted: 100,
                request_credits_remaining: 100,
                scope: BonusGrantScope::User,
            }];

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::Hidden,
            );
        });
    });
}

#[test]
fn test_buy_credits_banner_shows_when_non_ambient_bonus_credits_are_depleted() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(PurchaseAddOnCreditsPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants = vec![BonusGrant {
                created_at: Utc::now(),
                cost_cents: 0,
                expiration: Some(Utc::now() + chrono::Duration::days(7)),
                grant_type: BonusGrantType::Any,
                reason: "depleted standard bonus credits".to_string(),
                user_facing_message: None,
                request_credits_granted: 100,
                request_credits_remaining: 0,
                scope: BonusGrantScope::User,
            }];

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::OutOfCredits,
            );
        });
    });
}
#[test]
fn test_has_any_ai_remaining_false_when_no_requests_or_bonus() {
    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // At limit, no bonus credits and no overages.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            assert!(!model.has_any_ai_remaining(ctx));
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_user_bonus_credits() {
    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);

            // User-level bonus credits remaining.
            model.bonus_grants = vec![BonusGrant {
                created_at: Utc::now(),
                cost_cents: 0,
                expiration: Some(Utc::now() + chrono::Duration::days(7)),
                grant_type: BonusGrantType::Any,
                reason: "test user bonus".to_string(),
                user_facing_message: None,
                request_credits_granted: 5,
                request_credits_remaining: 5,
                scope: BonusGrantScope::User,
            }];

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when user bonus credits exist",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_workspace_overages() {
    App::test((), |mut app| async move {
        // Create a workspace with overages enabled and remaining.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.settings.usage_based_pricing_settings.enabled = true;
        workspace
            .settings
            .usage_based_pricing_settings
            .max_monthly_spend_cents = Some(1_000);
        workspace.billing_metadata.ai_overages = Some(AiOverages {
            current_monthly_request_cost_cents: 100,
            current_monthly_requests_used: 100,
            current_period_end: Utc::now() + chrono::Duration::days(7),
        });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests left and no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected overages to count as remaining AI when standard requests are exhausted",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_workspace_bonus_credits() {
    App::test((), |mut app| async move {
        let (uid, workspace) = create_test_workspace();
        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);

            // Workspace-level bonus credits remaining.
            model.bonus_grants = vec![BonusGrant {
                created_at: Utc::now(),
                cost_cents: 0,
                expiration: Some(Utc::now() + chrono::Duration::days(7)),
                grant_type: BonusGrantType::Any,
                reason: "test workspace bonus".to_string(),
                user_facing_message: None,
                request_credits_granted: 5,
                request_credits_remaining: 5,
                scope: BonusGrantScope::Workspace(uid),
            }];

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when workspace bonus credits exist",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_payg_enabled() {
    App::test((), |mut app| async move {
        // Create a workspace with pay-as-you-go enabled.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.billing_metadata.customer_type = CustomerType::Enterprise;
        workspace
            .billing_metadata
            .tier
            .enterprise_pay_as_you_go_policy = Some(EnterprisePayAsYouGoPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining, no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when pay-as-you-go is enabled",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_enterprise_auto_reload() {
    App::test((), |mut app| async move {
        // Create a workspace with enterprise auto-reload enabled.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.billing_metadata.customer_type = CustomerType::Enterprise;
        workspace
            .billing_metadata
            .tier
            .enterprise_credits_auto_reload_policy =
            Some(EnterpriseCreditsAutoReloadPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining, no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when enterprise auto-reload is enabled",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_false_with_workspace_no_pricing_no_overages_no_credits() {
    App::test((), |mut app| async move {
        // Create a workspace with no tier pricing (default).
        let (_uid, workspace) = create_test_workspace();

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining, no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false with no pricing, no overages, no credits",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_false_both_payg_and_autoreload_disabled() {
    App::test((), |mut app| async move {
        // Create a workspace with policies but payg disabled and auto-reload disabled.
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .enterprise_pay_as_you_go_policy = Some(EnterprisePayAsYouGoPolicy { enabled: false });
        workspace
            .billing_metadata
            .tier
            .enterprise_credits_auto_reload_policy =
            Some(EnterpriseCreditsAutoReloadPolicy { enabled: false });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining, no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false with policies but payg and auto-reload disabled",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_byok_enabled_and_key_provided() {
    App::test((), |mut app| async move {
        // Create a workspace with BYOK (Bring Your Own Key) enabled.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_openai_key(Some("test-key".to_string()), ctx);
        });

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining, no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when BYOK is enabled and a key is provided",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_false_with_byok_enabled_but_no_key() {
    App::test((), |mut app| async move {
        // Create a workspace with BYOK enabled but no key provided.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining, no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false when BYOK is enabled but no key is provided",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_byo_key_and_no_workspace() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SoloUserByok.override_enabled(true);

        // No workspace — user is not on a team.
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_openai_key(Some("test-key".to_string()), ctx);
        });

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining, no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when user has a BYO key but no workspace",
            );
        });
    });
}

#[test]
fn test_byo_api_key_disabled_for_anonymous_firebase_user() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SoloUserByok.override_enabled(true);

        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model_for_anonymous_users(&mut app);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_openai_key(Some("test-key".to_string()), ctx);
        });

        app.read(|ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).is_byo_api_key_enabled(ctx),
                "expected is_byo_api_key_enabled to be false for anonymous Firebase users even with SoloUserByok enabled",
            );
        });

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false for anonymous Firebase user even with BYO key and SoloUserByok enabled",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_false_with_only_ambient_bonus_credits() {
    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);

            // Only ambient-only bonus credits.
            model.bonus_grants = vec![BonusGrant {
                created_at: Utc::now(),
                cost_cents: 0,
                expiration: Some(Utc::now() + chrono::Duration::days(7)),
                grant_type: BonusGrantType::AmbientOnly,
                reason: "ambient trial credits".to_string(),
                user_facing_message: None,
                request_credits_granted: 1000,
                request_credits_remaining: 1000,
                scope: BonusGrantScope::User,
            }];

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false when only ambient-only bonus credits exist",
            );
        });
    });
}
