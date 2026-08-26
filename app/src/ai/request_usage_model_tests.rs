use std::sync::Arc;
use std::time::SystemTime;

use ai::LLMProvider;
use ai::api_keys::{ApiKeyManager, AwsCredentials, AwsCredentialsState, GrokTokens};
use chrono::Duration;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warp_graphql::billing::{AddonCreditsOption, OveragesPricing, PricingInfo};
use warpui::{App, ModelHandle};

use super::*;
use crate::ai::credit_availability::{AICreditAvailability, AICreditDenialReason, AICreditSource};
use crate::auth::AuthStateProvider;
use crate::pricing::PricingInfoModel;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::MockAIClient;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    AiOverages, ByoApiKeyPolicy, CustomerType, EnterpriseCreditsAutoReloadPolicy,
    EnterprisePayAsYouGoPolicy, PurchaseAddOnCreditsPolicy, Workspace, WorkspaceUid,
};

fn create_test_workspace() -> (WorkspaceUid, Workspace) {
    let server_id: crate::server::ids::ServerId = 1_i64.into();
    let uid = WorkspaceUid::from(server_id);
    let workspace = Workspace::from_local_cache(uid, "Test Workspace".to_string(), None, None);
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

fn add_request_usage_model_for_logged_out_users(app: &mut App) -> ModelHandle<AIRequestUsageModel> {
    app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
    add_request_usage_model_without_auth(app)
}
fn register_user_preferences_for_tests(app: &mut App) {
    if app
        .models_of_type::<settings::PrivatePreferences>()
        .is_empty()
    {
        app.update(crate::settings::init_and_register_user_preferences);
    }
}

fn add_request_usage_model_without_auth(app: &mut App) -> ModelHandle<AIRequestUsageModel> {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    register_user_preferences_for_tests(app);
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
        MockTelemetryContextProvider::register(ctx);
        ctx.add_singleton_model(ApiKeyManager::new);
    });
    app.add_singleton_model(|_| PricingInfoModel::new());
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    })
}

/// Registers the request usage model backed by an explicit AI client so tests
/// can control the availability fetch behavior.
fn add_request_usage_model_with_client(
    app: &mut App,
    ai_client: Arc<dyn AIClient>,
) -> ModelHandle<AIRequestUsageModel> {
    register_user_preferences_for_tests(app);
    app.add_singleton_model(|ctx| AIRequestUsageModel::new_for_test(ai_client, ctx))
}

fn set_addon_credits_pricing_info(app: &mut App) {
    PricingInfoModel::handle(app).update(app, |model, ctx| {
        model.update_pricing_info(
            PricingInfo {
                plans: vec![],
                overages: OveragesPricing {
                    price_per_request_usd_cents: 1,
                },
                addon_credits_options: vec![AddonCreditsOption {
                    credits: 1000,
                    price_usd_cents: 1000,
                }],
                promotion_message: None,
            },
            ctx,
        );
    });
}

fn standard_purchase_policy() -> PurchaseAddOnCreditsPolicy {
    PurchaseAddOnCreditsPolicy {
        enabled: true,
        premium_enabled: false,
        price_premium_bps: 0,
    }
}

fn premium_purchase_policy() -> PurchaseAddOnCreditsPolicy {
    PurchaseAddOnCreditsPolicy {
        enabled: false,
        premium_enabled: true,
        price_premium_bps: 1000,
    }
}

fn enable_auto_reload(workspace: &mut Workspace) {
    workspace
        .settings
        .addon_credits_settings
        .auto_reload_enabled = true;
    workspace
        .settings
        .addon_credits_settings
        .selected_auto_reload_credit_denomination = Some(1000);
}

#[test]
fn refresh_request_usage_returns_no_fresh_limit_when_logged_out() {
    App::test((), |mut app| async move {
        let request_usage_model = add_request_usage_model_for_logged_out_users(&mut app);
        let refresh =
            request_usage_model.update(&mut app, |model, ctx| model.refresh_request_usage(ctx));

        assert_eq!(refresh.await.unwrap(), None);
    });
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
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());

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
fn test_buy_credits_banner_shows_for_premium_enabled_plan_out_of_credits() {
    App::test((), |mut app| async move {
        // The test workspace has no teams: this covers the teamless fresh
        // free user, whose purchase policy lives on the workspace billing
        // metadata until their first purchase creates a team server-side.
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(premium_purchase_policy());

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            assert!(
                !UserWorkspaces::as_ref(ctx).has_teams(),
                "this test covers the teamless case"
            );
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::OutOfCredits,
            );
        });
    });
}

