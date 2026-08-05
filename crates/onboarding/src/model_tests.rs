use ai::LLMId;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warpui_core::{App, ModelHandle};

use crate::OnboardingIntention;
use crate::model::{
    AiSetupChoice, CreditPackOption, CreditPurchaseState, NoAiConfirmationSource,
    OnboardingAuthState, OnboardingStateModel, OnboardingStep, SelectedSettings,
};
use crate::slides::OfferVariant;

fn add_test_model(app: &mut App) -> ModelHandle<OnboardingStateModel> {
    app.update(MockTelemetryContextProvider::register);
    add_model(app)
}

fn add_model(app: &mut App) -> ModelHandle<OnboardingStateModel> {
    app.add_model(|_| {
        OnboardingStateModel::new(
            Vec::new(),
            LLMId::from("auto"),
            false,
            true,
            OnboardingAuthState::FreeUser,
        )
    })
}

fn step(app: &App, model: &ModelHandle<OnboardingStateModel>) -> OnboardingStep {
    model.read(app, |model, _| model.step())
}

#[test]
fn account_first_path_is_linear_and_reversible() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(true);
    let _settings_modes = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);

        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::ThemePicker);

        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);

        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Intro);
    });
}

#[test]
fn post_auth_offer_is_unclassified_until_selected_and_does_not_switch() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.read(&app, |model, _| {
            assert_eq!(model.offer_variant(), None);
        });
        model.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::HeadStart, ctx);
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
        });

        assert_eq!(step(&app, &model), OnboardingStep::PostAuthOffer);
        model.read(&app, |model, _| {
            assert_eq!(model.offer_variant(), Some(OfferVariant::HeadStart));
        });
    });
}

#[test]
fn post_auth_offer_supports_back_to_theme_and_no_direct_next() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(true);
    App::test((), |mut app| async move {
        app.update(MockTelemetryContextProvider::register);
        for variant in [OfferVariant::HeadStart, OfferVariant::ChooseHowToStart] {
            let model = add_model(&mut app);
            model.update(&mut app, |model, ctx| {
                model.show_post_auth_offer(variant, ctx);
            });

            assert_eq!(step(&app, &model), OnboardingStep::PostAuthOffer);
            model.read(&app, |model, _| {
                assert_eq!(model.offer_variant(), Some(variant));
                assert_eq!(model.progress(), (0, 0));
            });

            model.update(&mut app, |model, ctx| model.back(ctx));
            assert_eq!(step(&app, &model), OnboardingStep::ThemePicker);

            model.update(&mut app, |model, ctx| {
                model.show_post_auth_offer(variant, ctx);
                model.next(ctx);
            });
            assert_eq!(step(&app, &model), OnboardingStep::PostAuthOffer);
        }
    });
}

fn credit_packs() -> Vec<CreditPackOption> {
    vec![
        CreditPackOption {
            credits: 400,
            price_usd_cents: 1_200,
            savings_percent: 0,
        },
        CreditPackOption {
            credits: 1_000,
            price_usd_cents: 2_400,
            savings_percent: 20,
        },
    ]
}

fn purchase_state(app: &App, model: &ModelHandle<OnboardingStateModel>) -> CreditPurchaseState {
    model.read(app, |model, _| model.credit_purchase_state())
}

#[test]
fn credit_packs_default_to_the_first_option_and_are_selectable() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.read(&app, |model, _| {
            assert!(model.credit_pack_options().is_empty());
            assert_eq!(model.selected_credit_pack(), None);
        });

        model.update(&mut app, |model, ctx| {
            model.set_credit_pack_options(credit_packs(), ctx)
        });
        model.read(&app, |model, _| {
            assert_eq!(model.selected_credit_pack_index(), 0);
            assert_eq!(model.selected_credit_pack().map(|p| p.credits), Some(400));
        });

        model.update(&mut app, |model, ctx| model.select_credit_pack(1, ctx));
        model.read(&app, |model, _| {
            assert_eq!(model.selected_credit_pack().map(|p| p.credits), Some(1_000));
        });

        // Out-of-range selections are ignored rather than panicking.
        model.update(&mut app, |model, ctx| model.select_credit_pack(9, ctx));
        model.read(&app, |model, _| {
            assert_eq!(model.selected_credit_pack_index(), 1);
        });
    });
}

