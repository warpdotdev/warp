use warp_core::features::FeatureFlag;

use super::{
    ModelInUse, UNNAMED_FALLBACK_MODEL_WARPING_TEXT, WarpingModelInputs, WarpingModelMessage,
    warping_model_message,
};
use crate::ai::agent::OutputModelInfo;

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

fn model_info(display_name: &str, is_fallback: bool) -> OutputModelInfo {
    OutputModelInfo {
        model_id: "claude-4-5-sonnet".into(),
        display_name: display_name.to_owned(),
        is_fallback,
        prompt_cache_expires_at: None,
    }
}

#[test]
fn reads_a_reported_model_off_the_exchange_output() {
    assert_eq!(
        ModelInUse::from(&model_info("Claude Sonnet 4.5", true)),
        ModelInUse {
            display_name: Some("Claude Sonnet 4.5".to_owned()),
            is_fallback: true,
        }
    );
}

/// An empty display name is an absent one, not a name: carrying it through would
/// render "Warping with .".
#[test]
fn treats_an_empty_display_name_as_no_name() {
    assert_eq!(ModelInUse::from(&model_info("", false)).display_name, None);
}

#[test]
fn names_the_model_reported_for_the_exchange() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(named("Claude Sonnet 4.5")))),
        Some(WarpingModelMessage {
            text: "Warping with Claude Sonnet 4.5.".to_owned(),
            show_fallback_explanation: false,
        })
    );
}

/// The window before `auto` and custom routers resolve: the row keeps its generic
/// copy rather than guessing at a model.
#[test]
fn stays_generic_until_a_model_is_reported() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(warping_model_message(for_current(None)), None);
}

#[test]
fn stays_generic_when_naming_is_disabled() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(false);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

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

/// Today's shipped configuration — naming off, fallback messaging on — must behave
/// exactly as it did before naming existed.
#[test]
fn names_a_fallback_model_without_the_naming_flag() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(false);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(fallback("Claude Haiku 4.5")))),
        Some(WarpingModelMessage {
            text: "Warping with Claude Haiku 4.5.".to_owned(),
            show_fallback_explanation: true,
        })
    );
}

/// A fallback model is still the model in use, so naming it does not depend on the
/// older fallback flag surviving — only the explanation line does.
#[test]
fn names_a_fallback_model_without_the_explanation_line() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(false);

    assert_eq!(
        warping_model_message(for_current(Some(fallback("Claude Haiku 4.5")))),
        Some(WarpingModelMessage {
            text: "Warping with Claude Haiku 4.5.".to_owned(),
            show_fallback_explanation: false,
        })
    );
}

/// Goes through the conversion rather than hand-building the input, so this also
/// covers what an empty display name renders as: "Warping with ." before the
/// normalization, this afterwards.
#[test]
fn describes_an_unnamed_fallback_model_generically() {
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(ModelInUse::from(&model_info("", true))))),
        Some(WarpingModelMessage {
            text: UNNAMED_FALLBACK_MODEL_WARPING_TEXT.to_owned(),
            show_fallback_explanation: true,
        })
    );
}

#[test]
fn stays_generic_for_a_fallback_model_when_both_flags_are_disabled() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(false);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(false);

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
            show_fallback_explanation: true,
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

/// Only the fallback path may name another exchange's model, so an ordinary model
/// never leaks forward into an exchange that has not reported yet.
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
