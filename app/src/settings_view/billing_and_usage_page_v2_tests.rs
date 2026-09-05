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
        scope,
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

#[test]
fn excludes_grant_of_unknown_type_from_balance_cards() {
    // A grant type this client doesn't recognize (a fallback `Other` from a
    // newer server) must not be shown as spendable balance, since this
    // client cannot reason about how it may be spent.
    let current = workspace_uid(1);
    let grants = vec![grant(BonusGrantScope::User, BonusGrantType::Other, 10)];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert!(!classified.has_any());
}