/// Regression test for REV-1886: browser checkout must not advance onboarding
/// on its own. The purchase stays in flight until the credits actually land,
/// so abandoning checkout leaves the user on the offer slide.
#[test]
fn abandoned_checkout_leaves_the_purchase_in_flight() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
            model.set_credit_pack_options(credit_packs(), ctx);
            model.request_credit_purchase(ctx);
        });
        assert_eq!(
            purchase_state(&app, &model),
            CreditPurchaseState::Purchasing
        );

        model.update(&mut app, |model, ctx| model.on_credit_checkout_opened(ctx));
        assert_eq!(
            purchase_state(&app, &model),
            CreditPurchaseState::AwaitingCheckout
        );
        assert_eq!(step(&app, &model), OnboardingStep::PostAuthOffer);

        // Only the server reporting AI as available clears the in-flight
        // purchase.
        model.update(&mut app, |model, ctx| {
            model.on_credit_availability_observed(true, ctx)
        });
        assert_eq!(purchase_state(&app, &model), CreditPurchaseState::Idle);
    });
}

/// Regression test for REV-1886: cancelling browser checkout must leave the
/// user on the offer slide. The common case is a brand-new account that still
/// can't make an AI request, so every refresh while checkout is open reports
/// unavailable and the slide must hold.
#[test]
fn canceled_checkout_does_not_advance_a_user_without_ai_access() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
            model.set_credit_pack_options(credit_packs(), ctx);
            model.request_credit_purchase(ctx);
            model.on_credit_checkout_opened(ctx);
        });

        // Every refresh while checkout is open still reports no AI access.
        for _ in 0..3 {
            model.update(&mut app, |model, ctx| {
                model.on_credit_availability_observed(false, ctx)
            });
            assert_eq!(
                purchase_state(&app, &model),
                CreditPurchaseState::AwaitingCheckout,
                "an unavailable answer must not complete the purchase"
            );
            assert_eq!(step(&app, &model), OnboardingStep::PostAuthOffer);
        }

        // Access arriving completes it.
        model.update(&mut app, |model, ctx| {
            model.on_credit_availability_observed(true, ctx)
        });
        assert_eq!(purchase_state(&app, &model), CreditPurchaseState::Idle);
    });
}

/// Onboarding doesn't care *how* the user ended up able to use AI — a team
/// plan landing mid-checkout counts just as much as the add-on credits they
/// were buying. The bar is "can make an AI request", not "this purchase
/// settled".
#[test]
fn access_arriving_from_any_source_completes_the_purchase() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
            model.set_credit_pack_options(credit_packs(), ctx);
            model.request_credit_purchase(ctx);
            model.on_credit_checkout_opened(ctx);
            // Not the add-on credits: some other grant made AI usable.
            model.on_credit_availability_observed(true, ctx);
        });
        assert_eq!(purchase_state(&app, &model), CreditPurchaseState::Idle);
    });
}

/// The availability report rides along on a generic usage refresh, so it must
/// be inert outside a pending checkout.
#[test]
fn observing_availability_outside_checkout_does_nothing() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| {
            model.set_credit_pack_options(credit_packs(), ctx);
            model.on_credit_availability_observed(true, ctx);
        });
        assert_eq!(purchase_state(&app, &model), CreditPurchaseState::Idle);

        // Still inert while the purchase mutation is in flight: that path
        // completes on the server's explicit success, not on an availability
        // read.
        model.update(&mut app, |model, ctx| {
            model.request_credit_purchase(ctx);
            model.on_credit_availability_observed(true, ctx);
        });
        assert_eq!(
            purchase_state(&app, &model),
            CreditPurchaseState::Purchasing
        );
    });
}

