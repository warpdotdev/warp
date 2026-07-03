use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;

use anyhow::Result;
use chrono::{DateTime, Utc};
use vec1::Vec1;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ManagedSecretType {
    AnthropicApiKey,
    AnthropicBedrockAccessKey,
    AnthropicBedrockApiKey,
    Dotenvx,
    RawValue,
}

impl ManagedSecretType {
    /// The identifier for this secret type as used in the client-side upload envelope.
    pub fn envelope_name(&self) -> &str {
        match self {
            ManagedSecretType::AnthropicApiKey => "anthropic_api_key",
            ManagedSecretType::AnthropicBedrockAccessKey => "anthropic_bedrock_access_key",
            ManagedSecretType::AnthropicBedrockApiKey => "anthropic_bedrock_api_key",
            ManagedSecretType::Dotenvx => "dotenvx",
            ManagedSecretType::RawValue => "raw_value",
        }
    }
}

#[derive(Debug)]
pub struct ManagedSecretConfig {
    /// The base64-encoded public key.
    pub public_key: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ManagedSecretOwner {
    CurrentUser,
}

#[derive(Debug, Clone)]
pub struct ManagedSecret {
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub owner: ManagedSecretOwner,
    pub type_: ManagedSecretType,
}

#[derive(Debug)]
pub enum TaskManagedSecretValue {
    RawValue {
        value: String,
    },
    AnthropicApiKey {
        api_key: String,
    },
    AnthropicBedrockAccessKey {
        aws_access_key_id: String,
        aws_secret_access_key: String,
        aws_session_token: Option<String>,
        aws_region: String,
    },
    AnthropicBedrockApiKey {
        aws_bearer_token_bedrock: String,
        aws_region: String,
    },
    Unknown,
}

/// An OIDC identity token issued for a task workload.
#[derive(Debug, Clone)]
pub struct TaskIdentityToken {
    /// The signed OIDC JWT.
    pub token: String,
    /// When the token expires.
    pub expires_at: DateTime<Utc>,
    /// The OIDC issuer that signed the token.
    pub issuer: String,
}

/// Options for issuing an OIDC identity token.
pub struct IdentityTokenOptions {
    /// The intended audience for the token (e.g. a cloud provider URL).
    pub audience: String,
    /// The requested token lifetime. The server may cap this to a maximum value.
    pub requested_duration: Duration,
    /// Controls how the `sub` claim is formatted. Each element names a claim to
    /// include.
    pub subject_template: Vec1<String>,
}

/// Configuration for all managed secret stores accessible to the current user.
#[derive(Debug)]
pub struct ManagedSecretConfigs {
    /// Configuration for the user's personal secrets.
    pub user_secrets: Option<ManagedSecretConfig>,
}

#[derive(Debug, Clone)]
pub enum SecretOwner {
    CurrentUser,
}

#[async_trait]
pub trait ManagedSecretsClient: 'static + Send + Sync {
    async fn get_managed_secret_configs(&self) -> Result<ManagedSecretConfigs>;

    async fn create_managed_secret(
        &self,
        owner: SecretOwner,
        name: String,
        secret_type: ManagedSecretType,
        encrypted_value: String,
        description: Option<String>,
    ) -> Result<ManagedSecret>;

    async fn delete_managed_secret(&self, owner: SecretOwner, name: String) -> Result<()>;

    async fn update_managed_secret(
        &self,
        owner: SecretOwner,
        name: String,
        encrypted_value: Option<String>,
        description: Option<String>,
    ) -> Result<ManagedSecret>;

    async fn list_secrets(&self) -> Result<Vec<ManagedSecret>>;

    async fn get_task_secrets(
        &self,
        task_id: String,
        workload_token: String,
    ) -> Result<HashMap<String, TaskManagedSecretValue>>;

    /// Issue a short-lived OIDC identity token for the current task.
    ///
    /// The workload token is not passed explicitly - it's automatically provided
    /// as part of the client's cloud agent workload identity token support.
    async fn issue_task_identity_token(
        &self,
        options: IdentityTokenOptions,
    ) -> Result<TaskIdentityToken>;
}
