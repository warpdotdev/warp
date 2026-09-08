//! Domain types for the server-authoritative AI credit availability decision
//! (`User.aiCreditAvailability`). The server evaluates the same credit
//! waterfall used to authorize AI requests, so these values are the source of
//! truth for whether the user can start an interactive AI request.
use warp_graphql::ai::{
    AICreditAvailability as GqlAICreditAvailability,
    AICreditAvailabilityDenialReason as GqlAICreditAvailabilityDenialReason,
    AICreditAvailabilitySource as GqlAICreditAvailabilitySource,
};

/// Stable, client-safe reason the server reports when no inference access
/// exists. `None` is reported when the user is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AICreditDenialReason {
    None,
    OutOfCredits,
    Delinquent,
    EnterpriseTeamSpendLimitHit,
    EnterprisePerUserSpendLimitHit,
    EnterpriseWorkspaceSpendLimitHit,
    /// A reason from a newer server that this client version doesn't know.
    /// Treated as a generic denial for presentation purposes.
    Unknown,
}

/// The credit source the server selected when inference access exists.
/// Capability-only access (e.g. BYO API key) has no credit source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AICreditSource {
    BaseLimit,
    BonusGrant,
    Payg,
    Overage,
    AmbientBonusGrant,
    /// A source from a newer server that this client version doesn't know.
    Unknown,
}

/// The server-authoritative answer to "can this user start an interactive AI
/// request right now".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AICreditAvailability {
    pub available: bool,
    pub denial_reason: AICreditDenialReason,
    pub credit_source: Option<AICreditSource>,
}

impl AICreditAvailability {
    pub fn available_with_source(credit_source: Option<AICreditSource>) -> Self {
        Self {
            available: true,
            denial_reason: AICreditDenialReason::None,
            credit_source,
        }
    }

    pub fn unavailable(denial_reason: AICreditDenialReason) -> Self {
        Self {
            available: false,
            denial_reason,
            credit_source: None,
        }
    }
}

impl From<GqlAICreditAvailabilityDenialReason> for AICreditDenialReason {
    fn from(reason: GqlAICreditAvailabilityDenialReason) -> Self {
        match reason {
            GqlAICreditAvailabilityDenialReason::None => Self::None,
            GqlAICreditAvailabilityDenialReason::OutOfCredits => Self::OutOfCredits,
            GqlAICreditAvailabilityDenialReason::Delinquent => Self::Delinquent,
            GqlAICreditAvailabilityDenialReason::EnterpriseTeamSpendLimitHit => {
                Self::EnterpriseTeamSpendLimitHit
            }
            GqlAICreditAvailabilityDenialReason::EnterprisePerUserSpendLimitHit => {
                Self::EnterprisePerUserSpendLimitHit
            }
            GqlAICreditAvailabilityDenialReason::EnterpriseWorkspaceSpendLimitHit => {
                Self::EnterpriseWorkspaceSpendLimitHit
            }
            GqlAICreditAvailabilityDenialReason::Other(_) => Self::Unknown,
        }
    }
}

impl From<GqlAICreditAvailabilitySource> for AICreditSource {
    fn from(source: GqlAICreditAvailabilitySource) -> Self {
        match source {
            GqlAICreditAvailabilitySource::BaseLimit => Self::BaseLimit,
            GqlAICreditAvailabilitySource::BonusGrant => Self::BonusGrant,
            GqlAICreditAvailabilitySource::Payg => Self::Payg,
            GqlAICreditAvailabilitySource::Overage => Self::Overage,
            GqlAICreditAvailabilitySource::AmbientBonusGrant => Self::AmbientBonusGrant,
            GqlAICreditAvailabilitySource::Other(_) => Self::Unknown,
        }
    }
}

impl From<GqlAICreditAvailability> for AICreditAvailability {
    fn from(availability: GqlAICreditAvailability) -> Self {
        Self {
            available: availability.available,
            denial_reason: availability.denial_reason.into(),
            credit_source: availability.credit_source.map(Into::into),
        }
    }
}

#[cfg(test)]
#[path = "credit_availability_tests.rs"]
mod tests;