#[test]
fn a_synchronous_purchase_completes_without_checkout() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
            model.set_credit_pack_options(credit_packs(), ctx);
            model.request_credit_purchase(ctx);
            model.on_credit_purchase_completed(ctx);
        });
        assert_eq!(purchase_state(&app, &model), CreditPurchaseState::Idle);
    });
}

#[test]
fn a_rejected_purchase_is_retryable() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| {
            model.set_credit_pack_options(credit_packs(), ctx);
            model.request_credit_purchase(ctx);
            model.on_credit_purchase_failed(ctx);
        });
        assert_eq!(purchase_state(&app, &model), CreditPurchaseState::Failed);

        model.update(&mut app, |model, ctx| model.request_credit_purchase(ctx));
        assert_eq!(
            purchase_state(&app, &model),
            CreditPurchaseState::Purchasing
        );
    });
}

#[test]
fn a_purchase_cannot_start_without_packs_or_while_one_is_in_flight() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        // No packs offered yet: nothing to buy.
        model.update(&mut app, |model, ctx| model.request_credit_purchase(ctx));
        assert_eq!(purchase_state(&app, &model), CreditPurchaseState::Idle);

        model.update(&mut app, |model, ctx| {
            model.set_credit_pack_options(credit_packs(), ctx);
            model.request_credit_purchase(ctx);
            model.on_credit_checkout_opened(ctx);
            // A second request must not restart checkout...
            model.request_credit_purchase(ctx);
            // ...and the pack being paid for must not change underneath it.
            model.select_credit_pack(1, ctx);
        });
        assert_eq!(
            purchase_state(&app, &model),
            CreditPurchaseState::AwaitingCheckout
        );
        model.read(&app, |model, _| {
            assert_eq!(model.selected_credit_pack_index(), 0);
        });
    });
}

/// Completion callbacks are safe to fire speculatively (they are driven by a
/// generic usage refresh), so they must be inert when nothing was purchased.
#[test]
fn purchase_callbacks_are_inert_when_no_purchase_is_in_flight() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| {
            model.set_credit_pack_options(credit_packs(), ctx);
            model.on_credit_purchase_completed(ctx);
            model.on_credit_checkout_opened(ctx);
            model.on_credit_purchase_failed(ctx);
        });
        assert_eq!(purchase_state(&app, &model), CreditPurchaseState::Idle);
    });
}

#[test]
fn credit_pack_labels_are_formatted_for_display() {
    let pack = CreditPackOption {
        credits: 6_500,
        price_usd_cents: 12_000,
        savings_percent: 38,
    };
    assert_eq!(pack.credits_label(), "6,500");
    assert_eq!(pack.price_label(), "$120");

    let fractional = CreditPackOption {
        credits: 400,
        price_usd_cents: 1_250,
        savings_percent: 0,
    };
    assert_eq!(fractional.credits_label(), "400");
    assert_eq!(fractional.price_label(), "$12.50");
}

#[test]
fn account_first_path_uses_three_step_progress() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        let cases = [
            (OnboardingStep::Intro, (0, 3)),
            (OnboardingStep::Customize, (0, 3)),
            (OnboardingStep::ThemePicker, (1, 3)),
        ];

        for (target, expected) in cases {
            model.update(&mut app, |model, ctx| model.set_step(target, ctx));
            let progress = model.read(&app, |model, _| model.progress());
            assert_eq!(progress, expected, "unexpected dots for {target:?}");
        }
    });
}

#[test]
fn account_first_path_uses_agent_ui_defaults() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        model.read(&app, |model, _| {
            assert_eq!(
                *model.intention(),
                OnboardingIntention::AgentDrivenDevelopment
            );
            let SelectedSettings::AgentDrivenDevelopment {
                ui_customization: Some(ui),
                ..
            } = model.settings()
            else {
                panic!("account-first onboarding should preserve agent UI defaults");
            };
            assert!(ui.use_vertical_tabs);
            assert!(ui.show_conversation_history);
            assert!(ui.show_project_explorer);
            assert!(ui.show_global_search);
            assert!(ui.show_warp_drive);
            assert!(ui.show_code_review_button);
        });
    });
}

