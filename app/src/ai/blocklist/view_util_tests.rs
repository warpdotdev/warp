use warpui::App;

use super::*;
use crate::ai::agent::ChatGPTSubscriptionErrorActionKind;
use crate::settings::UsageDisplayUnit;
use crate::test_util::billing_unit::{set_charge_unit, set_usage_display_unit};
use crate::test_util::settings::initialize_settings_for_tests;

/// Registers the settings and workspaces the unit resolver reads.
fn initialize_usage_unit_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(UserWorkspaces::default_mock);
}

fn chatgpt_subscription_error(actions: Vec<ChatGPTSubscriptionErrorAction>) -> RenderableAIError {
    RenderableAIError::ChatGPTSubscriptionError {
        code: "subscription_sharing_usage_limit_exceeded".to_string(),
        title: "You've reached your ChatGPT usage limit".to_string(),
        message: "Continue with Warp credits to keep going.".to_string(),
        actions,
    }
}

fn chatgpt_action(kind: ChatGPTSubscriptionErrorActionKind) -> ChatGPTSubscriptionErrorAction {
    ChatGPTSubscriptionErrorAction {
        label: format!("{kind:?}"),
        kind,
    }
}

#[test]
fn chatgpt_subscription_error_presents_server_copy_and_actions_in_order() {
    App::test((), |app| async move {
        app.read(|ctx| {
            for actions in [
                vec![],
                vec![chatgpt_action(
                    ChatGPTSubscriptionErrorActionKind::ContinueWithWarpCredits,
                )],
                vec![
                    chatgpt_action(ChatGPTSubscriptionErrorActionKind::Retry),
                    chatgpt_action(ChatGPTSubscriptionErrorActionKind::ContinueWithWarpCredits),
                ],
                vec![
                    chatgpt_action(ChatGPTSubscriptionErrorActionKind::OpenUrl {
                        url: "https://chatgpt.com/#settings/Usage".to_string(),
                    }),
                    chatgpt_action(ChatGPTSubscriptionErrorActionKind::ContinueWithWarpCredits),
                ],
            ] {
                let error = chatgpt_subscription_error(actions.clone());
                assert_eq!(
                    failed_output_presentation(&error, false, ctx),
                    Some(FailedOutputPresentation::ChatGPTSubscription {
                        title: "You've reached your ChatGPT usage limit".to_string(),
                        message: "Continue with Warp credits to keep going.".to_string(),
                        actions,
                    })
                );
            }
        });
    });
}

#[test]
fn chatgpt_subscription_error_becomes_disclosure_once_conversation_uses_warp_credits() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let error = chatgpt_subscription_error(vec![chatgpt_action(
                ChatGPTSubscriptionErrorActionKind::ContinueWithWarpCredits,
            )]);
            assert_eq!(
                failed_output_presentation(&error, true, ctx),
                Some(FailedOutputPresentation::ChatGPTSubscriptionContinuedWithWarpCredits)
            );
        });
    });
}

#[test]
fn chatgpt_subscription_message_with_links_appends_only_url_actions() {
    let message = "Continue with Warp credits to keep going.";
    assert_eq!(
        chatgpt_subscription_message_with_links(message, &[]),
        message
    );
    assert_eq!(
        chatgpt_subscription_message_with_links(
            message,
            &[
                chatgpt_action(ChatGPTSubscriptionErrorActionKind::Retry),
                ChatGPTSubscriptionErrorAction {
                    kind: ChatGPTSubscriptionErrorActionKind::OpenUrl {
                        url: "https://chatgpt.com/#settings/Usage".to_string(),
                    },
                    label: "Manage usage".to_string(),
                },
                chatgpt_action(ChatGPTSubscriptionErrorActionKind::ContinueWithWarpCredits),
            ],
        ),
        format!("{message}\n\nManage usage: https://chatgpt.com/#settings/Usage")
    );
}

#[test]
fn chatgpt_subscription_error_suppresses_usage_notice() {
    let error = chatgpt_subscription_error(vec![]);
    assert!(!should_show_failed_output_usage_notice(
        &error, true, false, false
    ));
}

#[test]
fn format_credits_never_rounds_a_real_charge_to_zero() {
    assert_eq!(format_credits(0.0), "0 credits");
    assert_eq!(format_credits(0.03), "<0.1 credits");
    assert_eq!(format_credits(0.1), "0.1 credits");
    assert_eq!(format_credits(1.0), "1 credit");
    assert_eq!(format_credits(2.5), "2.5 credits");
}

#[test]
fn format_dollars_formats_zero_exactly() {
    assert_eq!(format_dollars(0.0), "$0.00");
    assert_eq!(format_dollars(-0.0), "$0.00");
}

#[test]
fn format_dollars_floors_positive_sub_cent_amounts() {
    assert_eq!(format_dollars(0.3), "<$0.01");
}

#[test]
fn format_dollars_formats_one_cent_exactly() {
    assert_eq!(format_dollars(1.0), "$0.01");
}

