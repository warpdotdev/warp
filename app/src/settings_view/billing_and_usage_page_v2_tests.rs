use chrono::{Duration, Utc};

use super::*;

fn workspace_uid(id: i64) -> WorkspaceUid {
    WorkspaceUid::from(ServerId::from(id))
}

fn grant(scope: BonusGrantScope, grant_type: BonusGrantType, remaining: i32) -> BonusGrant {
    BonusGrant {
        created_at: Utc::now(),
        cost_cents: 0,
        expiration: None,
        grant_type,
        reason: "test".to_string(),
        user_facing_message: None,
        request_credits_granted: remaining,
        request_credits_remaining: remaining,
        usage_cents_granted: None,
        usage_cents_remaining: None,
        scope,
    }
}

fn grant_in_dollars(
    scope: BonusGrantScope,
    remaining: i32,
    usage_cents_remaining: f64,
) -> BonusGrant {
    BonusGrant {
        usage_cents_granted: Some(usage_cents_remaining),
        usage_cents_remaining: Some(usage_cents_remaining),
        ..grant(scope, BonusGrantType::Any, remaining)
    }
}

#[test]
fn classifies_grants_into_personal_team_and_workspace_buckets() {
    let current = workspace_uid(1);
    let grants = vec![
        grant(BonusGrantScope::User, BonusGrantType::Any, 10),
        grant(BonusGrantScope::Team(current), BonusGrantType::Any, 20),
        grant(BonusGrantScope::Workspace(current), BonusGrantType::Any, 30),
    ];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert_eq!(classified.personal.total_balance(), 10);
    assert_eq!(classified.team.total_balance(), 20);
    assert_eq!(classified.workspace.total_balance(), 30);
    assert!(classified.has_any());
}

#[test]
fn excludes_grants_scoped_to_a_different_workspace() {
    let current = workspace_uid(1);
    let other = workspace_uid(2);
    let grants = vec![
        grant(BonusGrantScope::Team(other), BonusGrantType::Any, 20),
        grant(BonusGrantScope::Workspace(other), BonusGrantType::Any, 30),
    ];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert!(classified.team.is_empty());
    assert!(classified.workspace.is_empty());
    assert!(!classified.has_any());
}

#[test]
fn hides_buckets_with_no_grants() {
    let current = workspace_uid(1);
    let grants = vec![grant(BonusGrantScope::User, BonusGrantType::Any, 10)];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert!(!classified.personal.is_empty());
    assert!(classified.team.is_empty());
    assert!(classified.workspace.is_empty());
}

#[test]
fn bucket_balance_is_dollars_only_when_shown_in_dollars_with_every_grant_priced() {
    let current = workspace_uid(1);
    let dollars = ClassifiedGrants::new(
        &[
            grant_in_dollars(BonusGrantScope::Team(current), 10, 18.0),
            grant_in_dollars(BonusGrantScope::Team(current), 20, 36.0),
        ],
        Some(current),
    );
    assert_eq!(dollars.team.total_usage_cents_balance(), Some(54.0));
    assert_eq!(
        dollars.team.balance(UsageDisplayUnit::Dollars),
        BalanceAmount::Cents(54.0)
    );
    assert_eq!(dollars.team.total_balance(), 30);
    // Cents the server sends are never shown when displaying in credits.
    assert_eq!(
        dollars.team.balance(UsageDisplayUnit::Credits),
        BalanceAmount::Credits(30)
    );

    let mixed = ClassifiedGrants::new(
        &[
            grant_in_dollars(BonusGrantScope::Team(current), 10, 18.0),
            grant(BonusGrantScope::Team(current), BonusGrantType::Any, 20),
        ],
        Some(current),
    );
    assert_eq!(mixed.team.total_usage_cents_balance(), None);
    assert_eq!(
        mixed.team.balance(UsageDisplayUnit::Dollars),
        BalanceAmount::Credits(30)
    );
}

#[test]
fn balance_amount_formats_in_its_unit() {
    assert_eq!(BalanceAmount::Credits(1_500).format(), "1,500");
    assert_eq!(
        BalanceAmount::Credits(1_500).pool_label("Team"),
        "Team credits"
    );
    assert_eq!(BalanceAmount::Cents(1729.8).format(), "$17.30");
    assert_eq!(
        BalanceAmount::Cents(1729.8).pool_label("Team"),
        "Team usage"
    );
}

#[test]
fn base_allowance_balance_uses_dollars_when_shown_in_dollars() {
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Dollars,
            1_000,
            39,
            false,
            Some(1800.0),
            Some(75.0)
        ),
        (
            BalanceAmount::Cents(1725.0),
            Some(BalanceAmount::Cents(1800.0))
        )
    );
    // Overspend never shows a negative balance.
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Dollars,
            1_000,
            1_000,
            false,
            Some(1800.0),
            Some(1850.0)
        ),
        (
            BalanceAmount::Cents(0.0),
            Some(BalanceAmount::Cents(1800.0))
        )
    );
}

#[test]
fn base_allowance_balance_falls_back_to_credits() {
    assert_eq!(
        base_allowance_balance(UsageDisplayUnit::Dollars, 1_000, 39, false, None, None),
        (
            BalanceAmount::Credits(961),
            Some(BalanceAmount::Credits(1_000))
        )
    );
    // A missing used figure means the dollar balance is unknown.
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Dollars,
            1_000,
            39,
            false,
            Some(1800.0),
            None
        ),
        (
            BalanceAmount::Credits(961),
            Some(BalanceAmount::Credits(1_000))
        )
    );
    // Cents the server sends are never shown when displaying in credits.
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Credits,
            1_000,
            39,
            false,
            Some(1800.0),
            Some(75.0)
        ),
        (
            BalanceAmount::Credits(961),
            Some(BalanceAmount::Credits(1_000))
        )
    );
    // Unlimited subjects keep today's display.
    assert_eq!(
        base_allowance_balance(UsageDisplayUnit::Dollars, 999_999, 39, true, None, None),
        (BalanceAmount::Credits(999_960), None)
    );
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Dollars,
            999_999,
            39,
            true,
            Some(1800.0),
            Some(70.2)
        ),
        (BalanceAmount::Credits(999_960), None)
    );
}

#[test]
fn excludes_ambient_expired_and_depleted_grants() {
    let current = workspace_uid(1);
    let mut expired = grant(BonusGrantScope::Team(current), BonusGrantType::Any, 5);
    expired.expiration = Some(Utc::now() - Duration::days(1));

    let grants = vec![
        // Ambient-only credits are surfaced separately, not as balance cards.
        grant(
            BonusGrantScope::Workspace(current),
            BonusGrantType::AmbientOnly,
            100,
        ),
        // A grant with no credits left should not render a card.
        grant(BonusGrantScope::Team(current), BonusGrantType::Any, 0),
        expired,
    ];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert!(!classified.has_any());
}
