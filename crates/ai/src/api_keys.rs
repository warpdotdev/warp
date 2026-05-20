pub use crate::aws_credentials::{AwsCredentials, AwsCredentialsState};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_multi_agent_api as api;
use warpui::{Entity, ModelContext, SingletonEntity};
use warpui_extras::secure_storage::{self, AppContextExt};

const SECURE_STORAGE_KEY: &str = "AiApiKeys";

/// Emitted when user-provided API keys are updated in-memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyManagerEvent {
    KeysUpdated,
}

/// User-provided API keys for AI providers.
///
/// These are used for "Bring Your Own API Key" functionality, allowing
/// users to use their own API keys instead of Warp's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiKeys {
    pub google: Option<String>,
    pub anthropic: Option<String>,
    pub openai: Option<String>,
    pub open_router: Option<String>,
    pub custom_endpoints: Vec<CustomEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomEndpoint {
    pub name: String,
    pub url: String,
    pub api_key: String,
    pub models: Vec<CustomEndpointModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomEndpointModel {
    pub name: String,
    pub alias: Option<String>,
    /// Stable identifier used as `ModelConfig.{base,coding,cli_agent,computer_use_agent}` and
    /// as the `CustomModelProviders.providers[*].models[*].config_key` on the request wire.
    /// Generated as a UUIDv4 at model creation.
    pub config_key: String,
}

impl CustomEndpointModel {
    /// Picker label: prefer the user-provided alias; fall back to the raw model name
    /// so a row is never blank.
    pub fn display_label(&self) -> &str {
        match self.alias.as_deref() {
            Some(alias) if !alias.trim().is_empty() => alias,
            _ => &self.name,
        }
    }
}

impl ApiKeys {
    pub fn has_any_key(&self) -> bool {
        self.openai.is_some()
            || self.anthropic.is_some()
            || self.google.is_some()
            || self.open_router.is_some()
            || self
                .custom_endpoints
                .iter()
                .any(|endpoint| !endpoint.api_key.trim().is_empty())
    }

    /// Returns `true` when the user has at least one custom endpoint configured.
    pub fn has_custom_endpoints(&self) -> bool {
        !self.custom_endpoints.is_empty()
    }
}

/// Controls how AWS credentials are refreshed by [`ApiKeyManager`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AwsCredentialsRefreshStrategy {
    /// Load credentials from the local AWS credential chain (~/.aws). This is the default.
    #[default]
    LocalChain,
    /// Credentials are managed externally via OIDC/STS.
    /// The task ID is used to scope the STS AssumeRoleWithWebIdentity session.
    /// The role ARN is the IAM role to assume via STS.
    OidcManaged {
        task_id: Option<String>,
        role_arn: String,
    },
}

/// A structure that manages API keys for AI providers.
pub struct ApiKeyManager {
    keys: ApiKeys,
    pub(crate) aws_credentials_state: AwsCredentialsState,
    aws_credentials_refresh_strategy: AwsCredentialsRefreshStrategy,
    secure_storage_write_version: u64,
}

