use std::cell::RefCell;
use std::rc::Rc;

use ai::LLMId;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warp_core::ui::appearance::Appearance;
use warpui_core::elements::Empty;
use warpui_core::platform::WindowStyle;
use warpui_core::{App, AppContext, Element, Entity, ModelHandle, TypedActionView, View as _};

use super::{
    MAX_CREDIT_PACKS, OfferChoice, OfferSlide, OfferSlideAction, OfferVariant, OnboardingSlide as _,
};
use crate::model::{
    CreditPackOption, CreditPurchaseState, OnboardingAuthState, OnboardingStateModel,
};

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
            true,
            OnboardingAuthState::FreeUser,
        )
    })
}

fn credit_packs(count: usize) -> Vec<CreditPackOption> {
    (0..count)
        .map(|index| CreditPackOption {
            credits: 400 * (index as i32 + 1),
            price_usd_cents: 1_200 * (index as i32 + 1),
            savings_percent: index as u32 * 10,
        })
        .collect()
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
        variant.primary_description(false),
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
    assert_eq!(variant.primary_label(), "Subscribe to a Warp plan");
    assert_eq!(
        variant.primary_description(true),
        "Warp Agent works locally or in the cloud with frontier and OSS models. Get monthly credits at the best value, and save 20% on add-on credits with any Build plan."
    );
    assert_eq!(variant.secondary_label(), "Set up AI later");
    assert_eq!(
        variant.secondary_description(),
        "Explore the terminal, bring your own inference, or use another CLI agent. Add AI usage and features anytime."
    );
    assert_eq!(variant.credits_label(), "Buy AI credits");
    assert_eq!(
        variant.credits_description(),
        "Best for trying Warp without a subscription. Buy a one-time credit pack and start using the Warp Agent right away."
    );
    assert!(variant.included_features().is_empty());
    assert_eq!(variant.slide_name(), "choose_how_to_start");
    assert_eq!(variant.account_class(), "free_standard");
    assert_eq!(variant.primary_action(), "use_warp_with_ai");
    assert_eq!(variant.credits_action(), "buy_ai_credits");
}

/// The subscribe card must frame the add-on discount as a saving (matching the
/// web copy), never as a surcharge on the free plan.
#[test]
fn subscribe_copy_frames_add_on_credits_as_a_saving() {
    let description = OfferVariant::ChooseHowToStart.primary_description(true);

    assert!(description.contains("save 20% on add-on credits"));
    assert!(!description.to_lowercase().contains("surcharge"));
    assert!(!description.to_lowercase().contains("premium"));
}

/// The add-on savings line only makes sense beside the packs it refers to, so
/// without them the card falls back to its original copy.
#[test]
fn subscribe_copy_drops_the_add_on_line_when_no_packs_are_shown() {
    let without_packs = OfferVariant::ChooseHowToStart.primary_description(false);

    assert_eq!(
        without_packs,
        "Warp Agent works locally or in the cloud with frontier and OSS models. Proactively fix terminal errors, implement changes, and ship verified code."
    );
    assert!(!without_packs.contains("add-on credits"));

    // The head-start offer never shows packs and is unaffected either way.
    assert_eq!(
        OfferVariant::HeadStart.primary_description(true),
        OfferVariant::HeadStart.primary_description(false)
    );
}

/// The head-start offer already includes AI usage, so it keeps two options.
#[test]
fn only_the_free_standard_offer_supports_credit_packs() {
    assert!(OfferVariant::ChooseHowToStart.supports_credit_packs());
    assert!(!OfferVariant::HeadStart.supports_credit_packs());
}

#[test]
fn buy_credits_is_hidden_until_packs_are_available_and_on_the_head_start_offer() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(MockTelemetryContextProvider::register);
        let onboarding_state = add_onboarding_state(&mut app);
        let (_, slide) = app.add_window(WindowStyle::NotStealFocus, {
            let onboarding_state = onboarding_state.clone();
            move |_| OfferSlide::new(onboarding_state)
        });

        // Pricing hasn't arrived yet, so there is nothing to buy.
        onboarding_state.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::ChooseHowToStart, ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                slide
                    .as_ref(ctx)
                    .choices(OfferVariant::ChooseHowToStart, ctx),
                vec![OfferChoice::Primary, OfferChoice::SetUpLater]
            );
            drop(slide.as_ref(ctx).render(ctx));
        });

        onboarding_state.update(&mut app, |model, ctx| {
            model.set_credit_pack_options(credit_packs(4), ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                slide
                    .as_ref(ctx)
                    .choices(OfferVariant::ChooseHowToStart, ctx),
                vec![
                    OfferChoice::Primary,
                    OfferChoice::BuyCredits,
                    OfferChoice::SetUpLater
                ]
            );
            drop(slide.as_ref(ctx).render(ctx));
        });

        // The head-start offer never shows packs, even when pricing is known.
        let head_start_state = add_onboarding_state(&mut app);
        let (_, head_start_slide) = app.add_window(WindowStyle::NotStealFocus, {
            let head_start_state = head_start_state.clone();
            move |_| OfferSlide::new(head_start_state)
        });
        head_start_state.update(&mut app, |model, ctx| {
            model.show_post_auth_offer(OfferVariant::HeadStart, ctx);
            model.set_credit_pack_options(credit_packs(4), ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                head_start_slide
                    .as_ref(ctx)
                    .choices(OfferVariant::HeadStart, ctx),
                vec![OfferChoice::Primary, OfferChoice::SetUpLater]
            );
        });
    });
}

