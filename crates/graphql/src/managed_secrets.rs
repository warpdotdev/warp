use crate::object::Space;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::Enum, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ManagedSecretType {
    AnthropicApiKey,
    AnthropicBedrockAccessKey,
    AnthropicBedrockApiKey,
    Dotenvx,
    OpenaiApiKey,
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
            ManagedSecretType::OpenaiApiKey => "openai_api_key",
            ManagedSecretType::RawValue => "raw_value",
        }
    }
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ManagedSecretConfig {
    /// The base64-encoded public key.
    pub public_key: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ManagedSecret {
    pub name: String,
    pub description: Option<String>,
    pub created_at: Time,
    pub updated_at: Time,
    // In our GraphQL schema, `Space` is essentially the output type equivalent of an `Owner`.
    // Most Warp Drive code converts `Space` to `Owner`, but we don't have that conversion layer
    // for secrets.
    pub owner: Space,
    #[cynic(rename = "type")]
    pub type_: ManagedSecretType,
}
