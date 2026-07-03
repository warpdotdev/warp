pub use crate::aws_credentials::{AwsCredentials, AwsCredentialsState};
use serde::{Deserialize, Serialize};
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
pub struct ApiKeys {
    pub google: Option<String>,
    pub anthropic: Option<String>,
    pub openai: Option<String>,
    pub open_router: Option<String>,
    pub open_router_model: Option<String>,
}

impl ApiKeys {
    pub fn has_any_key(&self) -> bool {
        self.openai.is_some()
            || self.anthropic.is_some()
            || self.google.is_some()
            || self.open_router.is_some()
    }

    fn has_any_configured_value(&self) -> bool {
        self.has_any_key() || self.open_router_model.is_some()
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
    keys_loaded_from_secure_storage: bool,
    pub(crate) aws_credentials_state: AwsCredentialsState,
    aws_credentials_refresh_strategy: AwsCredentialsRefreshStrategy,
}

impl ApiKeyManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let should_load_secure_storage = Self::secure_storage_presence_marker_exists();
        let keys = if should_load_secure_storage {
            Self::load_keys_from_secure_storage(ctx)
        } else {
            Self::load_keys_from_environment(ApiKeys::default())
        };
        Self {
            keys,
            keys_loaded_from_secure_storage: should_load_secure_storage,
            aws_credentials_state: AwsCredentialsState::Missing,
            aws_credentials_refresh_strategy: AwsCredentialsRefreshStrategy::default(),
        }
    }

    pub fn keys(&self) -> &ApiKeys {
        &self.keys
    }

    pub fn load_keys_from_secure_storage_if_needed(&mut self, ctx: &mut ModelContext<Self>) {
        if self.keys_loaded_from_secure_storage || !Self::secure_storage_presence_marker_exists() {
            return;
        }

        self.keys = Self::load_keys_from_secure_storage(ctx);
        self.keys_loaded_from_secure_storage = true;
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
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

    pub fn set_open_router_model(&mut self, model: Option<String>, ctx: &mut ModelContext<Self>) {
        self.keys.open_router_model = model;
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
        let keys = match ctx.secure_storage().read_value(SECURE_STORAGE_KEY) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(keys) => keys,
                Err(e) => {
                    log::error!("Failed to deserialize API keys: {e:#}");
                    ApiKeys::default()
                }
            },
            Err(e) => {
                if !matches!(e, secure_storage::Error::NotFound) {
                    log::error!("Failed to read API keys from secure storage: {e:#}");
                }
                ApiKeys::default()
            }
        };

        Self::load_keys_from_environment(keys)
    }

    fn load_keys_from_environment(mut keys: ApiKeys) -> ApiKeys {
        if keys.open_router.is_none() {
            keys.open_router = std::env::var("OPENROUTER_API_KEY")
                .or_else(|_| std::env::var("OPEN_ROUTER_API_KEY"))
                .ok()
                .filter(|key| !key.is_empty());
        }

        if keys.open_router_model.is_none() {
            keys.open_router_model = std::env::var("OPENROUTER_MODEL")
                .or_else(|_| std::env::var("OPEN_ROUTER_MODEL"))
                .ok()
                .filter(|model| !model.is_empty());
        }

        keys
    }

    fn write_keys_to_secure_storage(&mut self, ctx: &mut ModelContext<Self>) {
        let keys = self.keys.clone();

        let json = match serde_json::to_string(&keys) {
            Ok(json) => json,
            Err(e) => {
                log::error!("Failed to serialize API keys: {e:#}");
                return;
            }
        };

        if let Err(e) = ctx.secure_storage().write_value(SECURE_STORAGE_KEY, &json) {
            log::error!("Failed to write API keys to secure storage: {e:#}");
            return;
        }

        self.keys_loaded_from_secure_storage = true;
        Self::write_secure_storage_presence_marker(self.keys.has_any_configured_value());
    }
    fn secure_storage_presence_marker_path() -> std::path::PathBuf {
        warp_core::paths::state_dir().join("ai-api-keys-present")
    }
    fn secure_storage_presence_marker_exists() -> bool {
        Self::secure_storage_presence_marker_path().is_file()
    }
    fn write_secure_storage_presence_marker(has_configured_value: bool) {
        let path = Self::secure_storage_presence_marker_path();
        if has_configured_value {
            if let Some(parent) = path.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    log::error!("Failed to create API key marker directory: {error:#}");
                    return;
                }
            }
            if let Err(error) = std::fs::write(&path, b"1") {
                log::error!("Failed to write API key marker: {error:#}");
            }
        } else if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::error!("Failed to remove API key marker: {error:#}");
            }
        }
    }
}

impl Entity for ApiKeyManager {
    type Event = ApiKeyManagerEvent;
}

impl SingletonEntity for ApiKeyManager {}
