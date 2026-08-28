use std::cell::RefCell;
use std::rc::Rc;

use ai::LLMId;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warp_core::ui::appearance::Appearance;
use warpui_core::elements::Empty;
use warpui_core::platform::WindowStyle;
use warpui_core::{App, AppContext, Element, Entity, ModelHandle, TypedActionView, View as _};

use super::{OfferChoice, OfferSlide, OfferSlideAction, OfferVariant, OnboardingSlide as _};
use crate::model::{OnboardingAuthState, OnboardingStateModel};

/// A do-nothing view used only to observe the events an [`OfferSlide`] emits.
struct EventObserver {
    events: Rc<RefCell<Vec<String>>>,
}

impl Entity for EventObserver {
    type Event = ();
}

impl warpui_core::View for EventObserver {
    fn ui_name() -> &'static str {
        "EventObserver"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for EventObserver {
    type Action = ();
}

fn add_onboarding_state(app: &mut App) -> ModelHandle<OnboardingStateModel> {
    app.add_model(|_| {
        OnboardingStateModel::new(
            Vec::new(),
            LLMId::from("auto"),
            false,
            OnboardingAuthState::FreeUser,
        )
    })
}

#[test]
fn offer_slide_can_render_before_classification() {
    App::test((), |mut app| async move {
        let onboarding_state = add_onboarding_state(&mut app);
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
    assert_eq!(
        variant.subtitle(),
        Some("To use AI, start with a plan or one-time credit packs.")
    );
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

#[test]
fn promotion_replaces_recommended_on_both_offer_variants() {
    let promotion = Some("50% off Fable and Opus 5");

    assert_eq!(
        OfferVariant::ChooseHowToStart.primary_badge_label(promotion),
        "50% off Fable and Opus 5"
    );
    assert_eq!(
        OfferVariant::ChooseHowToStart.primary_badge_label(None),
        "Recommended"
    );
    assert_eq!(
        OfferVariant::HeadStart.primary_badge_label(promotion),
        "50% off Fable and Opus 5"
    );
    assert_eq!(
        OfferVariant::HeadStart.primary_badge_label(None),
        "Recommended"
    );
}

#[test]
fn both_rendered_offer_paths_read_the_promotion_from_onboarding_state() {
    App::test((), |mut app| async move {
        let onboarding_state = add_onboarding_state(&mut app);
        onboarding_state.update(&mut app, |model, ctx| {
            model
                .set_pricing_promotion_message(Some("50% off Fable 5 and Opus 5".to_string()), ctx);
        });
        let slide = OfferSlide::new(onboarding_state);

        app.read(|ctx| {
            assert_eq!(
                slide.primary_badge_label(OfferVariant::HeadStart, ctx),
                "50% off Fable 5 and Opus 5"
            );
            assert_eq!(
                slide.primary_badge_label(OfferVariant::ChooseHowToStart, ctx),
                "50% off Fable 5 and Opus 5"
            );
        });
    });
}

/// The head-start offer already ships with AI usage, so it is not the surface
/// that sells any.
#[test]
fn only_the_free_standard_offer_sells_ai_usage() {
    assert!(OfferVariant::ChooseHowToStart.sells_ai_usage());
    assert!(!OfferVariant::HeadStart.sells_ai_usage());
}

/// Renders a classified offer in every state the slide can be in: both
/// variants, both selections, and with the auth prompt bar raised. Without a
/// variant set `render` bails to `Empty`, so the pre-classification render test
/// never reaches `render_options` / `render_option_card`; the windowed
/// behavioural tests reach them incidentally for the free-standard variant
/// only, leaving this as the sole coverage of the head-start layout.
#[test]
fn classified_offer_renders_in_every_selection_state() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(MockTelemetryContextProvider::register);

        // `show_post_auth_offer` is sticky, so each variant needs its own model.
        for variant in [OfferVariant::HeadStart, OfferVariant::ChooseHowToStart] {
            let onboarding_state = add_onboarding_state(&mut app);
            let (_, slide) = app.add_window(WindowStyle::NotStealFocus, {
                let onboarding_state = onboarding_state.clone();
                move |_| OfferSlide::new(onboarding_state)
            });
            onboarding_state.update(&mut app, |model, ctx| {
                model.show_post_auth_offer(variant, ctx);
            });

            // The primary card is selected by default, so rendering either side
            // of this selection change covers both cards selected and not.
            for action in [
                OfferSlideAction::SelectSetUpLater,
                OfferSlideAction::SelectPrimary,
            ] {
                app.read(|ctx| drop(slide.as_ref(ctx).render(ctx)));
                slide.update(&mut app, |slide, ctx| slide.handle_action(&action, ctx));
            }

            // "Get Warping" on the primary raises the auth prompt bar, which
            // renders stacked over the slide.
            slide.update(&mut app, |slide, ctx| {
                slide.handle_action(&OfferSlideAction::GetWarping, ctx)
            });
            assert!(
                slide.read(&app, |slide, _| slide.show_auth_prompt_bar),
                "{variant:?} should have raised the auth prompt bar"
            );
            app.read(|ctx| drop(slide.as_ref(ctx).render(ctx)));
        }
    });
}

/// Selecting the primary card must not open the upgrade page; only "Get
/// Warping" does.
#[test]
fn selecting_the_primary_card_does_not_launch_upgrade() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(MockTelemetryContextProvider::register);
        let onboarding_state = add_onboarding_state(&mut app);
        let (_, slide) = app.add_window(WindowStyle::NotStealFocus, {
            let onboarding_state = onboarding_state.clone();
            move |_| OfferSlide::new(onboarding_state)
        });
        onboarding_state.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
        });

        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::SelectSetUpLater, ctx);
            slide.handle_action(&OfferSlideAction::SelectPrimary, ctx);
        });
        assert_eq!(
            slide.read(&app, |slide, _| slide.selected_choice),
            OfferChoice::Primary
        );
        assert!(!slide.read(&app, |slide, _| slide.show_auth_prompt_bar));

        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::GetWarping, ctx)
        });
        assert!(slide.read(&app, |slide, _| slide.show_auth_prompt_bar));
    });
}

