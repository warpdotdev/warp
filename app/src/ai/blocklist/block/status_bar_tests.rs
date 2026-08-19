use warp_core::features::FeatureFlag;

use super::{
    ModelInUse, UNNAMED_FALLBACK_MODEL_WARPING_TEXT, WarpingModelInputs, WarpingModelMessage,
    warping_model_message,
};

fn named(display_name: &str) -> ModelInUse {
    ModelInUse {
        display_name: Some(display_name.to_owned()),
        is_fallback: false,
    }
}

fn fallback(display_name: &str) -> ModelInUse {
    ModelInUse {
        display_name: Some(display_name.to_owned()),
        is_fallback: true,
    }
}

fn for_current(current: Option<ModelInUse>) -> WarpingModelInputs {
    WarpingModelInputs {
        current,
        previous: None,
        is_new_user_query: false,
    }
}

#[test]
fn names_the_model_reported_for_the_exchange() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(named("Claude Sonnet 4.5")))),
        Some(WarpingModelMessage {
            text: "Warping with Claude Sonnet 4.5.".to_owned(),
            is_fallback: false,
        })
    );
}

/// The window before `auto` and custom routers resolve: the row keeps its
/// generic copy rather than guessing at a model.
#[test]
fn stays_generic_until_a_model_is_reported() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(warping_model_message(for_current(None)), None);
}

/// Routing outcomes arrive one after another within a conversation: nothing yet,
/// then whatever the router picked, then a different model on a later request,
/// then a fallback attempt.
#[test]
fn follows_the_model_across_routing_outcomes() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    let texts: Vec<Option<String>> = [
        None,
        Some(named("GPT-5.2 (high reasoning)")),
        Some(named("Claude Opus 4.5")),
        Some(fallback("Claude Haiku 4.5")),
    ]
    .into_iter()
    .map(|current| warping_model_message(for_current(current)).map(|message| message.text))
    .collect();

    assert_eq!(
        texts,
        vec![
            None,
            Some("Warping with GPT-5.2 (high reasoning).".to_owned()),
            Some("Warping with Claude Opus 4.5.".to_owned()),
            Some("Warping with Claude Haiku 4.5.".to_owned()),
        ]
    );
}

#[test]
fn stays_generic_when_naming_is_disabled() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(false);

    assert_eq!(
        warping_model_message(for_current(Some(named("Claude Sonnet 4.5")))),
        None
    );
}

#[test]
fn stays_generic_when_the_reported_model_has_no_display_name() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(ModelInUse {
            display_name: None,
            is_fallback: false,
        }))),
        None
    );
}

#[test]
fn names_a_fallback_model_and_asks_for_the_explanation_line() {
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);
    let _naming = FeatureFlag::WarpingModelName.override_enabled(false);

    assert_eq!(
        warping_model_message(for_current(Some(fallback("Claude Haiku 4.5")))),
        Some(WarpingModelMessage {
            text: "Warping with Claude Haiku 4.5.".to_owned(),
            is_fallback: true,
        })
    );
}

#[test]
fn describes_an_unnamed_fallback_model_generically() {
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(ModelInUse {
            display_name: None,
            is_fallback: true,
        }))),
        Some(WarpingModelMessage {
            text: UNNAMED_FALLBACK_MODEL_WARPING_TEXT.to_owned(),
            is_fallback: true,
        })
    );
}

#[test]
fn omits_a_fallback_model_when_fallback_messaging_is_disabled() {
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(false);
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(fallback("Claude Haiku 4.5")))),
        None
    );
}

#[test]
fn names_the_previous_exchanges_fallback_model_on_agent_follow_ups() {
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(WarpingModelInputs {
            current: None,
            previous: Some(fallback("Claude Haiku 4.5")),
            is_new_user_query: false,
        }),
        Some(WarpingModelMessage {
            text: "Warping with Claude Haiku 4.5.".to_owned(),
            is_fallback: true,
        })
    );
}

#[test]
fn ignores_the_previous_exchanges_fallback_model_after_a_new_user_query() {
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(WarpingModelInputs {
            current: None,
            previous: Some(fallback("Claude Haiku 4.5")),
            is_new_user_query: true,
        }),
        None
    );
}

/// Only the fallback path may name another exchange's model, so an ordinary
/// model never leaks forward into an exchange that has not reported yet.
#[test]
fn never_borrows_an_ordinary_model_from_the_previous_exchange() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(WarpingModelInputs {
            current: None,
            previous: Some(named("Claude Sonnet 4.5")),
            is_new_user_query: false,
        }),
        None
    );
}
