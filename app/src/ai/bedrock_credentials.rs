/// Proactive Bedrock OIDC credential refresh for cloud agent sandboxes.
///
/// The `OidcManaged` Bedrock credential path mints an OIDC token at agent startup,
/// exchanges it for 1-hour STS temporary credentials via `AssumeRoleWithWebIdentity`,
/// and stores them in `ApiKeyManager`. Without a proactive refresh, those credentials
/// expire after ~1 hour and all subsequent Bedrock LLM calls fail.
///
/// This module provides a background `refresh_loop` — modelled on `git_credentials`'s
/// loop — that proactively re-mints the OIDC token and re-calls STS every 50 minutes,
/// well ahead of the 1-hour STS expiry. The loop is raced against the run execution
/// future via `futures::select!` and dropped automatically when the run completes.
use std::time::{Duration, SystemTime};

use ai::api_keys::{ApiKeyManager, AwsCredentials, AwsCredentialsState};
use anyhow::{Context as _, Result};
use vec1::vec1;
use warp_errors::report_error;
use warp_managed_secrets::ManagedSecretManager;
use warp_managed_secrets::client::IdentityTokenOptions;
use warpui::{ModelSpawner, SingletonEntity};

use super::agent_sdk::driver::AgentDriver;
use super::aws_credentials::{
    AWS_BEDROCK_STS_AUDIENCE, BEDROCK_IDENTITY_TOKEN_DURATION, aws_role_session_name, sts_client,
};

/// How long to wait between Bedrock credential refresh attempts — well ahead of the
/// 1-hour STS temporary credential expiry, matching the approach used for git credentials.
pub(crate) const BEDROCK_CREDENTIALS_REFRESH_INTERVAL: Duration = Duration::from_secs(50 * 60);

/// Perform one Bedrock OIDC credential refresh attempt.
///
/// Returns `Ok(())` on success. Returns `Err` when token minting or the STS call
/// fails — these are transient failures worth retrying.
#[tracing::instrument(
    name = "bedrock_credentials::try_refresh",
    skip_all,
    err,
    fields(tags.cloud_agent = true, task_id)
)]
async fn try_refresh(
    task_id: &str,
    role_arn: &str,
    region: &str,
    foreground: &ModelSpawner<AgentDriver>,
) -> Result<()> {
    // Step 1: Mint a new OIDC identity token via the model context.
    let token_future = foreground
        .spawn(|_, ctx| {
            ManagedSecretManager::handle(ctx)
                .as_ref(ctx)
                .issue_task_identity_token(IdentityTokenOptions {
                    audience: AWS_BEDROCK_STS_AUDIENCE.to_string(),
                    requested_duration: BEDROCK_IDENTITY_TOKEN_DURATION,
                    subject_template: vec1!["scoped_principal".to_string()],
                })
        })
        .await
        .context("Failed to dispatch OIDC token request for Bedrock refresh")?;

    let token = token_future
        .await
        .context("Failed to mint OIDC identity token for Bedrock refresh")?;

    // Step 2: Exchange the OIDC token for fresh STS temporary credentials.
    let client = sts_client(region).await;
    let session_name = aws_role_session_name(task_id);
    let sts_creds = client
        .assume_role_with_web_identity()
        .role_arn(role_arn)
        .role_session_name(&session_name)
        .web_identity_token(&token.token)
        .send()
        .await
        .map_err(|err| {
            let detail = err
                .as_service_error()
                .map(|e| e.to_string())
                .unwrap_or_else(|| err.to_string());
            report_error!(
                anyhow::Error::new(err)
                    .context("Bedrock OIDC refresh: STS AssumeRoleWithWebIdentity error")
            );
            anyhow::anyhow!("STS AssumeRoleWithWebIdentity failed: {detail}")
        })?
        .credentials
        .context("STS response did not include credentials")?;

    let aws_creds = AwsCredentials::new(
        sts_creds.access_key_id().to_string(),
        sts_creds.secret_access_key().to_string(),
        Some(sts_creds.session_token().to_string()),
        SystemTime::try_from(*sts_creds.expiration()).ok(),
    );

    // Step 3: Update ApiKeyManager with the fresh credentials.
    foreground
        .spawn(move |_, ctx| {
            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.set_aws_credentials_state(
                    AwsCredentialsState::Loaded {
                        credentials: aws_creds,
                        loaded_at: SystemTime::now(),
                    },
                    ctx,
                );
            });
        })
        .await
        .context("Failed to dispatch Bedrock credential update to ApiKeyManager")?;

    log::info!("Bedrock OIDC: proactive credential refresh succeeded for task {task_id}");
    Ok(())
}

/// Infinite async loop that proactively refreshes AWS Bedrock OIDC credentials every
/// [`BEDROCK_CREDENTIALS_REFRESH_INTERVAL`], keeping long-running agents authenticated
/// for their entire duration.
///
/// On each iteration:
/// 1. Issue a new OIDC identity token via warp-server.
/// 2. Call STS `AssumeRoleWithWebIdentity` to get fresh temporary credentials.
/// 3. Update `ApiKeyManager` with the new credentials.
///
/// On transient failure, retries up to three times with exponential backoff
/// (1 min, 2 min, 4 min), keeping all retries well within the buffer before
/// the 1-hour STS credentials expire. If all retries fail, a warning is logged
/// and the next refresh is scheduled after the normal interval.
///
/// This future never resolves — it is designed to be raced with the run execution
/// future via `futures::select!` and dropped automatically when the run completes.
pub(crate) async fn refresh_loop(
    task_id: String,
    role_arn: String,
    region: String,
    foreground: &ModelSpawner<AgentDriver>,
) {
    loop {
        warpui::r#async::Timer::after(BEDROCK_CREDENTIALS_REFRESH_INTERVAL).await;

        log::info!("Proactively refreshing AWS Bedrock OIDC credentials for task {task_id}");

        let backoff_delays = [
            Duration::from_secs(60),
            Duration::from_secs(2 * 60),
            Duration::from_secs(4 * 60),
        ];
        let mut attempt = 0usize;
        loop {
            match try_refresh(&task_id, &role_arn, &region, foreground).await {
                Ok(()) => break,
                Err(e) if attempt < backoff_delays.len() => {
                    let delay = backoff_delays[attempt];
                    log::warn!(
                        "Bedrock credentials refresh failed (attempt {}): {e:#}; \
                         retrying in {}s",
                        attempt + 1,
                        delay.as_secs()
                    );
                    warpui::r#async::Timer::after(delay).await;
                    attempt += 1;
                }
                Err(e) => {
                    log::warn!(
                        "Bedrock credentials refresh failed after {} attempts: {e:#}; \
                         credentials may expire before next refresh cycle",
                        attempt + 1
                    );
                    break;
                }
            }
        }
    }
}
