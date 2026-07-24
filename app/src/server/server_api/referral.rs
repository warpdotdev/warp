use anyhow::{Result, anyhow};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
#[cfg(test)]
use mockall::{automock, predicate::*};
use warp_core::channel::ChannelState;
use warp_graphql::mutations::send_referral_invite_emails::{
    SendReferralInviteEmails, SendReferralInviteEmailsResult, SendReferralInviteEmailsVariables,
};
use warp_graphql::queries::get_referral_info::{GetReferralInfo, GetReferralInfoVariables};

use super::ServerApi;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};

/// Referral information for the logged-in user
pub struct ReferralInfo {
    /// Shareable URL that the user can use to invite friends
    pub url: String,
    /// The underlying referral code associated with the user
    pub code: String,
    /// Number of other users who have signed up with this user's referral code
    pub number_claimed: usize,
    /// Whether the user has been referred by another user
    pub is_referred: bool,
}

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait ReferralsClient: 'static + Send + Sync {
    /// Gets the user's referral information.
    async fn get_referral_info(&self) -> Result<ReferralInfo>;

    /// Send one or more email invites.
    async fn send_invite(&self, emails: Vec<String>) -> Result<Vec<String>>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ReferralsClient for ServerApi {
    async fn get_referral_info(&self) -> Result<ReferralInfo> {
        let variables = GetReferralInfoVariables {
            request_context: get_request_context(),
        };
        let operation = GetReferralInfo::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user {
            warp_graphql::queries::get_referral_info::UserResult::UserOutput(user_output) => {
                Ok(ReferralInfo {
                    url: format!(
                        "{}/referral/{}",
                        ChannelState::server_root_url(),
                        user_output.user.referrals.referral_code
                    ),
                    code: user_output.user.referrals.referral_code,
                    number_claimed: usize::try_from(user_output.user.referrals.number_claimed)
                        .expect("Negative referral count"),
                    is_referred: user_output.user.referrals.is_referred,
                })
            }
            warp_graphql::queries::get_referral_info::UserResult::Unknown => {
                Err(anyhow!("Unable to fetch referral info"))
            }
        }
    }

    async fn send_invite(&self, emails: Vec<String>) -> Result<Vec<String>> {
        let variables = SendReferralInviteEmailsVariables {
            input: warp_graphql::mutations::send_referral_invite_emails::SendReferralInviteEmailsInput {
                emails,
            },
            request_context: get_request_context(),
        };
        let operation = SendReferralInviteEmails::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        let send_referral_invite_emails_result = response.send_referral_invite_emails;

        match send_referral_invite_emails_result {
            SendReferralInviteEmailsResult::SendReferralInviteEmailsOutput(output) => {
                Ok(output.successful_emails)
            }
            SendReferralInviteEmailsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            SendReferralInviteEmailsResult::Unknown => Err(anyhow!(
                "unknown error while sending referral invite emails"
            )),
        }
    }
}
