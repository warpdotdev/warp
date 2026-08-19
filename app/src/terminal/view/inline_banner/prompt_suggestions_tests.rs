use std::sync::Arc;

use ai::api_keys::ApiKeyManager;
use warp_core::features::FeatureFlag;
use warpui::App;

use super::*;
use crate::ai::PromptSuggestionAllowance;
use crate::ai::request_usage_model::RequestLimitInfo;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;

/// Registers the minimal set of singletons `should_open_unavailable_modal`
/// and `should_accept_via_suggestion_allowance` read: `UserWorkspaces` (no
/// workspace, i.e. a teamless Free user) and `AIRequestUsageModel` seeded
/// with `request_limit: 0` (no interactive credits) and the given
/// prompt-suggestion allowance.
fn initialize_app_with_allowance(
    app: &mut App,
    prompt_suggestion_allowance: Option<PromptSuggestionAllowance>,
) {
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![],
            ctx,
        )
    });
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    if app
        .models_of_type::<settings::PrivatePreferences>()
        .is_empty()
    {
        app.update(crate::settings::init_and_register_user_preferences);
    }
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
        ctx.add_singleton_model(ApiKeyManager::new);
    });
    app.add_singleton_model(crate::settings::AISettings::new_with_defaults);
    app.add_singleton_model(|_| crate::pricing::PricingInfoModel::new());
    let request_usage_model = app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    request_usage_model.update(app, |model, ctx| {
        model.update_request_limit_info(
            RequestLimitInfo {
                prompt_suggestion_allowance,
                ..RequestLimitInfo::new_for_test(0, 0)
            },
            ctx,
        );
    });
}

#[test]
fn should_open_unavailable_modal_is_false_when_suggestion_remaining_is_positive() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
        initialize_app_with_allowance(
            &mut app,
            Some(PromptSuggestionAllowance {
                limit: 300,
                used: 100,
            }),
        );

        app.read(|ctx| {
            assert!(
                !should_open_unavailable_modal(&PromptAlertState::RequestLimitReached, ctx),
                "remaining > 0 must never open the interactive unavailable modal"
            );
            assert!(
                should_accept_via_suggestion_allowance(&PromptAlertState::RequestLimitReached, ctx),
                "remaining > 0 must allow the accept to go through"
            );
        });
    })
}

/// PRODUCT.md gates the click on remaining > 0 *and* the other accept rules
/// passing: a positive suggestion allowance must not override any disabled
/// state besides `RequestLimitReached`. Free users who are offline or
/// delinquent must stay blocked (with their own tooltip) even with credits.
#[test]
fn should_accept_via_suggestion_allowance_is_false_for_other_disabled_states_even_with_remaining() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
        initialize_app_with_allowance(
            &mut app,
            Some(PromptSuggestionAllowance {
                limit: 300,
                used: 100,
            }),
        );

        app.read(|ctx| {
            for state in [
                PromptAlertState::NoConnection,
                PromptAlertState::DelinquentDueToPaymentIssue,
                PromptAlertState::AnonymousUserRequestLimitHardGate,
                PromptAlertState::OveragesToggleableButNotEnabled,
                PromptAlertState::MonthlyOveragesSpendLimitReached,
            ] {
                assert!(
                    !should_accept_via_suggestion_allowance(&state, ctx),
                    "a positive suggestion allowance must not override the {state:?} disabled state"
                );
                assert!(
                    !should_open_unavailable_modal(&state, ctx),
                    "the interactive unavailable modal is specific to RequestLimitReached, not {state:?}"
                );
            }
        });
    })
}

#[test]
fn should_open_unavailable_modal_is_false_when_suggestion_allowance_is_exhausted() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
        initialize_app_with_allowance(
            &mut app,
            Some(PromptSuggestionAllowance {
                limit: 300,
                used: 300,
            }),
        );

        app.read(|ctx| {
            // Exhaustion has its own hover/disabled treatment; it must not
            // fall back to the generic interactive-credits modal either.
            assert!(!should_open_unavailable_modal(
                &PromptAlertState::RequestLimitReached,
                ctx
            ));
            assert!(!should_accept_via_suggestion_allowance(
                &PromptAlertState::RequestLimitReached,
                ctx
            ));
        });
    })
}

/// Regression guard: when this tier has no prompt-suggestion wallet at all
/// (`None`, e.g. paid, or an older server), the click gate must fall back to
/// exactly its pre-feature behavior (open the interactive unavailable modal).
#[test]
fn should_open_unavailable_modal_is_true_when_no_suggestion_allowance_exists() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
        initialize_app_with_allowance(&mut app, None);

        app.read(|ctx| {
            assert!(should_open_unavailable_modal(
                &PromptAlertState::RequestLimitReached,
                ctx
            ));
            assert!(!should_accept_via_suggestion_allowance(
                &PromptAlertState::RequestLimitReached,
                ctx
            ));
        });
    })
}

/// A null allowance (paid tier) leaves today's behavior on other alert
/// states (e.g. payment issues) untouched: the modal stays closed for states
/// other than `RequestLimitReached` regardless of the suggestion wallet.
#[test]
fn should_open_unavailable_modal_is_false_for_non_request_limit_alert_states() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
        initialize_app_with_allowance(&mut app, None);

        app.read(|ctx| {
            assert!(!should_open_unavailable_modal(
                &PromptAlertState::DelinquentDueToPaymentIssue,
                ctx
            ));
        });
    })
}
