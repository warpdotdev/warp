use crate::ai::AICreditAvailability;
use crate::billing::{PricingInfo, PurchaseAddOnCreditsPolicy};
use crate::experiment::Experiment;
use crate::request_context::RequestContext;
use crate::schema;
use crate::user::DiscoverableTeamData;
use crate::workspace::Workspace;

/*
query GetWorkspacesMetadataForUser($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        profile {
          uid
        }
        aiCreditAvailability {
          available
          denialReason
          creditSource
        }
        billingMetadata {
          tier {
            purchaseAddOnCreditsPolicy {
              enabled
              premiumEnabled
              pricePremiumBps
            }
          }
        }
        workspaces {
          uid
          name
          members {
            uid
            email
            role
            isDisabled
          }
          teams {
            uid
            name
            inviteLink
            members {
              uid
              email
              role
              isDisabled
            }
            visibility
            featureModelChoice { ... }
          }
          billingMetadata {
            customerType
            delinquencyStatus
            tier {
              name
              description
              warpAiPolicy {
                limit
                isCodeSuggestionsToggleable
                isPromptSuggestionsToggleable
                isNextCommandEnabled
                isGitOperationsAiEnabled
                isVoiceEnabled
              }
              teamSizePolicy {
                isUnlimited
                limit
              }
              sharedNotebooksPolicy {
                isUnlimited
                limit
              }
              sharedWorkflowsPolicy {
                isUnlimited
                limit
              }
              sessionSharingPolicy {
                enabled
                maxSessionBytesSize
              }
              anyoneWithLinkSharingPolicy {
                toggleable
              }
              directLinkSharingPolicy {
                toggleable
              }
              byoApiKeyPolicy {
                enabled
              }
              byoEndpointPolicy {
                enabled
              }
              managedByokByoePolicy {
                enabled
              }
              usageVisibilityPolicy {
                adminGranularity
                maxPriorCycles
              }
              pricing {
                enablePayAsYouGo
                autoReloadCreditDenomination
                autoReloadCostCents
              }
            }
            serviceAgreements {
              currentPeriodEnd
              status
              stripeSubscriptionId
              type
            }
          }
          billingCycleUsageHistory {
            currentPeriodStart
            currentPeriodEnd
            summaries {
              periodStart
              periodEnd
              entries {
                subjectType
                subjectUid
                subjectDisplayName
                costType
                usageBucket
                usageSource
                creditsUsed
                costCents
                attributedTeamUid
              }
            }
          }
          settings {
            isDiscoverable
            isInviteLinkEnabled
            llmSettings {
              enabled
            }
            teamByo {
              firstPartyEnabled
              endpointsEnabled
              allowUserKeys
              allowUserEndpoints
              firstPartyKeys {
                provider
                credentialUid
              }
              endpoints {
                uid
                name
                enabled
                credentialUid
                models {
                  configKey
                  slug
                  alias
                  displayName
                  enabled
                }
              }
            }
            telemetrySettings {
              forceEnabled
            }
            linkSharingSettings {
              anyoneWithLinkSharingEnabled
              directLinkSharingEnabled
            }
            codebaseContextSettings {
              enabled
            }
          }
          hasBillingHistory
          pendingEmailInvites {
            email
            expired
            teamUid
          }
          inviteLinkDomainRestrictions {
            uid
            domain
          }
          stripeCustomerId
          isEligibleForDiscovery
        }
        experiments
        discoverableTeams {
          teamUid
          numMembers
          name
          teamAcceptingInvites
        }
      }
    }
  }
  pricingInfo(requestContext: $requestContext) {
    ... on PricingInfoOutput {
      pricingInfo {
        plans {
          plan
          monthlyPlanPricePerMonthUsdCents
          yearlyPlanPricePerMonthUsdCents
          requestLimit
          codebaseLimit
          codebaseContextFileLimit
          maxTeamSize
        }
        overages {
          pricePerRequestUsdCents
        }
        promotionMessage
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct GetWorkspacesMetadataForUserVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct PricingInfoOutput {
    pub pricing_info: PricingInfo,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum PricingInfoResult {
    PricingInfoOutput(PricingInfoOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub profile: UserProfile,
    pub ai_credit_availability: AICreditAvailability,
    pub billing_metadata: Option<UserPurchasePolicyBillingMetadata>,
    pub workspaces: Vec<Workspace>,
    pub experiments: Option<Vec<Experiment>>,
    pub discoverable_teams: Vec<DiscoverableTeamData>,
}

/// Slim selection of the user-level `billingMetadata`: only the add-on
/// credits purchase policy. This is the teamless-purchase fallback (fresh
/// free users have no team and their only workspace is the server's
/// placeholder) — do not widen it into the full `BillingMetadata` selection.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "BillingMetadata")]
pub struct UserPurchasePolicyBillingMetadata {
    pub tier: UserPurchasePolicyTier,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Tier")]
pub struct UserPurchasePolicyTier {
    pub purchase_add_on_credits_policy: Option<PurchaseAddOnCreditsPolicy>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "FirebaseProfile")]
pub struct UserProfile {
    pub uid: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetWorkspacesMetadataForUserVariables"
)]
pub struct GetWorkspacesMetadataForUser {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
    #[arguments(requestContext: $request_context)]
    pub pricing_info: PricingInfoResult,
}
crate::client::define_operation! {
    get_workspaces_metadata_for_user(GetWorkspacesMetadataForUserVariables) -> GetWorkspacesMetadataForUser;
}