#[test]
fn agent_path_routes_through_ai_setup() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        // Default intention is agent-driven development.
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Intention);
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::AiSetup);

        // The default AI setup choice is the Warp agent.
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Agent);
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::AiAccess);
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::ThemePicker);

        // Back navigation mirrors the forward path.
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::AiAccess);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Agent);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::AiSetup);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Intention);
    });
}

#[test]
fn third_party_choice_routes_to_third_party_slide() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.next(ctx); // Intro → Intention
            model.next(ctx); // Intention → AiSetup
            model.set_ai_setup_choice(AiSetupChoice::ThirdParty, ctx);
            model.next(ctx); // AiSetup → ThirdParty
        });
        assert_eq!(step(&app, &model), OnboardingStep::ThirdParty);

        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);

        // Back from Customize returns to the chosen AI-setup slide.
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::ThirdParty);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::AiSetup);
    });
}

#[test]
fn confirm_no_ai_switches_to_terminal_path() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.next(ctx); // Intro → Intention
            model.set_intention_terminal(ctx);
            model.request_no_ai_confirmation(NoAiConfirmationSource::Intention, ctx);
        });

        // The confirmation modal is shown without leaving the intention slide yet.
        assert_eq!(step(&app, &model), OnboardingStep::Intention);
        model.read(&app, |model, _| {
            assert_eq!(
                model.no_ai_confirmation(),
                Some(NoAiConfirmationSource::Intention)
            );
        });

        // Confirming "I don't want AI" lands on the terminal path, never a dead end.
        model.update(&mut app, |model, ctx| model.confirm_no_ai(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);
        model.read(&app, |model, _| {
            assert_eq!(model.no_ai_confirmation(), None);
            assert_eq!(*model.intention(), OnboardingIntention::Terminal);
            assert!(!model.settings().is_ai_enabled());
        });

        // The terminal path continues to completion, skipping the third-party slide.
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::ThemePicker);
    });
}

#[test]
fn confirm_no_ai_from_intention_then_back_returns_to_intention() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.next(ctx); // Intro → Intention
            model.set_intention_terminal(ctx);
            model.request_no_ai_confirmation(NoAiConfirmationSource::Intention, ctx);
        });

        // "Just use the terminal" + Next does not advance until the user confirms.
        assert_eq!(step(&app, &model), OnboardingStep::Intention);

        model.update(&mut app, |model, ctx| model.confirm_no_ai(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);

        // Back from Customize goes to the intention fork, not the AI-setup slide.
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Intention);
    });
}

#[test]
fn cancel_no_ai_from_intention_routes_to_ai_setup() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.next(ctx); // Intro → Intention
            model.set_intention_terminal(ctx);
            model.request_no_ai_confirmation(NoAiConfirmationSource::Intention, ctx);
        });

        // "Give me AI features" switches onto the AI path and opens the AI-setup slide.
        model.update(&mut app, |model, ctx| model.cancel_no_ai(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::AiSetup);
        model.read(&app, |model, _| {
            assert_eq!(model.no_ai_confirmation(), None);
            assert_eq!(
                *model.intention(),
                OnboardingIntention::AgentDrivenDevelopment
            );
        });
    });
}

#[test]
fn dismiss_no_ai_closes_without_changing_path() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        model.update(&mut app, |model, ctx| {
            model.next(ctx); // Intro → Intention
            model.set_intention_terminal(ctx);
            model.request_no_ai_confirmation(NoAiConfirmationSource::Intention, ctx);
            model.dismiss_no_ai(ctx);
        });

        assert_eq!(step(&app, &model), OnboardingStep::Intention);
        model.read(&app, |model, _| {
            assert_eq!(model.no_ai_confirmation(), None);
            assert_eq!(*model.intention(), OnboardingIntention::Terminal);
        });
    });
}

