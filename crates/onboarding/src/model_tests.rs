use ai::LLMId;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warpui_core::{App, ModelHandle};

use crate::model::{
    AiSetupChoice, OnboardingAuthState, OnboardingStateModel, OnboardingStep, SelectedSettings,
};
use crate::OnboardingIntention;

fn add_test_model(
    app: &mut App,
    free_ai_removal_enrolled: bool,
) -> ModelHandle<OnboardingStateModel> {
    app.update(MockTelemetryContextProvider::register);
    app.add_model(|_| {
        OnboardingStateModel::new(
            Vec::new(),
            LLMId::from("auto"),
            false,
            true,
            free_ai_removal_enrolled,
            OnboardingAuthState::FreeUser,
        )
    })
}

fn step(app: &App, model: &ModelHandle<OnboardingStateModel>) -> OnboardingStep {
    model.read(app, |model, _| model.step())
}

#[test]
fn enrolled_agent_path_routes_through_ai_setup() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app, true);

        // Default intention is agent-driven development.
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Intention);
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::AiSetup);

        // The default AI setup choice is the Warp agent.
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Agent);
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::ThemePicker);

        // Back navigation mirrors the forward path.
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Customize);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Agent);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::AiSetup);
        model.update(&mut app, |model, ctx| model.back(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::Intention);
    });
}

#[test]
fn enrolled_third_party_choice_routes_to_third_party_slide() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app, true);

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
fn opt_out_of_ai_switches_to_terminal_path() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app, true);

        model.update(&mut app, |model, ctx| {
            model.next(ctx); // Intro → Intention
            model.next(ctx); // Intention → AiSetup
            model.opt_out_of_ai(ctx);
        });

        // "I don't want AI" lands on the terminal path, never a dead end.
        assert_eq!(step(&app, &model), OnboardingStep::Customize);
        model.read(&app, |model, _| {
            assert_eq!(*model.intention(), OnboardingIntention::Terminal);
            assert!(!model.settings().is_ai_enabled());
        });

        // The terminal path continues to completion as usual.
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::ThirdParty);
        model.update(&mut app, |model, ctx| model.next(ctx));
        assert_eq!(step(&app, &model), OnboardingStep::ThemePicker);

        // Back from Customize goes to the intention fork, not the AI-setup slide.
        model.update(&mut app, |model, ctx| {
            model.set_step(OnboardingStep::Customize, ctx);
            model.back(ctx);
        });
        assert_eq!(step(&app, &model), OnboardingStep::Intention);
    });
}

#[test]
fn terminal_settings_disable_ai_when_enrolled() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app, true);
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
fn unenrolled_agent_flow_is_unchanged() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app, false);

        for expected in [
            OnboardingStep::Intention,
            OnboardingStep::Customize,
            OnboardingStep::Agent,
            OnboardingStep::ThirdParty,
            OnboardingStep::ThemePicker,
        ] {
            model.update(&mut app, |model, ctx| model.next(ctx));
            assert_eq!(step(&app, &model), expected);
        }
    });
}

#[test]
fn enrolled_terminal_path_is_unchanged() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app, true);
        model.update(&mut app, |model, ctx| model.set_intention_terminal(ctx));

        for expected in [
            OnboardingStep::Intention,
            OnboardingStep::Customize,
            OnboardingStep::ThirdParty,
            OnboardingStep::ThemePicker,
        ] {
            model.update(&mut app, |model, ctx| model.next(ctx));
            assert_eq!(step(&app, &model), expected);
        }
    });
}

#[test]
fn progress_reports_v3_positions_for_enrolled_agent_path() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app, true);

        let cases = [
            (OnboardingStep::Intention, (0, 5)),
            (OnboardingStep::AiSetup, (1, 5)),
            (OnboardingStep::Agent, (2, 5)),
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
fn progress_reports_existing_positions_when_not_enrolled() {
    let _flag = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);
    App::test((), |mut app| async move {
        let model = add_test_model(&mut app, false);

        let cases = [
            (OnboardingStep::Intention, (0, 5)),
            (OnboardingStep::Customize, (1, 5)),
            (OnboardingStep::Agent, (2, 5)),
            (OnboardingStep::ThirdParty, (3, 5)),
            (OnboardingStep::ThemePicker, (4, 5)),
        ];
        for (target, expected) in cases {
            model.update(&mut app, |model, ctx| model.set_step(target, ctx));
            let progress = model.read(&app, |model, _| model.progress());
            assert_eq!(progress, expected, "unexpected dots for {target:?}");
        }

        // Terminal intention uses the four-dot variant regardless of enrollment.
        model.update(&mut app, |model, ctx| model.set_intention_terminal(ctx));
        let cases = [
            (OnboardingStep::Intention, (0, 4)),
            (OnboardingStep::Customize, (1, 4)),
            (OnboardingStep::ThirdParty, (2, 4)),
            (OnboardingStep::ThemePicker, (3, 4)),
        ];
        for (target, expected) in cases {
            model.update(&mut app, |model, ctx| model.set_step(target, ctx));
            let progress = model.read(&app, |model, _| model.progress());
            assert_eq!(progress, expected, "unexpected dots for {target:?}");
        }
    });
}
