use ai::LLMId;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warpui_core::{App, Entity, ModelHandle};

use crate::OnboardingIntention;
use crate::model::{
    AiSetupChoice, NoAiConfirmationSource, OnboardingAuthState, OnboardingStateEvent,
    OnboardingStateModel, OnboardingStep, SelectedSettings,
};
use crate::slides::OfferVariant;

fn add_test_model(app: &mut App) -> ModelHandle<OnboardingStateModel> {
    app.update(MockTelemetryContextProvider::register);
    add_model(app)
}

#[test]
fn pricing_promotion_message_can_be_replaced_and_cleared() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        model.read(&app, |model, _| {
            assert_eq!(model.pricing_promotion_message(), None);
        });

        model.update(&mut app, |model, ctx| {
            model.set_pricing_promotion_message(Some("50% off Fable and Opus 5".to_string()), ctx)
        });
        model.read(&app, |model, _| {
            assert_eq!(
                model.pricing_promotion_message(),
                Some("50% off Fable and Opus 5")
            );
        });

        model.update(&mut app, |model, ctx| {
            model.set_pricing_promotion_message(None, ctx)
        });
        model.read(&app, |model, _| {
            assert_eq!(model.pricing_promotion_message(), None);
        });
    });
}

fn add_model(app: &mut App) -> ModelHandle<OnboardingStateModel> {
    app.add_model(|_| {
        OnboardingStateModel::new(
            Vec::new(),
            LLMId::from("auto"),
            false,
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

/// A do-nothing model used only to count the completion events the onboarding
/// model emits. Completion is an event rather than a state change, so it can't
/// be read back off the model itself.
#[derive(Default)]
struct CompletionObserver {
    completions: usize,
}

impl Entity for CompletionObserver {
    type Event = ();
}

fn observe_completions(
    app: &mut App,
    model: &ModelHandle<OnboardingStateModel>,
) -> ModelHandle<CompletionObserver> {
    let model = model.clone();
    app.add_model(move |ctx| {
        ctx.subscribe_to_model(&model, |observer: &mut CompletionObserver, _, event, _| {
            if matches!(event, OnboardingStateEvent::AiSellOfferSatisfied) {
                observer.completions += 1;
            }
        });
        CompletionObserver::default()
    })
}

fn completions(app: &App, observer: &ModelHandle<CompletionObserver>) -> usize {
    observer.read(app, |observer, _| observer.completions)
}

/// The availability report rides along on a generic usage refresh, so it must
/// be inert while no AI-sell offer is on screen.
#[test]
fn observing_availability_outside_the_offer_does_nothing() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        let observer = observe_completions(&mut app, &model);

        model.update(&mut app, |model, ctx| {
            model.on_credit_availability_observed(true, ctx)
        });
        assert_eq!(completions(&app, &observer), 0);
    });
}

/// The user leaves the offer through the plan call to action and buys on the
/// web, so nothing was ever recorded client-side. Completion has to come from
/// the account having AI (REV-1952).
#[test]
fn credit_availability_advances_the_ai_sell_offer() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        let observer = observe_completions(&mut app, &model);
        model.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
        });

        model.update(&mut app, |model, ctx| {
            model.on_credit_availability_observed(false, ctx)
        });
        assert_eq!(
            completions(&app, &observer),
            0,
            "a user who still can't use AI must stay on the offer"
        );

        model.update(&mut app, |model, ctx| {
            model.on_credit_availability_observed(true, ctx)
        });
        assert_eq!(completions(&app, &observer), 1);
    });
}

/// Regression test for REV-1952: following the confirmation page's link back
/// into the app advances onboarding, so the flow no longer stalls on the offer
/// while the credit grant catches up.
#[test]
fn the_checkout_success_handoff_advances_the_ai_sell_offer() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        let observer = observe_completions(&mut app, &model);
        model.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
        });

        let advanced = model.update(&mut app, |model, ctx| model.on_checkout_succeeded(ctx));
        assert!(advanced);
        assert_eq!(completions(&app, &observer), 1);
    });
}

/// The hand-off arrives on a generic deeplink, so it must be inert anywhere the
/// user isn't being sold AI: before the offer is shown, and on the head-start
/// offer, whose account already includes AI usage.
#[test]
fn the_checkout_success_handoff_is_inert_outside_an_ai_sell_offer() {
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);
        let observer = observe_completions(&mut app, &model);

        let advanced = model.update(&mut app, |model, ctx| model.on_checkout_succeeded(ctx));
        assert!(!advanced, "no offer is showing yet");

        model.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::HeadStart, ctx);
        });
        let advanced = model.update(&mut app, |model, ctx| model.on_checkout_succeeded(ctx));
        assert!(!advanced, "the head-start offer is not selling AI usage");

        model.update(&mut app, |model, ctx| {
            model.on_credit_availability_observed(true, ctx)
        });
        assert_eq!(completions(&app, &observer), 0);
    });
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