#[test]
fn arrow_keys_move_through_both_options() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(MockTelemetryContextProvider::register);
        let onboarding_state = add_onboarding_state(&mut app);
        let (_, slide) = app.add_window(WindowStyle::NotStealFocus, {
            let onboarding_state = onboarding_state.clone();
            move |_| OfferSlide::new(onboarding_state)
        });
        onboarding_state.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
        });

        let selected = |app: &App| slide.read(app, |slide, _| slide.selected_choice);
        assert_eq!(selected(&app), OfferChoice::Primary);

        slide.update(&mut app, |slide, ctx| slide.on_down(ctx));
        assert_eq!(selected(&app), OfferChoice::SetUpLater);
        // Clamped at the end rather than wrapping.
        slide.update(&mut app, |slide, ctx| slide.on_down(ctx));
        assert_eq!(selected(&app), OfferChoice::SetUpLater);

        slide.update(&mut app, |slide, ctx| slide.on_up(ctx));
        assert_eq!(selected(&app), OfferChoice::Primary);
        slide.update(&mut app, |slide, ctx| slide.on_up(ctx));
        assert_eq!(selected(&app), OfferChoice::Primary);
    });
}

#[test]
fn get_warping_on_set_up_later_emits_exactly_one_event() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(MockTelemetryContextProvider::register);
        let onboarding_state = add_onboarding_state(&mut app);
        let (_, slide) = app.add_window(WindowStyle::NotStealFocus, {
            let onboarding_state = onboarding_state.clone();
            move |_| OfferSlide::new(onboarding_state)
        });
        onboarding_state.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
        });

        let events = Rc::new(RefCell::new(Vec::new()));
        let (_, observer) = app.add_window(WindowStyle::NotStealFocus, {
            let events = events.clone();
            move |_| EventObserver { events }
        });
        observer.update(&mut app, |_, ctx| {
            ctx.subscribe_to_view(&slide, |observer, _, event, _| {
                observer.events.borrow_mut().push(format!("{event:?}"));
            });
        });

        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::SelectSetUpLater, ctx);
            slide.handle_action(&OfferSlideAction::GetWarping, ctx);
        });

        let recorded = events.borrow().clone();
        assert_eq!(recorded.len(), 1, "expected one event, got {recorded:?}");
        assert!(recorded[0].contains("SetUpLaterSelected"));
    });
}
