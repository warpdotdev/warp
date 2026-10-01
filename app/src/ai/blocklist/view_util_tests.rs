use warp_core::features::FeatureFlag;
use warpui::App;

use super::*;
use crate::ai::agent::ChatGPTSubscriptionErrorActionKind;
use crate::settings::UsageDisplayUnit;

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
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_usage(20.0, None, Some(0.4), UsageDisplayUnit::Dollars),
        "<$0.01"
    );
}

#[test]
fn format_usage_returns_credits_only_when_flag_disabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(false);

    assert_eq!(
        format_usage(20.0, Some(12345), Some(36.0), UsageDisplayUnit::Dollars),
        format_credits(20.0)
    );
}

#[test]
fn format_usage_uses_credits_unit() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_usage(20.0, Some(12345), Some(36.0), UsageDisplayUnit::Credits),
        "12,345 tokens / 20 credits"
    );
}

#[test]
fn format_usage_uses_dollars_unit() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_usage(20.0, Some(12345), Some(36.0), UsageDisplayUnit::Dollars),
        "12,345 tokens / $0.36"
    );
}

#[test]
fn format_usage_formats_large_token_counts_with_thousands_separators() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_usage(26.9, Some(719_124), Some(48.0), UsageDisplayUnit::Dollars),
        "719,124 tokens / $0.48"
    );
}

#[test]
fn format_usage_falls_back_to_credits_when_dollars_unavailable() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_usage(20.0, Some(12345), None, UsageDisplayUnit::Dollars),
        format_credits(20.0)
    );
}

#[test]
fn format_usage_omits_tokens_when_tokens_is_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_usage(20.0, None, Some(36.0), UsageDisplayUnit::Dollars),
        "$0.36"
    );
    assert_eq!(
        format_usage(20.0, None, Some(36.0), UsageDisplayUnit::Credits),
        format_credits(20.0)
    );
}

#[test]
fn format_usage_omits_tokens_when_tokens_is_zero() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_usage(20.0, Some(0), Some(36.0), UsageDisplayUnit::Dollars),
        "$0.36"
    );
}

#[test]
fn format_usage_credits_unit_omits_tokens_when_tokens_is_zero() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_usage(20.0, Some(0), None, UsageDisplayUnit::Credits),
        format_credits(20.0)
    );
}

#[test]
fn usage_label_uses_dollars_wording_when_unit_is_dollars_and_flag_enabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

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
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

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
fn usage_label_uses_credits_wording_when_flag_disabled_even_if_unit_is_dollars() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(false);

    assert_eq!(
        usage_label(UsageLabelKind::Plain, Some(36.0), UsageDisplayUnit::Dollars),
        "Credits spent"
    );
}

#[test]
fn usage_label_uses_credits_wording_when_dollars_requested_but_cost_unavailable() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        usage_label(UsageLabelKind::Plain, None, UsageDisplayUnit::Dollars),
        "Credits spent"
    );
}