#[test]
fn arrow_keys_move_through_all_three_options() {
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
            model.set_credit_pack_options(credit_packs(4), ctx);
        });

        let selected = |app: &App| slide.read(app, |slide, _| slide.selected_choice);
        assert_eq!(selected(&app), OfferChoice::Primary);

        slide.update(&mut app, |slide, ctx| slide.on_down(ctx));
        assert_eq!(selected(&app), OfferChoice::BuyCredits);
        slide.update(&mut app, |slide, ctx| slide.on_down(ctx));
        assert_eq!(selected(&app), OfferChoice::SetUpLater);
        // Clamped at the end rather than wrapping.
        slide.update(&mut app, |slide, ctx| slide.on_down(ctx));
        assert_eq!(selected(&app), OfferChoice::SetUpLater);

        slide.update(&mut app, |slide, ctx| slide.on_up(ctx));
        assert_eq!(selected(&app), OfferChoice::BuyCredits);
        slide.update(&mut app, |slide, ctx| slide.on_up(ctx));
        assert_eq!(selected(&app), OfferChoice::Primary);
        slide.update(&mut app, |slide, ctx| slide.on_up(ctx));
        assert_eq!(selected(&app), OfferChoice::Primary);
    });
}

/// Regression test for REV-1886: "Get Warping" on the buy-credits option must
/// start a purchase rather than opening the upgrade page, and must not advance
/// onboarding while that purchase is still in flight.
#[test]
fn get_warping_buys_credits_when_the_credit_option_is_selected() {
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
            model.set_credit_pack_options(credit_packs(4), ctx);
        });

        // Selecting a pack also selects the buy-credits option.
        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::SelectCreditPack(2), ctx)
        });
        assert_eq!(
            slide.read(&app, |slide, _| slide.selected_choice),
            OfferChoice::BuyCredits
        );
        onboarding_state.read(&app, |model, _| {
            assert_eq!(
                model.selected_credit_pack().map(|pack| pack.credits),
                Some(1_200)
            );
        });

        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::GetWarping, ctx)
        });
        onboarding_state.read(&app, |model, _| {
            assert_eq!(
                model.credit_purchase_state(),
                CreditPurchaseState::Purchasing
            );
        });

        // A second Get Warping while the purchase is in flight is a no-op.
        onboarding_state.update(&mut app, |model, ctx| model.on_credit_checkout_opened(ctx));
        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::GetWarping, ctx)
        });
        onboarding_state.read(&app, |model, _| {
            assert_eq!(
                model.credit_purchase_state(),
                CreditPurchaseState::AwaitingCheckout
            );
        });
    });
}

/// "Set up AI later" remains the escape hatch even while a credit purchase is
/// awaiting checkout, so an abandoned checkout never traps the user.
#[test]
fn set_up_later_still_works_while_checkout_is_pending() {
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
            model.set_credit_pack_options(credit_packs(4), ctx);
            model.request_credit_purchase(ctx);
            model.on_credit_checkout_opened(ctx);
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

/// Regression test for REV-1940: the pack selection lives inside the
/// buy-credits card, so no pack may render as selected while another option is
/// chosen. The model keeps a default pack index for the purchase, which must
/// not accent the smallest pack on the slide's initial state.
#[test]
fn packs_only_render_as_selected_while_the_credit_option_is_chosen() {
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
            model.set_credit_pack_options(credit_packs(4), ctx);
        });

        let selected_packs = |app: &App| {
            app.read(|ctx| {
                (0..4)
                    .filter(|index| {
                        slide.as_ref(ctx).credit_pack_is_selected(
                            OfferVariant::ChooseHowToStart,
                            *index,
                            ctx,
                        )
                    })
                    .collect::<Vec<_>>()
            })
        };

        // Subscribe is selected by default, so the packs show no selection.
        assert_eq!(selected_packs(&app), Vec::<usize>::new());

        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::SelectSetUpLater, ctx)
        });
        assert_eq!(selected_packs(&app), Vec::<usize>::new());

        // Choosing the credit option surfaces the pack the purchase would use.
        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::SelectBuyCredits, ctx)
        });
        assert_eq!(selected_packs(&app), vec![0]);

        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::SelectCreditPack(2), ctx)
        });
        assert_eq!(selected_packs(&app), vec![2]);

        // Moving back off the credit option clears the selection again, even
        // though the model still remembers the chosen pack.
        slide.update(&mut app, |slide, ctx| {
            slide.handle_action(&OfferSlideAction::SelectPrimary, ctx)
        });
        assert_eq!(selected_packs(&app), Vec::<usize>::new());
        onboarding_state.read(&app, |model, _| {
            assert_eq!(model.selected_credit_pack_index(), 2);
        });
    });
}

/// The pack rows draw from a fixed pool of hover handles, so an unexpectedly
/// long server list must be truncated rather than panic.
#[test]
fn more_packs_than_the_render_cap_are_truncated() {
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
            model.set_credit_pack_options(credit_packs(MAX_CREDIT_PACKS + 3), ctx);
        });

        app.read(|ctx| {
            let slide = slide.as_ref(ctx);
            assert_eq!(
                slide
                    .credit_packs(OfferVariant::ChooseHowToStart, ctx)
                    .len(),
                MAX_CREDIT_PACKS
            );
            drop(slide.render(ctx));
        });
    });
}