#[test]
fn test_buy_credits_banner_hidden_when_policy_fully_disabled() {
    App::test((), |mut app| async move {
        // Also a teamless workspace: without premiumEnabled the purchase
        // surfaces must stay hidden for fresh free users.
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(PurchaseAddOnCreditsPolicy {
            enabled: false,
            premium_enabled: false,
            price_premium_bps: 0,
        });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::Hidden,
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
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());

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
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());

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
fn test_buy_credits_banner_hidden_when_server_reports_available() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // Local base quota is exhausted; without server availability the
            // banner would show. A non-ambient server source must hide it.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();
            model.apply_server_availability(
                Ok(AICreditAvailability::available_with_source(Some(
                    AICreditSource::BonusGrant,
                ))),
                ctx,
            );

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::Hidden,
            );
        });
    });
}

#[test]
fn test_buy_credits_banner_shows_when_server_reports_out_of_credits() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // Stale local base quota must not suppress the banner once the
            // server has denied interactive AI.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 5);
            model.bonus_grants.clear();
            model.apply_server_availability(
                Ok(AICreditAvailability::unavailable(
                    AICreditDenialReason::OutOfCredits,
                )),
                ctx,
            );

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::OutOfCredits,
            );
        });
    });
}

#[test]
fn test_buy_credits_banner_shows_when_server_source_is_ambient_only() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();
            model.apply_server_availability(
                Ok(AICreditAvailability::available_with_source(Some(
                    AICreditSource::AmbientBonusGrant,
                ))),
                ctx,
            );

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::OutOfCredits,
            );
        });
    });
}

#[test]
fn test_buy_credits_banner_hidden_when_out_of_credits_refined_by_local_byo() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_provider_key(LLMProvider::OpenAI, Some("test-key".to_string()), ctx);
        });

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();
            model.apply_server_availability(
                Ok(AICreditAvailability::unavailable(
                    AICreditDenialReason::OutOfCredits,
                )),
                ctx,
            );

            assert!(
                model.has_any_ai_remaining(ctx),
                "local BYO should refine OutOfCredits into available AI"
            );
            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::Hidden,
            );
        });
    });
}

#[test]
fn test_buy_credits_banner_respects_monthly_limit_under_server_out_of_credits() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());
        enable_auto_reload(&mut workspace);
        // Zero monthly spend limit means any auto-reload is blocked.
        workspace
            .settings
            .addon_credits_settings
            .max_monthly_spend_cents = Some(0);

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);
        set_addon_credits_pricing_info(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();
            model.apply_server_availability(
                Ok(AICreditAvailability::unavailable(
                    AICreditDenialReason::OutOfCredits,
                )),
                ctx,
            );

            assert_eq!(
                model.compute_buy_addon_credits_banner_display_state(ctx),
                BuyCreditsBannerDisplayState::MonthlyLimitReached,
            );
        });
    });
}

#[test]
fn test_ambient_credits_banner_dismissal_is_persisted() {
    App::test((), |mut app| async move {
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            assert!(!model.is_ambient_credits_banner_dismissed());
            model.dismiss_ambient_credits_banner(ctx);
            assert!(model.is_ambient_credits_banner_dismissed());
        });

        app.update(|ctx| {
            let stored_value = ctx
                .private_user_preferences()
                .read_value(AMBIENT_CREDITS_BANNER_DISMISSED_KEY)
                .unwrap();
            assert_eq!(stored_value, Some("true".to_owned()));
        });
    });
}

