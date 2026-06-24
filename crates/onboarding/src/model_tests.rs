use ai::LLMId;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warpui_core::{App, ModelHandle};

use crate::model::{
    AiSetupChoice, NoAiConfirmationSource, OnboardingAuthState, OnboardingStateModel,
    OnboardingStep, SelectedSettings,
};
use crate::OnboardingIntention;

fn add_test_model(app: &mut App) -> ModelHandle<OnboardingStateModel> {
    app.update(MockTelemetryContextProvider::register);
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
fn third_party_choice_disables_warp_ai() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app);

        // Default agent intention + "Use Warp Agent" keeps Warp AI on.
        model.read(&app, |model, _| assert!(model.settings().is_ai_enabled()));

        // "Use third party agents" turns Warp AI off.
        model.update(&mut app, |model, ctx| {
            model.set_ai_setup_choice(AiSetupChoice::ThirdParty, ctx)
        });
        model.read(&app, |model, _| assert!(!model.settings().is_ai_enabled()));

        // Switching back to Warp Agent re-enables it.
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
