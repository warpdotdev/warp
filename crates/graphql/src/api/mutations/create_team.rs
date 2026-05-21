use crate::{
    error::UserFacingError, object::CloudObjectEventEntrypoint, request_context::RequestContext,
    response_context::ResponseContext, schema, workspace::Workspace,
};

/*
mutation CreateTeam($input: CreateTeamInput!, $request_context: RequestContext!) {
  createTeam(input: $input, requestContext: $request_context) {
    ... on CreateTeamOutput {
      workspace {
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
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      error {
        message
      }
      responseContext {
        serverVersion
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateTeamVariables {
    pub input: CreateTeamInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateTeamInput {
    pub discoverable: bool,
    pub entrypoint: CloudObjectEventEntrypoint,
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "CreateTeamVariables")]
pub struct CreateTeam {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_team: CreateTeamResult,
}
crate::client::define_operation! {
    create_team(CreateTeamVariables) -> CreateTeam;
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CreateTeamResult {
    CreateTeamOutput(CreateTeamOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateTeamOutput {
    pub workspace: Workspace,
    pub response_context: ResponseContext,
}