#[test]
fn test_ambient_credits_banner_dismissal_loads_from_preferences() {
    App::test((), |mut app| async move {
        register_user_preferences_for_tests(&mut app);
        app.update(|ctx| {
            ctx.private_user_preferences()
                .write_value(AMBIENT_CREDITS_BANNER_DISMISSED_KEY, "true".to_owned())
                .unwrap();
        });

        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, _ctx| {
            assert!(model.is_ambient_credits_banner_dismissed());
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
fn test_total_workspace_and_team_bonus_credits_counts_both_scopes() {
    App::test((), |mut app| async move {
        let (uid, workspace) = create_test_workspace();
        let other_uid = WorkspaceUid::from(crate::server::ids::ServerId::from(2_i64));
        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, _ctx| {
            let make = |scope, remaining| BonusGrant {
                created_at: Utc::now(),
                cost_cents: 0,
                expiration: None,
                grant_type: BonusGrantType::Any,
                reason: "test".to_string(),
                user_facing_message: None,
                request_credits_granted: remaining,
                request_credits_remaining: remaining,
                scope,
            };
            model.bonus_grants = vec![
                make(BonusGrantScope::User, 5),
                make(BonusGrantScope::Team(uid), 7),
                make(BonusGrantScope::Workspace(uid), 11),
                make(BonusGrantScope::Team(other_uid), 13),
            ];

            assert_eq!(
                model.total_workspace_and_team_bonus_credits_remaining(uid),
                18
            );
            assert_eq!(model.total_user_interactive_bonus_credits_remaining(), 5);
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
fn test_has_any_ai_remaining_false_with_enterprise_auto_reload_policy_on_non_enterprise() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .enterprise_credits_auto_reload_policy =
            Some(EnterpriseCreditsAutoReloadPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false when enterprise auto-reload policy is enabled for a non-enterprise workspace",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_self_serve_auto_reload() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());
        enable_auto_reload(&mut workspace);

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);
        set_addon_credits_pricing_info(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when self-serve auto-reload is enabled",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_premium_auto_reload() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(premium_purchase_policy());
        enable_auto_reload(&mut workspace);

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);
        set_addon_credits_pricing_info(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when premium-plan auto-reload is enabled",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_false_when_premium_auto_reload_would_exceed_limit() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(premium_purchase_policy());
        enable_auto_reload(&mut workspace);
        // The reload's list price is $10.00 but the premium price is $11.00;
        // a $10.50 monthly limit only blocks the reload when the premium
        // surcharge is included in the check.
        workspace
            .settings
            .addon_credits_settings
            .max_monthly_spend_cents = Some(1050);

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);
        set_addon_credits_pricing_info(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false when the premium-priced reload would exceed the monthly spend limit",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_true_with_self_serve_auto_reload_and_billing_v2_disabled() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::BillingAndUsagePageV2.override_enabled(false);

        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());
        enable_auto_reload(&mut workspace);

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);
        set_addon_credits_pricing_info(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when self-serve auto-reload is enabled without Billing and Usage V2",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_false_with_add_on_credits_policy_when_purchase_would_exceed_limit() {
    App::test((), |mut app| async move {
        let (_uid, mut workspace) = create_test_workspace();
        workspace
            .billing_metadata
            .tier
            .purchase_add_on_credits_policy = Some(standard_purchase_policy());
        enable_auto_reload(&mut workspace);
        workspace
            .settings
            .addon_credits_settings
            .max_monthly_spend_cents = Some(1000);
        workspace.bonus_grants_purchased_this_month.cents_spent = 500;

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);
        set_addon_credits_pricing_info(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false when add-on credit purchase would exceed the monthly spend limit",
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
            manager.set_provider_key(LLMProvider::OpenAI, Some("test-key".to_string()), ctx);
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
fn test_has_any_ai_remaining_true_with_grok_subscription_connected() {
    App::test((), |mut app| async move {
        // Workspace with BYO enabled — the policy a connected Grok
        // subscription's OAuth token rides on.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        // Connect a Grok subscription but provide no pasted API key.
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_grok_tokens(
                Some(GrokTokens {
                    access_token: "grok-test-token".to_string(),
                    ..Default::default()
                }),
                ctx,
            );
        });

        request_usage_model.update(&mut app, |model, ctx| {
            // No standard requests remaining, no bonus credits.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be true when a Grok subscription is connected and BYO is enabled",
            );
        });
    });
}

#[test]
fn test_has_any_ai_remaining_false_with_grok_subscription_but_byo_disabled() {
    App::test((), |mut app| async move {
        // No BYO policy: the Grok token can't be sent, so it must not count as
        // available AI.
        let (_uid, workspace) = create_test_workspace();

        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_grok_tokens(
                Some(GrokTokens {
                    access_token: "grok-test-token".to_string(),
                    ..Default::default()
                }),
                ctx,
            );
        });

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.bonus_grants.clear();

            assert!(
                !model.has_any_ai_remaining(ctx),
                "expected has_any_ai_remaining to be false when a Grok subscription is connected but BYO is disabled",
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
            manager.set_provider_key(LLMProvider::OpenAI, Some("test-key".to_string()), ctx);
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
            manager.set_provider_key(LLMProvider::OpenAI, Some("test-key".to_string()), ctx);
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

#[test]
fn test_server_availability_overrides_locally_derived_state() {
    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // Local state says AI is available.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 5);
            assert!(model.has_any_ai_remaining(ctx));

            // The server-authoritative decision wins over local state.
            model.apply_server_availability(
                Ok(AICreditAvailability::unavailable(
                    AICreditDenialReason::OutOfCredits,
                )),
                ctx,
            );
            assert!(!model.has_any_ai_remaining(ctx));

            // And in the other direction: local state is exhausted, but the
            // server reports a usable fallback source.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.apply_server_availability(
                Ok(AICreditAvailability::available_with_source(Some(
                    AICreditSource::BonusGrant,
                ))),
                ctx,
            );
            assert!(model.has_any_ai_remaining(ctx));
        });
    });
}

#[test]
fn test_availability_refresh_failure_keeps_last_known_good() {
    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            // Local state says AI is available, so the pre-server-decision fallback would
            // report `true`.
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 5);

            let denied = AICreditAvailability::unavailable(AICreditDenialReason::Delinquent);
            model.apply_server_availability(Ok(denied), ctx);
            model.apply_server_availability(Err(anyhow::anyhow!("transient failure")), ctx);

            // The last-known-good server decision is retained: the error is
            // recorded but neither flips availability nor re-enables the
            // pre-server-decision fallback.
            assert_eq!(model.server_availability(), Some(denied));
            assert!(!model.has_any_ai_remaining(ctx));
            assert_eq!(
                model.server_availability.last_error.as_deref(),
                Some("transient failure")
            );
        });
    });
}

