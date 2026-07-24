use ai::LLMId;
use warpui_core::{App, View as _};

use super::{OfferSlide, OfferVariant};
use crate::model::{OnboardingAuthState, OnboardingStateModel};

#[test]
fn offer_slide_can_render_before_classification() {
    App::test((), |mut app| async move {
        let onboarding_state = app.add_model(|_| {
            OnboardingStateModel::new(
                Vec::new(),
                LLMId::from("auto"),
                false,
                true,
                OnboardingAuthState::FreeUser,
            )
        });
        let slide = OfferSlide::new(onboarding_state);

        app.read(|ctx| {
            drop(slide.render(ctx));
        });
    });
}

#[test]
fn head_start_copy_and_telemetry_names_match_spec() {
    let variant = OfferVariant::HeadStart;

    assert_eq!(variant.title(), "You've got a head start");
    assert_eq!(
        variant.subtitle(),
        Some("Your account includes AI usage to help you get started.")
    );
    assert_eq!(variant.primary_label(), "Unlock the full AI experience");
    assert_eq!(
        variant.primary_description(),
        "Get more monthly usage, expanded cloud agent access, and collaboration features."
    );
    assert_eq!(variant.secondary_label(), "Start with included AI");
    assert_eq!(
        variant.secondary_description(),
        "Explore with the AI usage included with your account and upgrade to add more anytime."
    );
    assert_eq!(
        variant.included_features(),
        &[
            "Limited monthly AI usage for occasional tasks",
            "Access to premium and open-source models",
            "Use the Warp Agent locally and in the cloud",
        ]
    );
    assert_eq!(variant.slide_name(), "head_start");
    assert_eq!(variant.account_class(), "free_icp");
    assert_eq!(variant.primary_action(), "get_more_ai");
}

#[test]
fn choose_how_to_start_copy_and_telemetry_names_match_spec() {
    let variant = OfferVariant::ChooseHowToStart;

    assert_eq!(variant.title(), "Choose how to start");
    assert_eq!(variant.subtitle(), None);
    assert_eq!(variant.primary_label(), "Use Warp with AI");
    assert_eq!(
        variant.primary_description(),
        "Warp Agent works locally or in the cloud with frontier and OSS models. Proactively fix terminal errors, implement changes, and ship verified code."
    );
    assert_eq!(variant.secondary_label(), "Set up AI later");
    assert_eq!(
        variant.secondary_description(),
        "Explore the terminal, bring your own inference, or use another CLI agent. Add AI usage and features anytime."
    );
    assert!(variant.included_features().is_empty());
    assert_eq!(variant.slide_name(), "choose_how_to_start");
    assert_eq!(variant.account_class(), "free_standard");
    assert_eq!(variant.primary_action(), "use_warp_with_ai");
}
