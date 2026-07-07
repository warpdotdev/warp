use crate::billing::PricingInfo;
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
        workspaces {
          uid
          name
          members {
            uid
            email
            role
          }
          teams {
            uid
            name
            members {
              uid
              email
              role
            }
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
              }
            }
          }
          settings {
            isDiscoverable
            isInviteLinkEnabled
            llmSettings {
              enabled
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
          inviteCode
          pendingEmailInvites {
            email
            expired
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
    pub workspaces: Vec<Workspace>,
    pub experiments: Option<Vec<Experiment>>,
    pub discoverable_teams: Vec<DiscoverableTeamData>,
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
