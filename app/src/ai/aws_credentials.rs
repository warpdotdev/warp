use std::time::{Duration, SystemTime};

use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::terminal::event::{AfterBlockCompletedEvent, BlockType, UserBlockCompleted};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
pub use ai::api_keys::AwsCredentials;
use ai::api_keys::{ApiKeyManager, AwsCredentialsRefreshStrategy, AwsCredentialsState};
use anyhow::Context;
use aws_credential_types::provider::error::CredentialsError;
use aws_credential_types::provider::ProvideCredentials;
use futures::channel::oneshot::channel;
use futures::future::BoxFuture;
use tokio::sync::OnceCell;
use vec1::vec1;
use warp_managed_secrets::{client::IdentityTokenOptions, ManagedSecretManager};
use warpui::{ModelContext, ModelHandle, SingletonEntity};

/// Errors that can occur when loading AWS credentials.
#[derive(Debug, Clone)]
pub enum LoadAwsCredentialsError {
    /// No AWS credentials are configured on this machine.
    /// The user needs to configure credentials via environment variables,
    /// shared credentials file (~/.aws/credentials), or other AWS credential sources.
    NotConfigured,
    /// AWS credentials are configured but could not be loaded.
    /// This can happen when credentials are expired, invalid, or the
    /// credential source (e.g., SSO session) needs to be refreshed.
    CredentialsLoadFailed(String),
}

impl std::fmt::Display for LoadAwsCredentialsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "No AWS credentials configured"),
            Self::CredentialsLoadFailed(msg) => {
                write!(f, "Failed to load AWS credentials: {msg}")
            }
        }
    }
}

fn aws_profile_reference_for_message(profile: &str, capitalize_first_word: bool) -> String {
    let profile = profile.trim();
    if profile.is_empty() {
        if capitalize_first_word {
            "The default AWS profile".to_string()
        } else {
            "the default AWS profile".to_string()
        }
    } else {
        let article = if capitalize_first_word { "The" } else { "the" };
        format!("{article} AWS profile `{profile}`")
    }
}

fn user_facing_aws_credentials_error_message(err: &CredentialsError, profile: &str) -> String {
    match err {
        CredentialsError::CredentialsNotLoaded(_) => format!(
            "AWS credentials were not found for {}. Log in with the AWS CLI or update your AWS credentials configuration, then refresh.",
            aws_profile_reference_for_message(profile, false)
        ),
        CredentialsError::ProviderTimedOut(_) => {
            "Timed out while loading AWS credentials. Refresh and try again.".to_string()
        }
        CredentialsError::InvalidConfiguration(_) => format!(
            "{} is invalid or incomplete in your local AWS configuration. Update your AWS profile settings and credentials, then refresh.",
            aws_profile_reference_for_message(profile, true)
        ),
        CredentialsError::ProviderError(_) => {
            "Unable to load AWS credentials from your configured provider. Refresh your AWS login and try again."
                .to_string()
        }
        CredentialsError::Unhandled(_) => {
            "Unexpected error while loading AWS credentials. Refresh your AWS login and try again."
                .to_string()
        }
        _ => "Unable to load AWS credentials. Refresh your AWS login and try again."
            .to_string(),
    }
}

impl std::error::Error for LoadAwsCredentialsError {}

const AWS_BEDROCK_STS_AUDIENCE: &str = "sts.amazonaws.com";
const BEDROCK_IDENTITY_TOKEN_DURATION: Duration = Duration::from_secs(60 * 60);

pub(crate) fn aws_role_session_name(run_id: &str) -> String {
    format!("Oz_Run_{run_id}")
}

/// Cached STS client for OIDC credential refreshes.
///
/// `AssumeRoleWithWebIdentity` is unauthenticated (the web identity token is the
/// credential), so we skip the default credentials chain via `no_credentials()`
/// and reuse a single client across refreshes.
static STS_CLIENT: OnceCell<aws_sdk_sts::Client> = OnceCell::const_new();

async fn sts_client() -> &'static aws_sdk_sts::Client {
    STS_CLIENT
        .get_or_init(|| async {
            let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .no_credentials()
                .load()
                .await;
            aws_sdk_sts::Client::new(&config)
        })
        .await
}

fn aws_credentials_state_for_error(err: LoadAwsCredentialsError) -> AwsCredentialsState {
    match err {
        LoadAwsCredentialsError::NotConfigured => AwsCredentialsState::Missing,
        LoadAwsCredentialsError::CredentialsLoadFailed(message) => {
            AwsCredentialsState::Failed { message }
        }
    }
}

