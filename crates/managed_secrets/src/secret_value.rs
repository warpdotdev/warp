use std::fmt;

use serde::Serialize;
use warp_graphql::managed_secrets::ManagedSecretType;

#[derive(Serialize)]
#[serde(untagged)]
pub enum ManagedSecretValue {
    RawValue {
        value: String,
    },
    AnthropicApiKey {
        api_key: String,
    },
    AnthropicBedrockAccessKey {
        aws_access_key_id: String,
        aws_secret_access_key: String,
        /// Optional AWS session token. Only required for temporary/STS credentials;
        /// persistent IAM access keys do not need one. When `None`, the field is
        /// omitted from the serialized JSON payload sent to the server.
        #[serde(skip_serializing_if = "Option::is_none")]
        aws_session_token: Option<String>,
        aws_region: String,
    },
    AnthropicBedrockApiKey {
        aws_bearer_token_bedrock: String,
        aws_region: String,
    },
    OpenaiApiKey {
        api_key: String,
        /// Optional base URL for the OpenAI API (e.g. regional endpoints).
        /// When absent, the harness uses the provider's default endpoint.
        #[serde(skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
}

impl ManagedSecretValue {
    pub fn raw_value(s: impl Into<String>) -> Self {
        Self::RawValue { value: s.into() }
    }

    pub fn anthropic_api_key(s: impl Into<String>) -> Self {
        Self::AnthropicApiKey { api_key: s.into() }
    }

    /// Construct an Anthropic Bedrock access key secret from IAM credentials and AWS region.
    ///
    /// `session_token` is optional and may be `None` for persistent IAM credentials
    /// that do not require a session token.
    pub fn anthropic_bedrock_access_key(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
        region: impl Into<String>,
    ) -> Self {
        Self::AnthropicBedrockAccessKey {
            aws_access_key_id: access_key_id.into(),
            aws_secret_access_key: secret_access_key.into(),
            aws_session_token: session_token,
            aws_region: region.into(),
        }
    }

    /// Construct an Anthropic Bedrock API key secret from a bearer token and AWS region.
    pub fn anthropic_bedrock_api_key(token: impl Into<String>, region: impl Into<String>) -> Self {
        Self::AnthropicBedrockApiKey {
            aws_bearer_token_bedrock: token.into(),
            aws_region: region.into(),
        }
    }

    /// Construct an OpenAI API key secret value with an optional base URL.
    pub fn openai_api_key(api_key: impl Into<String>, base_url: Option<String>) -> Self {
        Self::OpenaiApiKey {
            api_key: api_key.into(),
            base_url,
        }
    }

    pub fn secret_type(&self) -> ManagedSecretType {
        match self {
            ManagedSecretValue::RawValue { .. } => ManagedSecretType::RawValue,
            ManagedSecretValue::AnthropicApiKey { .. } => ManagedSecretType::AnthropicApiKey,
            ManagedSecretValue::AnthropicBedrockAccessKey { .. } => {
                ManagedSecretType::AnthropicBedrockAccessKey
            }
            ManagedSecretValue::AnthropicBedrockApiKey { .. } => {
                ManagedSecretType::AnthropicBedrockApiKey
            }
            ManagedSecretValue::OpenaiApiKey { .. } => ManagedSecretType::OpenaiApiKey,
        }
    }
}

impl fmt::Debug for ManagedSecretValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ManagedSecretValue::RawValue { .. } => f
                .debug_struct("ManagedSecret::RawValue")
                .finish_non_exhaustive(),
            ManagedSecretValue::AnthropicApiKey { .. } => f
                .debug_struct("ManagedSecret::AnthropicApiKey")
                .finish_non_exhaustive(),
            ManagedSecretValue::AnthropicBedrockAccessKey { .. } => f
                .debug_struct("ManagedSecret::AnthropicBedrockAccessKey")
                .finish_non_exhaustive(),
            ManagedSecretValue::AnthropicBedrockApiKey { .. } => f
                .debug_struct("ManagedSecret::AnthropicBedrockApiKey")
                .finish_non_exhaustive(),
            ManagedSecretValue::OpenaiApiKey { .. } => f
                .debug_struct("ManagedSecret::OpenaiApiKey")
                .finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
#[path = "secret_value_tests.rs"]
mod tests;
