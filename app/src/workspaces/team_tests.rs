use chrono::Utc;
use warp_graphql::billing::{ServiceAgreement, ServiceAgreementStatus, ServiceAgreementType};
use warp_graphql::scalars::time::ServerTimestamp;

use super::*;
use crate::workspaces::workspace::BillingMetadata;

fn make_service_agreement(status: ServiceAgreementStatus) -> ServiceAgreement {
    make_service_agreement_with_type(status, ServiceAgreementType::SelfServe)
}

fn make_service_agreement_with_type(
    status: ServiceAgreementStatus,
    type_: ServiceAgreementType,
) -> ServiceAgreement {
    ServiceAgreement {
        addon_credit_auto_reload_status: None,
        current_period_end: ServerTimestamp::new(Utc::now() + chrono::Duration::days(30)),
        status,
        stripe_subscription_id: None,
        type_,
        sunsetted_to_build_ts: None,
    }
}

fn solo_owner_team_with_billing(email: &str, billing_metadata: BillingMetadata) -> Team {
    Team {
        uid: 1_i64.into(),
        name: "Test Team".to_string(),
        invite_code: None,
        members: vec![TeamMember {
            uid: UserUid::new(email),
            email: email.to_string(),
            role: MembershipRole::Owner,
        }],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata,
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

/// An active subscription must block deletion so users cannot lose billing data.
#[test]
fn test_get_delete_disabled_reason_active_subscription_blocks_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement(ServiceAgreementStatus::Active)],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(
        reason,
        Some(TeamDeleteDisabledReason::ActivePaidSubscription),
    );
}

/// A cancelled subscription must NOT block deletion — users who cancelled their
/// plan should still be able to delete their team and join another one (REV-1795).
#[test]
fn test_get_delete_disabled_reason_cancelled_subscription_allows_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement(ServiceAgreementStatus::Canceled)],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, None);
}

/// When there are no service agreements on file, deletion should be permitted.
#[test]
fn test_get_delete_disabled_reason_no_service_agreements_allows_delete() {
    let billing = BillingMetadata::default();
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, None);
}

/// Other team members must always block deletion, regardless of billing state.
#[test]
fn test_get_delete_disabled_reason_other_members_block_delete() {
    let team = Team {
        uid: 1_i64.into(),
        name: "Test Team".to_string(),
        invite_code: None,
        members: vec![
            TeamMember {
                uid: UserUid::new("owner@example.com"),
                email: "owner@example.com".to_string(),
                role: MembershipRole::Owner,
            },
            TeamMember {
                uid: UserUid::new("other@example.com"),
                email: "other@example.com".to_string(),
                role: MembershipRole::User,
            },
        ],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: BillingMetadata::default(),
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    };
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, Some(TeamDeleteDisabledReason::OtherMembers));
}

/// Remaining bonus credits must block deletion regardless of subscription state.
#[test]
fn test_get_delete_disabled_reason_remaining_credits_block_delete() {
    let billing = BillingMetadata::default();
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 100);
    assert_eq!(
        reason,
        Some(TeamDeleteDisabledReason::RemainingBonusCredits),
    );
}

// --- Tests that pin the client/server rule parity ---
//
// The server blocks deletion when the *first non-Canceled* SA is
// self-serviceable (`SelfServe | Turbo | Prosumer | Business | Lightspeed`).
// `PastDue` and `Unpaid` SAs count as "live" on the server and must also
// block deletion on the client (regression tests for the previous bug where
// the client only checked for `Active` status).
// Non-self-serviceable types such as `ProTrial`, `TeamTrial`, and `Legacy`
// must NOT block deletion even when the SA is `Active` (the server allows it).

/// A `PastDue` self-serve SA is live on the server and must block deletion.
#[test]
fn test_get_delete_disabled_reason_past_due_subscription_blocks_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement(ServiceAgreementStatus::PastDue)],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(
        reason,
        Some(TeamDeleteDisabledReason::ActivePaidSubscription),
    );
}

/// An `Unpaid` self-serve SA is live on the server and must block deletion.
#[test]
fn test_get_delete_disabled_reason_unpaid_subscription_blocks_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement(ServiceAgreementStatus::Unpaid)],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(
        reason,
        Some(TeamDeleteDisabledReason::ActivePaidSubscription),
    );
}

/// An `Active` SA of a non-self-serviceable type (`ProTrial`) must NOT block
/// deletion — the server allows it via `IsSelfServicableAgreementType`.
#[test]
fn test_get_delete_disabled_reason_active_pro_trial_allows_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement_with_type(
            ServiceAgreementStatus::Active,
            ServiceAgreementType::ProTrial,
        )],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, None);
}

/// An `Active` SA of a non-self-serviceable type (`TeamTrial`) must NOT block
/// deletion — the server allows it via `IsSelfServicableAgreementType`.
#[test]
fn test_get_delete_disabled_reason_active_team_trial_allows_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement_with_type(
            ServiceAgreementStatus::Active,
            ServiceAgreementType::TeamTrial,
        )],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, None);
}

/// An `Active` SA of a non-self-serviceable type (`Legacy`) must NOT block
/// deletion — the server allows it via `IsSelfServicableAgreementType`.
#[test]
fn test_get_delete_disabled_reason_active_legacy_allows_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement_with_type(
            ServiceAgreementStatus::Active,
            ServiceAgreementType::Legacy,
        )],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, None);
}