/// Loads AWS credentials from the AWS SDK.
///
/// # Arguments
/// * `profile` - AWS profile name. If empty, uses the default AWS SDK behavior
///   (checks AWS_PROFILE env var, then uses "default").
pub async fn load_aws_credentials_from_sdk(
    profile: &str,
) -> Result<AwsCredentials, LoadAwsCredentialsError> {
    let region_provider = aws_config::meta::region::RegionProviderChain::default_provider();
    let loader =
        aws_config::defaults(aws_config::BehaviorVersion::latest()).region(region_provider);
    let loader = if profile.trim().is_empty() {
        loader // Let AWS SDK use its default behavior
    } else {
        loader.profile_name(profile)
    };
    let config = loader.load().await;

    let provider = config
        .credentials_provider()
        .ok_or(LoadAwsCredentialsError::NotConfigured)?;

    let creds = provider.provide_credentials().await.map_err(|e| {
        let message = user_facing_aws_credentials_error_message(&e, profile);
        log::warn!("{e}");
        // TODO(isaiah): turn this full SDK dump back down to debug once we've resolved
        // the current customer-facing AWS credential issue and no longer need prod-visible
        // provider internals for support debugging.
        log::warn!("{e:#?}");
        log::info!("AWS credential load failure message shown to user: {message}");
        LoadAwsCredentialsError::CredentialsLoadFailed(message)
    })?;

    Ok(AwsCredentials::new(
        creds.access_key_id().to_string(),
        creds.secret_access_key().to_string(),
        creds.session_token().map(|s| s.to_string()),
        creds.expiry(),
    ))
}

/// Extension trait for `ApiKeyManager` to handle AWS credential refresh.
pub trait AwsCredentialRefresher {
    /// Registers a `ModelEventDispatcher` to listen for block completion events.
    /// When a user executes a command matching the AWS auth refresh command,
    /// this will automatically refresh AWS credentials.
    fn register_model_event_dispatcher(
        &mut self,
        model_events: &ModelHandle<ModelEventDispatcher>,
        ctx: &mut ModelContext<Self>,
    ) where
        Self: Sized;

    /// Sets up subscriptions to `UserWorkspaces` and `AISettings` to refresh AWS credentials
    /// when workspace settings or AWS Bedrock settings change.
    fn subscribe_to_settings_changes(&mut self, ctx: &mut ModelContext<Self>)
    where
        Self: Sized;
}

impl AwsCredentialRefresher for ApiKeyManager {
    fn register_model_event_dispatcher(
        &mut self,
        model_events: &ModelHandle<ModelEventDispatcher>,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.subscribe_to_model(model_events, |manager, event, ctx| {
            if let ModelEvent::AfterBlockCompleted(AfterBlockCompletedEvent {
                block_type: BlockType::User(UserBlockCompleted { command, .. }),
                ..
            }) = event
            {
                let auth_command = &AISettings::as_ref(ctx).aws_bedrock_auth_refresh_command;
                if command.trim().starts_with(auth_command.trim()) {
                    log::debug!("Detected AWS auth command completion, refreshing credentials");
                    drop(refresh_aws_credentials(manager, ctx));
                }
            }
        });
    }

    fn subscribe_to_settings_changes(&mut self, ctx: &mut ModelContext<Self>) {
        // Subscribe to UserWorkspaces events to refresh AWS credentials when workspace settings change
        // (this also initializes AWS credentials on app startup via TeamsChanged)
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |manager, event, ctx| {
            if matches!(
                event,
                UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess
                    | UserWorkspacesEvent::TeamsChanged
            ) {
                drop(refresh_aws_credentials(manager, ctx));
            }
        });

        // Subscribe to AISettings changes to refresh AWS credentials when AWS Bedrock settings change
        ctx.subscribe_to_model(&AISettings::handle(ctx), |manager, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::AwsBedrockProfile { .. }
                    | AISettingsChangedEvent::AwsBedrockAuthRefreshCommand { .. }
                    | AISettingsChangedEvent::AwsBedrockCredentialsEnabled { .. }
            ) {
                drop(refresh_aws_credentials(manager, ctx));
            }
        });
    }
}
/// Refreshes AWS credentials, dispatching to the appropriate strategy.
///
/// Returns a future that resolves when the refresh completes. Subscription-triggered
/// callers that don't need to wait should drop the returned future — the underlying
/// work has already been scheduled on the executor by the time this returns.
pub(crate) fn refresh_aws_credentials(
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) -> BoxFuture<'static, Result<(), String>> {
    match manager.aws_credentials_refresh_strategy() {
        AwsCredentialsRefreshStrategy::LocalChain => {
            refresh_aws_credentials_local_chain(manager, ctx)
        }
        AwsCredentialsRefreshStrategy::OidcManaged { task_id, role_arn } => {
            refresh_aws_credentials_oidc(task_id, role_arn, manager, ctx)
        }
    }
}

