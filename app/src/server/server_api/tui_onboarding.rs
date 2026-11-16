use anyhow::{Result, anyhow};
use async_trait::async_trait;
use cynic::QueryBuilder;
#[cfg(test)]
use mockall::automock;
use warp_graphql::queries::tui_onboarding_markers::{
    TuiOnboardingMarkers, TuiOnboardingMarkersResult, TuiOnboardingMarkersVariables,
};

use super::ServerApi;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiOnboardingMarkersSnapshot {
    pub first_zero_state_shown: bool,
    pub first_credit_gate_shown: bool,
}

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait TuiOnboardingClient: 'static + Send + Sync {
    async fn get_tui_onboarding_markers(&self) -> Result<TuiOnboardingMarkersSnapshot>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl TuiOnboardingClient for ServerApi {
    async fn get_tui_onboarding_markers(&self) -> Result<TuiOnboardingMarkersSnapshot> {
        let operation = TuiOnboardingMarkers::build(TuiOnboardingMarkersVariables {
            request_context: get_request_context(),
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.tui_onboarding_markers {
            TuiOnboardingMarkersResult::TuiOnboardingMarkersOutput(output) => {
                Ok(TuiOnboardingMarkersSnapshot {
                    first_zero_state_shown: output.first_zero_state_shown,
                    first_credit_gate_shown: output.first_credit_gate_shown,
                })
            }
            TuiOnboardingMarkersResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            TuiOnboardingMarkersResult::Unknown => {
                Err(anyhow!("Unable to load TUI onboarding markers"))
            }
        }
    }
}
