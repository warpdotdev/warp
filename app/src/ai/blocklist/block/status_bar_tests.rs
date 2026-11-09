use std::cell::RefCell;
use std::rc::Rc;

use warp_core::features::FeatureFlag;
use warpui::{App, EntityId, SingletonEntity};

use super::{
    BlocklistAIStatusBarEvent, ModelInUse, UNNAMED_FALLBACK_MODEL_WARPING_TEXT, WarpingModelInputs,
    WarpingModelMessage, warping_model_message,
};
use crate::ai::agent::OutputModelInfo;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::test_util::ai_agent_tasks::{create_api_task, create_message};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

#[test]
fn inactive_exchange_updates_do_not_close_summarization_dialog() {
    App::test((), |mut app| async move {
        let _tips = FeatureFlag::AgentTips.override_enabled(false);
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_surface_id = terminal.id();
        let status_bar = terminal.read(&app, |terminal, ctx| {
            terminal.input().as_ref(ctx).agent_status_bar().clone()
        });
        let history = app.read(BlocklistAIHistoryModel::handle);
        let conversation = AIConversation::new_restored(
            AIConversationId::new(),
            vec![create_api_task(
                "root",
                (0..2)
                    .map(|index| {
                        let mut message = create_message(&format!("message-{index}"), "root");
                        message.request_id = format!("request-{index}");
                        message
                    })
                    .collect(),
            )],
            None,
        )
        .unwrap();
        let conversation_id = conversation.id();
        let task_id = conversation.get_root_task_id().clone();
        let exchange_ids: Vec<_> = conversation
            .all_exchanges()
            .iter()
            .map(|exchange| exchange.id)
            .collect();
        assert_eq!(exchange_ids.len(), 2);
        history.update(&mut app, |history, ctx| {
            history.restore_conversations(terminal_surface_id, vec![conversation], ctx);
        });
        for exchange_id in &exchange_ids {
            history.update(&mut app, |_, ctx| {
                ctx.emit(BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id: *exchange_id,
                    conversation_id,
                    task_id: task_id.clone(),
                    terminal_surface_id,
                    is_hidden: false,
                    response_stream_id: None,
                });
            });
        }
        let dialog_events = Rc::new(RefCell::new(Vec::new()));
        let observed_dialog_events = dialog_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&status_bar, move |_, event, _| {
                if let BlocklistAIStatusBarEvent::SummarizationCancelDialogToggled { is_open } =
                    event
                {
                    observed_dialog_events.borrow_mut().push(*is_open);
                }
            });
        });
        status_bar.update(&mut app, |bar, ctx| {
            bar.open_summarization_cancel_dialog(ctx);
        });

        history.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id: exchange_ids[0],
                conversation_id,
                terminal_surface_id,
                is_hidden: false,
            });
        });
        assert_eq!(*dialog_events.borrow(), vec![true]);

        history.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id: exchange_ids[1],
                conversation_id,
                terminal_surface_id: EntityId::new(),
                is_hidden: false,
            });
        });
        assert_eq!(*dialog_events.borrow(), vec![true, false]);
    });
}

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
/// render "Warping with ...".
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
            text: "Warping with Claude Sonnet 4.5...".to_owned(),
            model_display_name: Some("Claude Sonnet 4.5".to_owned()),
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
/// exactly as it did before naming existed, down to the full stop. The fallback
/// message keeps its period while everything named since ends in an ellipsis, and
/// `model_display_name` stays `None` so the legacy flag cannot smuggle the name
/// into the row's other messages.
#[test]
fn names_a_fallback_model_without_the_naming_flag() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(false);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(fallback("Claude Haiku 4.5")))),
        Some(WarpingModelMessage {
            text: "Warping with Claude Haiku 4.5.".to_owned(),
            model_display_name: None,
            show_fallback_explanation: true,
        })
    );
}

/// The two copy rules side by side, so neither can drift into the other: the
/// fallback message ends in a period, the model naming ends in an ellipsis.
#[test]
fn ends_the_fallback_message_in_a_period_and_the_named_message_in_an_ellipsis() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    let fallback_text =
        warping_model_message(for_current(Some(fallback("Claude Haiku 4.5")))).map(|m| m.text);
    let named_text =
        warping_model_message(for_current(Some(named("Claude Haiku 4.5")))).map(|m| m.text);

    assert_eq!(
        fallback_text.as_deref(),
        Some("Warping with Claude Haiku 4.5.")
    );
    assert_eq!(
        named_text.as_deref(),
        Some("Warping with Claude Haiku 4.5...")
    );
}

/// A fallback model is still the model in use, so naming it does not depend on the
/// older fallback flag surviving — only the explanation line does. The copy is the
/// new one, ellipsis and all, because the period belongs to the fallback message
/// and that message does not exist with its flag off.
#[test]
fn names_a_fallback_model_without_the_explanation_line() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(false);

    assert_eq!(
        warping_model_message(for_current(Some(fallback("Claude Haiku 4.5")))),
        Some(WarpingModelMessage {
            text: "Warping with Claude Haiku 4.5...".to_owned(),
            model_display_name: Some("Claude Haiku 4.5".to_owned()),
            show_fallback_explanation: false,
        })
    );
}

/// Goes through the conversion rather than hand-building the input, so this also
/// covers what an empty display name renders as: "Warping with ." before the
/// normalization, this afterwards. No name means no name for the other status
/// messages either.
#[test]
fn describes_an_unnamed_fallback_model_generically() {
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(for_current(Some(ModelInUse::from(&model_info("", true))))),
        Some(WarpingModelMessage {
            text: UNNAMED_FALLBACK_MODEL_WARPING_TEXT.to_owned(),
            model_display_name: None,
            show_fallback_explanation: true,
        })
    );
}

/// Without the fallback message there is no copy for a model that arrived with no
/// name, so the row keeps its generic text rather than borrowing the fallback's.
#[test]
fn stays_generic_for_an_unnamed_fallback_model_when_fallback_messaging_is_disabled() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(false);

    assert_eq!(
        warping_model_message(for_current(Some(ModelInUse {
            display_name: None,
            is_fallback: true,
        }))),
        None
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

/// Naming is on here so the `None` below is attributable to the name being
/// borrowed rather than to the flag.
#[test]
fn names_the_previous_exchanges_fallback_model_on_agent_follow_ups() {
    let _naming = FeatureFlag::WarpingModelName.override_enabled(true);
    let _fallback_messaging = FeatureFlag::FallbackModelLoadOutputMessaging.override_enabled(true);

    assert_eq!(
        warping_model_message(WarpingModelInputs {
            current: None,
            previous: Some(fallback("Claude Haiku 4.5")),
            is_new_user_query: false,
        }),
        Some(WarpingModelMessage {
            text: "Warping with Claude Haiku 4.5.".to_owned(),
            // Borrowed, so it stays out of the row's other messages: the lookback
            // was justified for this one sentence, not for five more.
            model_display_name: None,
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
