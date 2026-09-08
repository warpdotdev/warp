use warp_graphql::ai::{
    AICreditAvailability as GqlAICreditAvailability,
    AICreditAvailabilityDenialReason as GqlDenialReason, AICreditAvailabilitySource as GqlSource,
};

use super::{AICreditAvailability, AICreditDenialReason, AICreditSource};

#[test]
fn converts_every_documented_denial_reason() {
    let cases = [
        (GqlDenialReason::None, AICreditDenialReason::None),
        (
            GqlDenialReason::OutOfCredits,
            AICreditDenialReason::OutOfCredits,
        ),
        (
            GqlDenialReason::Delinquent,
            AICreditDenialReason::Delinquent,
        ),
        (
            GqlDenialReason::EnterpriseTeamSpendLimitHit,
            AICreditDenialReason::EnterpriseTeamSpendLimitHit,
        ),
        (
            GqlDenialReason::EnterprisePerUserSpendLimitHit,
            AICreditDenialReason::EnterprisePerUserSpendLimitHit,
        ),
        (
            GqlDenialReason::EnterpriseWorkspaceSpendLimitHit,
            AICreditDenialReason::EnterpriseWorkspaceSpendLimitHit,
        ),
    ];
    for (gql, expected) in cases {
        assert_eq!(AICreditDenialReason::from(gql), expected);
    }
}

#[test]
fn converts_every_documented_credit_source() {
    let cases = [
        (GqlSource::BaseLimit, AICreditSource::BaseLimit),
        (GqlSource::BonusGrant, AICreditSource::BonusGrant),
        (GqlSource::Payg, AICreditSource::Payg),
        (GqlSource::Overage, AICreditSource::Overage),
        (
            GqlSource::AmbientBonusGrant,
            AICreditSource::AmbientBonusGrant,
        ),
    ];
    for (gql, expected) in cases {
        assert_eq!(AICreditSource::from(gql), expected);
    }
}

#[test]
fn converts_unknown_enum_values_to_unknown() {
    assert_eq!(
        AICreditDenialReason::from(GqlDenialReason::Other("FUTURE_REASON".to_string())),
        AICreditDenialReason::Unknown
    );
    assert_eq!(
        AICreditSource::from(GqlSource::Other("FUTURE_SOURCE".to_string())),
        AICreditSource::Unknown
    );
}

#[test]
fn converts_full_availability_payload() {
    let available = AICreditAvailability::from(GqlAICreditAvailability {
        available: true,
        denial_reason: GqlDenialReason::None,
        credit_source: Some(GqlSource::BaseLimit),
    });
    assert_eq!(
        available,
        AICreditAvailability::available_with_source(Some(AICreditSource::BaseLimit))
    );

    let capability_only = AICreditAvailability::from(GqlAICreditAvailability {
        available: true,
        denial_reason: GqlDenialReason::None,
        credit_source: None,
    });
    assert_eq!(
        capability_only,
        AICreditAvailability::available_with_source(None)
    );

    let denied = AICreditAvailability::from(GqlAICreditAvailability {
        available: false,
        denial_reason: GqlDenialReason::OutOfCredits,
        credit_source: None,
    });
    assert_eq!(
        denied,
        AICreditAvailability::unavailable(AICreditDenialReason::OutOfCredits)
    );
}
