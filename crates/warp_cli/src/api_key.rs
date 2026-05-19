use crate::{SortOrderArg, date_time::parse_rfc3339, json_filter::JsonOutput};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand, ValueEnum};

/// API key-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ApiKeyCommand {
    /// List active API keys.
    List(ListApiKeysArgs),
    /// Create a new API key.
    Create(CreateApiKeyArgs),
    /// Immediately expire an API key.
    #[command(alias = "delete")]
    Expire(ExpireApiKeyArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ListApiKeysArgs {
    /// Sort field.
    #[arg(long = "sort-by", value_enum, value_name = "FIELD")]
    pub sort_by: Option<ApiKeySortByArg>,

    /// Sort direction.
    #[arg(long = "sort-order", value_enum, value_name = "DIR")]
    pub sort_order: Option<SortOrderArg>,

    /// JSON formatting configuration.
    #[command(flatten)]
    pub json_output: JsonOutput,
}

#[derive(Debug, Clone, Args)]
pub struct CreateApiKeyArgs {
    /// Name of the API key to create.
    pub name: String,

    /// UID of the agent to authenticate as.
    #[arg(long = "agent", value_name = "UID")]
    pub agent_uid: Option<String>,

    #[command(flatten)]
    pub expiration: ApiKeyExpirationArgs,

    /// JSON formatting configuration.
    #[command(flatten)]
    pub json_output: JsonOutput,
}

/// API key expiration arguments. Exactly one expiration decision is required.
#[derive(Debug, Clone, Args)]
#[group(required = true, multiple = false)]
pub struct ApiKeyExpirationArgs {
    /// Expire the API key after this duration, such as "30d", "12h", or "90m".
    #[arg(long = "expires-in", value_name = "DURATION")]
    pub expires_in: Option<humantime::Duration>,

    /// Expire the API key at a specific time.
    #[arg(long = "expires-at", value_name = "RFC3339", value_parser = parse_rfc3339)]
    pub expires_at: Option<DateTime<Utc>>,

    /// Create an API key with no expiration.
    #[arg(long = "no-expiration")]
    pub no_expiration: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ExpireApiKeyArgs {
    /// Name or UID of the API key to expire.
    #[arg(value_name = "NAME_OR_UID")]
    pub key_uid: String,

    /// Expire without asking for confirmation.
    #[arg(long, default_value_t = false)]
    pub force: bool,

    /// JSON formatting configuration.
    #[command(flatten)]
    pub json_output: JsonOutput,
}

/// Sort-by values accepted by `--sort-by`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ApiKeySortByArg {
    #[value(name = "name")]
    Name,
    #[value(name = "created-at")]
    CreatedAt,
    #[value(name = "last-used-at")]
    LastUsedAt,
    #[value(name = "expires-at")]
    ExpiresAt,
    #[value(name = "scope")]
    Scope,
}

#[cfg(test)]
#[path = "api_key_tests.rs"]
mod tests;
