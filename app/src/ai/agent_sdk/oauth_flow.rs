use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use warp_graphql::queries::get_oauth_connect_tx_status::OauthConnectTxStatus;
use warpui::r#async::Timer;

use crate::server::server_api::integrations::IntegrationsClient;

/// Shared helpers for OAuth-based connect flows (txId + polling).
///
/// This is used by the integration CLI create flow, and can be reused by
/// other command flows that rely on the same OAuth connect transaction API.
pub async fn poll_oauth_until_terminal(
    integrations_client: Arc<dyn IntegrationsClient>,
    tx_id: String,
) -> Result<OauthConnectTxStatus> {
    const POLL_INTERVAL: Duration = Duration::from_secs(5);
    const MAX_ATTEMPTS: u32 = 120; // 10 minutes total
    // TODO(bens): render some kind of spinner here
    println!(
        "Waiting for authorization to complete... If this doesn't update after authorizing, please restart the command and try again.\n"
    );

    for attempt in 1..=MAX_ATTEMPTS {
        Timer::after(POLL_INTERVAL).await;

        let status = integrations_client
            .poll_oauth_connect_status(tx_id.clone())
            .await?;

        match status {
            OauthConnectTxStatus::Completed
            | OauthConnectTxStatus::Failed
            | OauthConnectTxStatus::Expired => {
                return Ok(status);
            }
            OauthConnectTxStatus::Pending | OauthConnectTxStatus::InProgress => {
                if attempt % 5 == 0 {
                    log::debug!("Still waiting for authorization... ({attempt}/{MAX_ATTEMPTS})",);
                }
            }
        }
    }

    Err(anyhow!("Timed out waiting for OAuth authorization"))
}