#[test]
fn test_availability_refresh_failure_before_first_success_uses_prefetch_fallback() {
    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 5);
            model.apply_server_availability(Err(anyhow::anyhow!("unsupported operation")), ctx);

            // Without any successful fetch (e.g. server doesn't support the
            // field yet), the pre-server-decision fallback still applies.
            assert_eq!(model.server_availability(), None);
            assert!(model.has_any_ai_remaining(ctx));
        });
    });
}

#[test]
fn test_reset_server_availability_restores_prefetch_fallback() {
    App::test((), |mut app| async move {
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 5);
            model.apply_server_availability(
                Ok(AICreditAvailability::unavailable(
                    AICreditDenialReason::OutOfCredits,
                )),
                ctx,
            );
            assert!(!model.has_any_ai_remaining(ctx));

            // On logout the server decision is cleared and the pre-server-decision
            // fallback is restored for the next principal.
            model.reset_server_availability(ctx);
            assert_eq!(model.server_availability(), None);
            assert!(model.has_any_ai_remaining(ctx));
        });
    });
}

#[test]
fn test_out_of_credits_refined_by_local_byo_key() {
    App::test((), |mut app| async move {
        // BYOK is allowed by policy, but no key has been stored locally.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });
        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        // OUT_OF_CREDITS means the server found no path it can see; locally
        // stored keys are request-level parameters invisible to it.
        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.apply_server_availability(
                Ok(AICreditAvailability::unavailable(
                    AICreditDenialReason::OutOfCredits,
                )),
                ctx,
            );
            assert!(
                !model.has_any_ai_remaining(ctx),
                "out of credits without a stored key should gate AI",
            );
        });

        // Storing a key supplies the one fact the server cannot know.
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_provider_key(LLMProvider::OpenAI, Some("test-key".to_string()), ctx);
        });
        request_usage_model.read(&app, |model, ctx| {
            assert!(
                model.has_any_ai_remaining(ctx),
                "out of credits with a stored key should permit AI",
            );
        });
    });
}

