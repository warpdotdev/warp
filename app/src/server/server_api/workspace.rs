use anyhow::{anyhow, Result};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
#[cfg(test)]
use mockall::{automock, predicate::*};
use warp_graphql::error::UserFacingErrorInterface;
use warp_graphql::mutations::purchase_addon_credits::{
    PurchaseAddonCredits, PurchaseAddonCreditsInput, PurchaseAddonCreditsResult,
    PurchaseAddonCreditsVariables,
};
use warp_graphql::mutations::stripe_billing_portal::{
    StripeBillingPortal, StripeBillingPortalInput, StripeBillingPortalResult,
    StripeBillingPortalVariables,
};
use warp_graphql::mutations::update_workspace_settings::{
    AddonCreditsSettingsInput, UpdateWorkspaceSettings, UpdateWorkspaceSettingsInput,
    UpdateWorkspaceSettingsResult, UpdateWorkspaceSettingsVariables,
    UsageBasedPricingSettingsInput,
};
use warp_graphql::queries::get_ai_overages_for_workspace::{
    GetAiOveragesForWorkspace, GetAiOveragesForWorkspaceVariables, UserResult,
};

use super::team::TeamClient;
use super::ServerApi;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};
use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::WorkspacesMetadataResponse;
use crate::workspaces::workspace::AiOverages;

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait WorkspaceClient: 'static + Send + Sync {
    async fn generate_stripe_billing_portal_link(&self, team_uid: ServerId) -> Result<String>;

    async fn update_usage_based_pricing_settings(
        &self,
        team_uid: ServerId,
        usage_based_pricing_enabled: bool,
        max_monthly_spend_cents: Option<u32>,
    ) -> Result<WorkspacesMetadataResponse>;

    async fn refresh_ai_overages(&self) -> Result<AiOverages>;

    async fn purchase_addon_credits(
        &self,
        team_uid: ServerId,
        credits: i32,
    ) -> Result<WorkspacesMetadataResponse>;

    async fn update_addon_credits_settings(
        &self,
        team_uid: ServerId,
        auto_reload_enabled: Option<bool>,
        max_monthly_spend_cents: Option<i32>,
        selected_auto_reload_credit_denomination: Option<i32>,
    ) -> Result<WorkspacesMetadataResponse>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl WorkspaceClient for ServerApi {
    async fn generate_stripe_billing_portal_link(&self, team_uid: ServerId) -> Result<String> {
        let variables = StripeBillingPortalVariables {
            input: StripeBillingPortalInput {
                team_uid: team_uid.into(),
            },
            request_context: get_request_context(),
        };
        let operation = StripeBillingPortal::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.stripe_billing_portal {
            StripeBillingPortalResult::StripeBillingPortalOutput(output) => Ok(output.url),
            StripeBillingPortalResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            StripeBillingPortalResult::Unknown => Err(anyhow!("Unknown error")),
        }
    }

    async fn update_usage_based_pricing_settings(
        &self,
        team_uid: ServerId,
        usage_based_pricing_enabled: bool,
        max_monthly_spend_cents: Option<u32>,
    ) -> Result<WorkspacesMetadataResponse> {
        if let Some(cents) = max_monthly_spend_cents {
            if cents > i32::MAX as u32 {
                return Err(anyhow!(
                    "Maximum monthly spend cannot exceed {} cents",
                    i32::MAX
                ));
            }
        }

        let variables = UpdateWorkspaceSettingsVariables {
            input: UpdateWorkspaceSettingsInput {
                workspace_uid: team_uid.to_string(),
                set_usage_based_pricing_settings: Some(UsageBasedPricingSettingsInput {
                    enabled: Some(usage_based_pricing_enabled),
                    max_monthly_spend_cents: max_monthly_spend_cents.map(|cents| cents as i32),
                }),
                set_addon_credits_settings: None,
            },
            request_context: get_request_context(),
        };
        let operation = UpdateWorkspaceSettings::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.update_workspace_settings {
            UpdateWorkspaceSettingsResult::UpdateWorkspaceSettingsOutput(_) => {
                TeamClient::workspaces_metadata(self)
                    .await
                    .map(|w| w.metadata)
            }
            UpdateWorkspaceSettingsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            UpdateWorkspaceSettingsResult::Unknown => Err(anyhow!("Unknown error")),
        }
    }

    async fn refresh_ai_overages(&self) -> Result<AiOverages> {
        let variables = GetAiOveragesForWorkspaceVariables {
            request_context: get_request_context(),
        };
        let operation = GetAiOveragesForWorkspace::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user {
            UserResult::UserOutput(user_output) => user_output
                .user
                .workspaces
                .first()
                .as_ref()
                .ok_or_else(|| anyhow!("No workspace found"))?
                .billing_metadata
                .ai_overages
                .as_ref()
                .ok_or_else(|| anyhow!("No AI overages found"))
                .map(|overages| AiOverages {
                    current_monthly_request_cost_cents: overages.current_monthly_request_cost_cents,
                    current_monthly_requests_used: overages.current_monthly_requests_used,
                    current_period_end: overages.current_period_end.utc(),
                }),
            UserResult::Unknown => Err(anyhow!("Unknown error")),
        }
    }

    async fn purchase_addon_credits(
        &self,
        team_uid: ServerId,
        credits: i32,
    ) -> Result<WorkspacesMetadataResponse> {
        let variables = PurchaseAddonCreditsVariables {
            input: PurchaseAddonCreditsInput {
                team_uid: team_uid.into(),
                credits,
            },
            request_context: get_request_context(),
        };
        let operation = PurchaseAddonCredits::build(variables);
        let response = self.send_graphql_request(operation, None).await;

        match response {
            Err(_) => Err(anyhow!("Failed to purchase add-on credits")),
            Ok(response) => match response.purchase_addon_credits {
                PurchaseAddonCreditsResult::PurchaseAddonCreditsOutput(_) => {
                    TeamClient::workspaces_metadata(self)
                        .await
                        .map(|w| w.metadata)
                }
                PurchaseAddonCreditsResult::UserFacingError(error) => match error.error {
                    UserFacingErrorInterface::BudgetExceededError(budget_error) => {
                        Err(budget_error.into())
                    }
                    UserFacingErrorInterface::PaymentMethodDeclinedError(
                        payment_declined_error,
                    ) => Err(payment_declined_error.into()),
                    _ => Err(anyhow!(get_user_facing_error_message(error))),
                },
                PurchaseAddonCreditsResult::Unknown => Err(anyhow!("Unknown error")),
            },
        }
    }

    async fn update_addon_credits_settings(
        &self,
        team_uid: ServerId,
        auto_reload_enabled: Option<bool>,
        max_monthly_spend_cents: Option<i32>,
        selected_auto_reload_credit_denomination: Option<i32>,
    ) -> Result<WorkspacesMetadataResponse> {
        let variables = UpdateWorkspaceSettingsVariables {
            input: UpdateWorkspaceSettingsInput {
                workspace_uid: team_uid.to_string(),
                set_usage_based_pricing_settings: None,
                set_addon_credits_settings: Some(AddonCreditsSettingsInput {
                    auto_reload_enabled,
                    max_monthly_spend_cents,
                    selected_auto_reload_credit_denomination,
                }),
            },
            request_context: get_request_context(),
        };
        let operation = UpdateWorkspaceSettings::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.update_workspace_settings {
            UpdateWorkspaceSettingsResult::UpdateWorkspaceSettingsOutput(_) => {
                TeamClient::workspaces_metadata(self)
                    .await
                    .map(|w| w.metadata)
            }
            UpdateWorkspaceSettingsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            UpdateWorkspaceSettingsResult::Unknown => Err(anyhow!("Unknown error")),
        }
    }
}