impl ApiKeyManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let keys = Self::load_keys_from_secure_storage(ctx);
        Self {
            keys,
            aws_credentials_state: AwsCredentialsState::Missing,
            aws_credentials_refresh_strategy: AwsCredentialsRefreshStrategy::default(),
            secure_storage_write_version: 0,
        }
    }

    pub fn keys(&self) -> &ApiKeys {
        &self.keys
    }

    pub fn set_google_key(&mut self, key: Option<String>, ctx: &mut ModelContext<Self>) {
        self.keys.google = key;
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
        self.write_keys_to_secure_storage(ctx);
    }

    pub fn set_anthropic_key(&mut self, key: Option<String>, ctx: &mut ModelContext<Self>) {
        self.keys.anthropic = key;
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
        self.write_keys_to_secure_storage(ctx);
    }

    pub fn set_openai_key(&mut self, key: Option<String>, ctx: &mut ModelContext<Self>) {
        self.keys.openai = key;
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
        self.write_keys_to_secure_storage(ctx);
    }

    pub fn set_open_router_key(&mut self, key: Option<String>, ctx: &mut ModelContext<Self>) {
        self.keys.open_router = key;
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
        self.write_keys_to_secure_storage(ctx);
    }

    pub fn add_custom_endpoint(
        &mut self,
        name: String,
        url: String,
        api_key: String,
        models: Vec<(String, Option<String>, Option<String>)>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.keys.custom_endpoints.push(CustomEndpoint {
            name,
            url,
            api_key,
            models: models
                .into_iter()
                .map(|(name, alias, config_key)| CustomEndpointModel {
                    name,
                    alias,
                    config_key: config_key
                        .filter(|k| !k.is_empty())
                        .unwrap_or_else(|| Uuid::new_v4().to_string()),
                })
                .collect(),
        });
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
        self.write_keys_to_secure_storage(ctx);
    }

    pub fn save_custom_endpoint(
        &mut self,
        index: usize,
        name: String,
        url: String,
        api_key: String,
        models: Vec<(String, Option<String>, Option<String>)>,
        ctx: &mut ModelContext<Self>,
    ) {
        if index >= self.keys.custom_endpoints.len() {
            return;
        }
        self.keys.custom_endpoints[index] = CustomEndpoint {
            name,
            url,
            api_key,
            models: models
                .into_iter()
                .map(|(name, alias, config_key)| CustomEndpointModel {
                    name,
                    alias,
                    config_key: config_key
                        .filter(|k| !k.is_empty())
                        .unwrap_or_else(|| Uuid::new_v4().to_string()),
                })
                .collect(),
        };
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
        self.write_keys_to_secure_storage(ctx);
    }

    pub fn remove_custom_endpoint(&mut self, index: usize, ctx: &mut ModelContext<Self>) {
        if index >= self.keys.custom_endpoints.len() {
            return;
        }
        self.keys.custom_endpoints.remove(index);
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
        self.write_keys_to_secure_storage(ctx);
    }

    pub fn clear_custom_endpoints(&mut self, ctx: &mut ModelContext<Self>) {
        if self.keys.custom_endpoints.is_empty() {
            return;
        }
        self.keys.custom_endpoints.clear();
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
        self.write_keys_to_secure_storage(ctx);
    }

    pub fn set_aws_credentials_state(
        &mut self,
        state: AwsCredentialsState,
        ctx: &mut ModelContext<Self>,
    ) {
        self.aws_credentials_state = state;
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
    }

    pub fn aws_credentials_state(&self) -> &AwsCredentialsState {
        &self.aws_credentials_state
    }

    pub fn aws_credentials_refresh_strategy(&self) -> AwsCredentialsRefreshStrategy {
        self.aws_credentials_refresh_strategy.clone()
    }

    pub fn set_aws_credentials_refresh_strategy(
        &mut self,
        strategy: AwsCredentialsRefreshStrategy,
    ) {
        self.aws_credentials_refresh_strategy = strategy;
    }

    /// Builds the `CustomModelProviders` registry that ships with every agent request.
    ///
    /// Emits one [`CustomModelProvider`] per configured [`CustomEndpoint`], each populated with
    /// all of its [`CustomEndpointModel`]s. The per-model `config_key` is what the server uses
    /// to map a `ModelConfig.{base,coding,cli_agent,computer_use_agent}` selection back to a
    /// user-provided endpoint, so it MUST be the same UUID we store locally.
    ///
    /// Returns `None` when custom models should not be included or no endpoint has both a
    /// non-empty URL and API key.
    pub fn custom_model_providers_for_request(
        &self,
        include_custom_models: bool,
    ) -> Option<api::request::settings::CustomModelProviders> {
        if !include_custom_models {
            return None;
        }

        let providers: Vec<_> = self
            .keys
            .custom_endpoints
            .iter()
            .filter(|endpoint| !endpoint.url.trim().is_empty() && !endpoint.api_key.is_empty())
            .map(
                |endpoint| api::request::settings::custom_model_providers::CustomModelProvider {
                    base_url: endpoint.url.clone(),
                    api_key: endpoint.api_key.clone(),
                    models: endpoint
                        .models
                        .iter()
                        .filter(|m| !m.name.trim().is_empty() && !m.config_key.is_empty())
                        .map(
                            |m| api::request::settings::custom_model_providers::CustomModel {
                                slug: m.name.clone(),
                                config_key: m.config_key.clone(),
                            },
                        )
                        .collect(),
                },
            )
            .filter(|provider| !provider.models.is_empty())
            .collect();

        if providers.is_empty() {
            None
        } else {
            Some(api::request::settings::CustomModelProviders { providers })
        }
    }

    pub fn api_keys_for_request(
        &self,
        include_byo_keys: bool,
        include_aws_bedrock_credentials: bool,
    ) -> Option<api::request::settings::ApiKeys> {
        let anthropic = include_byo_keys
            .then(|| self.keys.anthropic.clone())
            .flatten()
            .unwrap_or_default();
        let openai = include_byo_keys
            .then(|| self.keys.openai.clone())
            .flatten()
            .unwrap_or_default();
        let google = include_byo_keys
            .then(|| self.keys.google.clone())
            .flatten()
            .unwrap_or_default();
        let open_router = include_byo_keys
            .then(|| self.keys.open_router.clone())
            .flatten()
            .unwrap_or_default();

        // Also include credentials when running with OIDC-managed Bedrock inference, regardless
        // of the per-user setting flag (which only applies to the local credential chain path).
        let include_aws = include_aws_bedrock_credentials
            || matches!(
                self.aws_credentials_refresh_strategy,
                AwsCredentialsRefreshStrategy::OidcManaged { .. }
            );
        let aws_credentials = include_aws
            .then(|| match self.aws_credentials_state {
                AwsCredentialsState::Loaded {
                    ref credentials, ..
                } => Some(credentials.clone().into()),
                _ => None,
            })
            .flatten();

        if anthropic.is_empty()
            && openai.is_empty()
            && google.is_empty()
            && open_router.is_empty()
            && aws_credentials.is_none()
        {
            None
        } else {
            Some(api::request::settings::ApiKeys {
                anthropic,
                openai,
                google,
                open_router,
                allow_use_of_warp_credits: false,
                aws_credentials,
            })
        }
    }

    fn load_keys_from_secure_storage(ctx: &mut ModelContext<Self>) -> ApiKeys {
        let key_json = match ctx.secure_storage().read_value(SECURE_STORAGE_KEY) {
            Ok(json) => json,
            Err(e) => {
                if !matches!(e, secure_storage::Error::NotFound) {
                    log::error!("Failed to read API keys from secure storage: {e:#}");
                }
                return ApiKeys::default();
            }
        };

        match serde_json::from_str(&key_json) {
            Ok(keys) => keys,
            Err(e) => {
                log::error!("Failed to deserialize API keys: {e:#}");
                ApiKeys::default()
            }
        }
    }

    fn write_keys_to_secure_storage(&mut self, ctx: &mut ModelContext<Self>) {
        let json = match serde_json::to_string(&self.keys) {
            Ok(json) => json,
            Err(e) => {
                log::error!("Failed to serialize API keys: {e:#}");
                return;
            }
        };
        self.secure_storage_write_version += 1;
        let write_version = self.secure_storage_write_version;

        // Defer the keychain write so it doesn't block the current event
        // processing. The in-memory state is already updated and events
        // already emitted, so the UI updates immediately while the
        // potentially slow platform secure-storage call runs in a
        // subsequent main-thread callback. Skip stale callbacks so older
        // writes cannot complete after and overwrite a newer payload.
        ctx.spawn(async move { json }, move |me, json, ctx| {
            if write_version != me.secure_storage_write_version {
                return;
            }
            if let Err(e) = ctx.secure_storage().write_value(SECURE_STORAGE_KEY, &json) {
                log::error!("Failed to write API keys to secure storage: {e:#}");
            }
        });
    }
}

impl Entity for ApiKeyManager {
    type Event = ApiKeyManagerEvent;
}

impl SingletonEntity for ApiKeyManager {}

#[cfg(test)]
#[path = "api_keys_tests.rs"]
mod tests;
