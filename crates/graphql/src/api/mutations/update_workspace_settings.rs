use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;
use crate::workspace::WorkspaceSettings;

// Note that `isInviteLinkEnabled` and `IsDiscoverable` aren't fetchable from this mutation;
// they don't come from the same code path as fetching org settings. We could change this
// but it's not something the server will populate at the moment.

/*
mutation UpdateWorkspaceSettings($input: UpdateWorkspaceSettingsInput!, $requestContext: RequestContext!) {
  updateWorkspaceSettings(requestContext: $requestContext, input: $input) {
    ... on UpdateWorkspaceSettingsOutput {
      workspaceSettings {
        llmSettings {
          enabled
        }
        telemetrySettings {
          forceEnabled
        }
        ugcCollectionSettings {
          setting
        }
        linkSharingSettings {
          anyoneWithLinkSharingEnabled
          directLinkSharingEnabled
        }
        secretRedactionSettings {
          enabled
          regexList
        }
        aiPermissionsSettings {
          allowAiInRemoteSessions
          remoteSessionRegexList
        }
        aiAutonomySettings {
          applyCodeDiffsSetting
          readFilesSetting
          readFilesAllowlist
          createPlansSetting
          executeCommandsSetting
          executeCommandsAllowlist
          executeCommandsDenylist
          writeToPtySetting
        }
        usageBasedPricingSettings {
          enabled
          maxMonthlySpendCents
        }
        addonCreditsSettings {
          autoReloadEnabled
          maxMonthlySpendCents
          selectedAutoReloadCreditDenomination
        }
        codebaseContextSettings {
          enabled
        }
      }
    }
  }
}
*/

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpdateWorkspaceSettingsVariables"
)]
pub struct UpdateWorkspaceSettings {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_workspace_settings: UpdateWorkspaceSettingsResult,
}
crate::client::define_operation! {
    update_workspace_settings(UpdateWorkspaceSettingsVariables) -> UpdateWorkspaceSettings;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateWorkspaceSettingsVariables {
    pub input: UpdateWorkspaceSettingsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateWorkspaceSettingsInput {
    pub workspace_uid: String,
    pub set_usage_based_pricing_settings: Option<UsageBasedPricingSettingsInput>,
    pub set_addon_credits_settings: Option<AddonCreditsSettingsInput>,
}

#[derive(cynic::InputObject, Debug)]
pub struct UsageBasedPricingSettingsInput {
    pub enabled: Option<bool>,
    pub max_monthly_spend_cents: Option<i32>,
}

#[derive(cynic::InputObject, Debug)]
pub struct AddonCreditsSettingsInput {
    pub auto_reload_enabled: Option<bool>,
    pub max_monthly_spend_cents: Option<i32>,
    pub selected_auto_reload_credit_denomination: Option<i32>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateWorkspaceSettingsOutput {
    pub response_context: ResponseContext,
    pub workspace_settings: WorkspaceSettings,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateWorkspaceSettingsResult {
    UpdateWorkspaceSettingsOutput(Box<UpdateWorkspaceSettingsOutput>),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