/// Refreshes credentials from the local AWS SDK credential chain (~/.aws).
fn refresh_aws_credentials_local_chain(
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) -> BoxFuture<'static, Result<(), String>> {
    let is_available = UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx);

    if !is_available {
        manager.set_aws_credentials_state(AwsCredentialsState::Disabled, ctx);
        return Box::pin(async { Ok(()) });
    }

    let profile = (*AISettings::as_ref(ctx).aws_bedrock_profile).clone();

    manager.set_aws_credentials_state(AwsCredentialsState::Refreshing, ctx);

    let (tx, rx) = channel();
    // credential fetch from aws cli's disk cache
    let _ = ctx.spawn(
        async move { load_aws_credentials_from_sdk(&profile).await },
        move |manager, result, ctx| {
            let (new_state, tx_result) = match result {
                Ok(credentials) => (
                    AwsCredentialsState::Loaded {
                        credentials,
                        loaded_at: SystemTime::now(),
                    },
                    Ok(()),
                ),
                Err(err) => {
                    let state = aws_credentials_state_for_error(err);
                    let (_, message, _) = state.user_facing_components();
                    (state, Err(message))
                }
            };
            manager.set_aws_credentials_state(new_state, ctx);
            let _ = tx.send(tx_result);
        },
    );
    Box::pin(async move {
        rx.await
            .unwrap_or_else(|_| Err("Credential refresh was interrupted".to_string()))
    })
}

/// Refreshes credentials via OIDC identity token + STS AssumeRoleWithWebIdentity.
fn refresh_aws_credentials_oidc(
    task_id: Option<String>,
    role_arn: String,
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) -> BoxFuture<'static, Result<(), String>> {
    // Skip if credentials are already loaded and have not yet expired.
    if let AwsCredentialsState::Loaded { credentials, .. } = manager.aws_credentials_state() {
        let still_valid = credentials
            .expires_at()
            .and_then(|exp| exp.duration_since(SystemTime::now()).ok())
            .is_some();
        if still_valid {
            log::info!("Bedrock OIDC: credentials still valid, skipping refresh");
            return Box::pin(async { Ok(()) });
        }
    }

    let Some(task_id) = task_id else {
        let message = "AWS Bedrock inference requires an ambient task ID before credentials \
                       can be minted"
            .to_string();
        manager.set_aws_credentials_state(
            AwsCredentialsState::Failed {
                message: message.clone(),
            },
            ctx,
        );
        return Box::pin(async move { Err(message) });
    };

    log::info!("Bedrock OIDC: preparing token mint for task {task_id:?}");
    manager.set_aws_credentials_state(AwsCredentialsState::Refreshing, ctx);
    let token_future = ManagedSecretManager::handle(ctx)
        .as_ref(ctx)
        .issue_task_identity_token(IdentityTokenOptions {
            audience: AWS_BEDROCK_STS_AUDIENCE.to_string(),
            requested_duration: BEDROCK_IDENTITY_TOKEN_DURATION,
            subject_template: vec1!["scoped_principal".to_string()],
        });

    let (tx, rx) = channel();
    let _ = ctx.spawn(
        async move {
            let token = token_future
                .await
                .context("Failed to mint AWS Bedrock task identity token")?;

            let client = sts_client().await;
            let session_name = aws_role_session_name(&task_id);
            let credentials = client
                .assume_role_with_web_identity()
                .role_arn(&role_arn)
                .role_session_name(session_name)
                .web_identity_token(token.token)
                .send()
                .await
                .map_err(|err| {
                    log::error!("Bedrock OIDC: STS AssumeRoleWithWebIdentity SDK error: {err:#?}");
                    // Surface the AWS service error message for a user-friendly error.
                    let detail = err
                        .as_service_error()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| err.to_string());
                    anyhow::anyhow!("STS AssumeRoleWithWebIdentity failed: {detail}")
                })?
                .credentials
                .context("STS response did not include credentials")?;

            anyhow::Ok(AwsCredentials::new(
                credentials.access_key_id().to_string(),
                credentials.secret_access_key().to_string(),
                Some(credentials.session_token().to_string()),
                SystemTime::try_from(*credentials.expiration()).ok(),
            ))
        },
        move |manager, result, ctx| {
            let (new_state, tx_result) = match result {
                Ok(credentials) => {
                    log::info!("Bedrock OIDC: credentials loaded successfully");
                    (
                        AwsCredentialsState::Loaded {
                            credentials,
                            loaded_at: SystemTime::now(),
                        },
                        Ok(()),
                    )
                }
                Err(e) => {
                    log::error!("Bedrock OIDC: failed to load credentials: {e:#}");
                    let message = e.to_string();
                    (
                        AwsCredentialsState::Failed {
                            message: message.clone(),
                        },
                        Err(message),
                    )
                }
            };
            manager.set_aws_credentials_state(new_state, ctx);
            let _ = tx.send(tx_result);
        },
    );
    Box::pin(async move {
        rx.await
            .unwrap_or_else(|_| Err("Credential refresh was interrupted".to_string()))
    })
}

#[cfg(test)]
#[path = "aws_credentials_tests.rs"]
mod tests;