#[test]
fn terminal_settings_disable_ai() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| model.set_intention_terminal(ctx));
        model.read(&app, |model, _| {
            assert!(matches!(
                model.settings(),
                SelectedSettings::Terminal { .. }
            ));
            assert!(!model.settings().is_ai_enabled());
        });
    });
}

#[test]
fn agent_intent_keeps_ai_enabled_for_any_setup_choice() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        // Default agent intention + "Use Warp Agent" enables AI.
        model.read(&app, |model, _| assert!(model.settings().is_ai_enabled()));

        // "Use third party agents" still keeps AI enabled: agent intent always
        // means the user wants AI, even when bringing their own agents.
        model.update(&mut app, |model, ctx| {
            model.set_ai_setup_choice(AiSetupChoice::ThirdParty, ctx)
        });
        model.read(&app, |model, _| assert!(model.settings().is_ai_enabled()));

        // Switching back to Warp Agent also keeps AI enabled.
        model.update(&mut app, |model, ctx| {
            model.set_ai_setup_choice(AiSetupChoice::WarpAgent, ctx)
        });
        model.read(&app, |model, _| assert!(model.settings().is_ai_enabled()));
    });
}

#[test]
fn terminal_path_skips_third_party() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| model.set_intention_terminal(ctx));

        // Terminal goes Intention → Customize → ThemePicker; the "Customize third
        // party agents" slide is only for the agent → third-party choice.
        for expected in [
            OnboardingStep::Intention,
            OnboardingStep::Customize,
            OnboardingStep::ThemePicker,
        ] {
            model.update(&mut app, |model, ctx| model.next(ctx));
            assert_eq!(step(&app, &model), expected);
        }

        // Back navigation mirrors the forward path, also skipping third-party.
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Intention);
    });
}

#[test]
fn progress_reports_v3_positions_for_agent_path() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        // Warp Agent path: Intention → AiSetup → Agent → AiAccess → Customize → ThemePicker.
        let cases = [
            (OnboardingStep::Intention, (0, 6)),
            (OnboardingStep::AiSetup, (1, 6)),
            (OnboardingStep::Agent, (2, 6)),
            (OnboardingStep::AiAccess, (3, 6)),
            (OnboardingStep::Customize, (4, 6)),
            (OnboardingStep::ThemePicker, (5, 6)),
        ];
        for (target, expected) in cases {
            model.update(&mut app, |model, ctx| model.set_step(target, ctx));
            let progress = model.read(&app, |model, _| model.progress());
            assert_eq!(progress, expected, "unexpected dots for {target:?}");
        }
    });
}

#[test]
fn progress_reports_v3_positions_for_third_party_path() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| {
            model.set_ai_setup_choice(AiSetupChoice::ThirdParty, ctx)
        });

        // Third-party path has no "Choose how to access AI" step, so it is one
        // dot shorter than the Warp Agent path.
        let cases = [
            (OnboardingStep::Intention, (0, 5)),
            (OnboardingStep::AiSetup, (1, 5)),
            (OnboardingStep::ThirdParty, (2, 5)),
            (OnboardingStep::Customize, (3, 5)),
            (OnboardingStep::ThemePicker, (4, 5)),
        ];
        for (target, expected) in cases {
            model.update(&mut app, |model, ctx| model.set_step(target, ctx));
            let progress = model.read(&app, |model, _| model.progress());
            assert_eq!(progress, expected, "unexpected dots for {target:?}");
        }
    });
}

#[test]
fn progress_reports_terminal_path_uses_three_dot_variant() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.update(&mut app, |model, ctx| model.set_intention_terminal(ctx));
        let cases = [
            (OnboardingStep::Intention, (0, 3)),
            (OnboardingStep::Customize, (1, 3)),
            (OnboardingStep::ThemePicker, (2, 3)),
        ];
        for (target, expected) in cases {
            model.update(&mut app, |model, ctx| model.set_step(target, ctx));
            let progress = model.read(&app, |model, _| model.progress());
            assert_eq!(progress, expected, "unexpected dots for {target:?}");
        }
    });
}