#[test]
fn test_out_of_credits_refined_by_local_bedrock_credentials() {
    App::test((), |mut app| async move {
        // Bedrock via the local AWS chain: the org enables the host, but the
        // credentials live on this machine.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.settings.llm_settings.enabled = true;
        workspace.settings.llm_settings.host_configs.insert(
            crate::ai::llms::LLMModelHost::AwsBedrock,
            crate::workspaces::workspace::LlmHostSettings {
                enabled: true,
                enablement_setting: crate::workspaces::workspace::HostEnablementSetting::Enforce,
                ..Default::default()
            },
        );
        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.apply_server_availability(
                Ok(AICreditAvailability::unavailable(
                    AICreditDenialReason::OutOfCredits,
                )),
                ctx,
            );
            assert!(!model.has_any_ai_remaining(ctx));
        });

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_aws_credentials_state(
                AwsCredentialsState::Loaded {
                    credentials: AwsCredentials::new(
                        "access".to_string(),
                        "secret".to_string(),
                        None,
                        None,
                    ),
                    loaded_at: SystemTime::now(),
                },
                ctx,
            );
        });
        request_usage_model.read(&app, |model, ctx| {
            assert!(model.has_any_ai_remaining(ctx));
        });
    });
}

#[test]
fn test_server_managed_availability_trusted_without_local_keys() {
    App::test((), |mut app| async move {
        // `available` with no credit source now means a server-managed BYO
        // path (team keys/endpoints or enterprise custom LLM) is configured;
        // the server knows this for a fact, so no local key is required.
        app.add_singleton_model(UserWorkspaces::default_mock);
        let request_usage_model = add_request_usage_model(&mut app);

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_limit_info = RequestLimitInfo::new_for_test(10, 10);
            model.apply_server_availability(
                Ok(AICreditAvailability::available_with_source(None)),
                ctx,
            );
            assert!(model.has_any_ai_remaining(ctx));
        });
    });
}

#[test]
fn test_server_unavailable_overrides_local_byo_key() {
    App::test((), |mut app| async move {
        // A locally stored key never overrides a hard server denial — the
        // local refinement applies only to OUT_OF_CREDITS.
        let (_uid, mut workspace) = create_test_workspace();
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });
        add_user_workspaces_with_workspace(&mut app, workspace);
        let request_usage_model = add_request_usage_model(&mut app);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_provider_key(LLMProvider::OpenAI, Some("test-key".to_string()), ctx);
        });

        request_usage_model.update(&mut app, |model, ctx| {
            model.apply_server_availability(
                Ok(AICreditAvailability::unavailable(
                    AICreditDenialReason::Delinquent,
                )),
                ctx,
            );
            assert!(!model.has_any_ai_remaining(ctx));
        });
    });
}

#[test]
fn test_availability_refresh_coalesces_concurrent_fetches() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());

        let mut ai_client = MockAIClient::new();
        // Exactly one fetch may go out even though two triggers fire while the
        // first request is still in flight.
        ai_client
            .expect_get_ai_credit_availability()
            .times(1)
            .returning(|| {
                Ok(AICreditAvailability::available_with_source(Some(
                    AICreditSource::BaseLimit,
                )))
            });
        let request_usage_model =
            add_request_usage_model_with_client(&mut app, Arc::new(ai_client));

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_availability_refresh(ctx);
            model.request_availability_refresh(ctx);
            assert!(model.server_availability.refresh_in_flight);
        });

        // Let the spawned fetch complete.
        warpui::r#async::Timer::after(std::time::Duration::from_millis(100)).await;

        request_usage_model.read(&app, |model, _| {
            assert!(!model.server_availability.refresh_in_flight);
            assert_eq!(
                model.server_availability(),
                Some(AICreditAvailability::available_with_source(Some(
                    AICreditSource::BaseLimit,
                )))
            );
        });
    });
}

#[test]
fn test_availability_refresh_skipped_when_logged_out() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());

        // No expectations: any fetch would panic the test.
        let ai_client = MockAIClient::new();
        let request_usage_model =
            add_request_usage_model_with_client(&mut app, Arc::new(ai_client));

        request_usage_model.update(&mut app, |model, ctx| {
            model.request_availability_refresh(ctx);
            assert!(!model.server_availability.refresh_in_flight);
        });

        request_usage_model.read(&app, |model, _| {
            assert_eq!(model.server_availability(), None);
        });
    });
}