#[test]
fn format_usage_floors_positive_sub_cent_dollar_amounts() {
    assert_eq!(
        format_usage(20.0, Some(0.4), UsageDisplayUnit::Dollars),
        "<$0.01"
    );
}

/// A viewer charged in cents follows the display preference, which defaults to credits.
#[test]
fn usage_display_unit_follows_the_preference_for_cents_charged_viewers() {
    App::test((), |mut app| async move {
        initialize_usage_unit_test_app(&mut app);
        set_charge_unit(&mut app, ChargeUnit::Cents);

        app.read(|ctx| {
            assert_eq!(usage_display_unit(ctx), UsageDisplayUnit::Credits);
            assert_eq!(
                effective_usage_unit(Some(36.0), ctx),
                UsageDisplayUnit::Credits
            );
        });

        set_usage_display_unit(&mut app, UsageDisplayUnit::Dollars);
        app.read(|ctx| {
            assert_eq!(usage_display_unit(ctx), UsageDisplayUnit::Dollars);
            assert_eq!(
                effective_usage_unit(Some(36.0), ctx),
                UsageDisplayUnit::Dollars
            );
        });
    });
}

/// Without a cents figure there is nothing to show in dollars, so a viewer who prefers dollars
/// falls back to the credits string for that figure.
#[test]
fn effective_usage_unit_falls_back_to_credits_without_a_cents_figure() {
    App::test((), |mut app| async move {
        initialize_usage_unit_test_app(&mut app);
        set_charge_unit(&mut app, ChargeUnit::Cents);
        set_usage_display_unit(&mut app, UsageDisplayUnit::Dollars);

        app.read(|ctx| {
            assert_eq!(effective_usage_unit(None, ctx), UsageDisplayUnit::Credits);
        });
    });
}

/// A tier charged in credits is always displayed in credits: neither a cents figure nor a
/// dollars preference changes that.
#[test]
fn usage_display_unit_is_credits_for_credit_charged_viewers() {
    App::test((), |mut app| async move {
        initialize_usage_unit_test_app(&mut app);
        set_charge_unit(&mut app, ChargeUnit::Credits);
        set_usage_display_unit(&mut app, UsageDisplayUnit::Dollars);

        app.read(|ctx| {
            assert_eq!(usage_display_unit(ctx), UsageDisplayUnit::Credits);
            assert_eq!(
                effective_usage_unit(Some(36.0), ctx),
                UsageDisplayUnit::Credits
            );
        });
    });
}

#[test]
fn format_usage_uses_credits_unit() {
    assert_eq!(
        format_usage(20.0, Some(36.0), UsageDisplayUnit::Credits),
        "20 credits"
    );
}

#[test]
fn format_usage_uses_dollars_unit() {
    assert_eq!(
        format_usage(20.0, Some(36.0), UsageDisplayUnit::Dollars),
        "$0.36"
    );
}

#[test]
fn format_usage_falls_back_to_credits_when_dollars_unavailable() {
    assert_eq!(
        format_usage(20.0, None, UsageDisplayUnit::Dollars),
        format_credits(20.0)
    );
}

#[test]
fn usage_label_uses_dollars_wording_when_unit_is_dollars() {
    assert_eq!(
        usage_label(UsageLabelKind::Plain, Some(36.0), UsageDisplayUnit::Dollars),
        "Usage charged"
    );
    assert_eq!(
        usage_label(
            UsageLabelKind::LastResponse,
            Some(36.0),
            UsageDisplayUnit::Dollars
        ),
        "Usage charged (last response)"
    );
    assert_eq!(
        usage_label(UsageLabelKind::Total, Some(36.0), UsageDisplayUnit::Dollars),
        "Usage charged (total)"
    );
    assert_eq!(
        usage_label(
            UsageLabelKind::DetailsPanel,
            Some(36.0),
            UsageDisplayUnit::Dollars
        ),
        "Usage"
    );
}

#[test]
fn usage_label_uses_credits_wording_when_unit_is_credits() {
    assert_eq!(
        usage_label(UsageLabelKind::Plain, None, UsageDisplayUnit::Credits),
        "Credits spent"
    );
    assert_eq!(
        usage_label(
            UsageLabelKind::LastResponse,
            None,
            UsageDisplayUnit::Credits
        ),
        "Credits spent (last response)"
    );
    assert_eq!(
        usage_label(UsageLabelKind::Total, None, UsageDisplayUnit::Credits),
        "Credits spent (total)"
    );
    assert_eq!(
        usage_label(
            UsageLabelKind::DetailsPanel,
            None,
            UsageDisplayUnit::Credits
        ),
        "Credits used"
    );
}

#[test]
fn usage_label_uses_credits_wording_when_dollars_requested_but_cost_unavailable() {
    assert_eq!(
        usage_label(UsageLabelKind::Plain, None, UsageDisplayUnit::Dollars),
        "Credits spent"
    );
}
