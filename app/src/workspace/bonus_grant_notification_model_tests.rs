use chrono::Utc;
use warp_graphql::billing::BonusGrantType;

use super::BonusGrantNotificationModel;
use crate::ai::request_usage_model::{BonusGrant, BonusGrantScope};
use crate::server::ids::ServerId;
use crate::settings::UsageDisplayUnit;
use crate::workspaces::workspace::WorkspaceUid;

fn grant(scope: BonusGrantScope, usage_cents_granted: Option<f64>) -> BonusGrant {
    BonusGrant {
        created_at: Utc::now(),
        cost_cents: 0,
        expiration: None,
        grant_type: BonusGrantType::Any,
        reason: "promo".to_string(),
        user_facing_message: None,
        request_credits_granted: 1000,
        request_credits_remaining: 1000,
        usage_cents_granted,
        usage_cents_remaining: usage_cents_granted,
        scope,
    }
}

#[test]
fn generic_grant_message_shows_dollars_when_shown_in_dollars() {
    let team = WorkspaceUid::from(ServerId::from(1_i64));
    assert_eq!(
        BonusGrantNotificationModel::format_generic_grant_message(
            &grant(BonusGrantScope::Team(team), Some(1800.0)),
            UsageDisplayUnit::Dollars
        ),
        "$18.00 has been added to your team."
    );
}

#[test]
fn generic_grant_message_falls_back_to_credits() {
    assert_eq!(
        BonusGrantNotificationModel::format_generic_grant_message(
            &grant(BonusGrantScope::User, None),
            UsageDisplayUnit::Dollars
        ),
        "1000 Reload Credits have been added to your account."
    );
    // Cents the server sends are never shown when displaying in credits.
    assert_eq!(
        BonusGrantNotificationModel::format_generic_grant_message(
            &grant(BonusGrantScope::User, Some(1800.0)),
            UsageDisplayUnit::Credits
        ),
        "1000 Reload Credits have been added to your account."
    );
}
