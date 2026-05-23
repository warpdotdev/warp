use std::fmt;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use crate::scope::ObjectScope;

/// Secret-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum SecretCommand {
    /// Create a new secret.
    ///
    /// Use `oz secret create claude api-key <NAME>` to create a Claude/Anthropic auth secret,
    /// or `oz secret create codex api-key <NAME>` to create a Codex/OpenAI auth secret.
    Create(CreateSecretArgs),
    /// Delete a secret.
    Delete(DeleteSecretArgs),
    /// Update a secret.
    ///
    /// This command supports changing the value (via the `--value` or `--value-file` flags) or the description.
    /// Moving or renaming secrets is not currently supported.
    Update(UpdateSecretArgs),
    /// List secrets.
    List(ListSecretsArgs),
}

#[derive(Debug, Clone, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct CreateSecretArgs {
    /// Provider-specific creation subcommand.
    #[command(subcommand)]
    pub provider: Option<CreateProvider>,

    // --- Fields below are only used when no subcommand is given (generic create). ---
    /// Name of the secret to create.
    pub name: Option<String>,

    #[arg(long = "type", short = 't', default_value_t = Default::default())]
    pub secret_type: SecretType,

    #[clap(flatten)]
    pub value: ValueArgs,

    /// Description of the secret.
    #[arg(long = "description", short = 'd')]
    pub description: Option<String>,

    #[clap(flatten)]
    pub scope: ObjectScope,
}

/// Provider-specific secret creation subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum CreateProvider {
    /// Create a Claude/Anthropic auth secret.
    #[command(name = "claude")]
    Anthropic(AnthropicCreateArgs),
    /// Create a Codex/OpenAI auth secret.
    Codex(CodexCreateArgs),
}

#[derive(Debug, Clone, Args)]
pub struct AnthropicCreateArgs {
    #[command(subcommand)]
    pub method: AnthropicMethod,
}

/// Anthropic credential type.
#[derive(Debug, Clone, Subcommand)]
pub enum AnthropicMethod {
    /// Direct Anthropic API key.
    #[command(name = "api-key")]
    ApiKey(AnthropicApiKeyArgs),
    /// Anthropic API key via Amazon Bedrock.
    #[command(name = "bedrock-api-key")]
    BedrockApiKey(BedrockApiKeyArgs),
    /// Anthropic Bedrock authentication via AWS access keys.
    #[command(name = "bedrock-access-key")]
    BedrockAccessKey(BedrockAccessKeyArgs),
}

/// Fields shared by all provider-specific secret creation subcommands.
#[derive(Debug, Clone, Args)]
pub struct CommonSecretCreateArgs {
    /// Name of the secret.
    pub name: String,

    /// Description of the secret.
    #[arg(long = "description", short = 'd')]
    pub description: Option<String>,

    #[clap(flatten)]
    pub scope: ObjectScope,
}

/// Arguments for creating an Anthropic API key secret.
#[derive(Debug, Clone, Args)]
pub struct AnthropicApiKeyArgs {
    #[clap(flatten)]
    pub common: CommonSecretCreateArgs,

    #[clap(flatten)]
    pub value: ValueArgs,
}

/// Arguments for creating an Anthropic Bedrock API key secret.
#[derive(Debug, Clone, Args)]
pub struct BedrockApiKeyArgs {
    #[clap(flatten)]
    pub common: CommonSecretCreateArgs,

    /// Bedrock API key. If not provided, prompts interactively.
    #[arg(long = "bedrock-api-key")]
    pub bedrock_api_key: Option<String>,

    /// AWS region for the Bedrock endpoint. If not provided, prompts interactively.
    #[arg(long = "region")]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct CodexCreateArgs {
    #[command(subcommand)]
    pub method: CodexMethod,
}

/// Codex credential type.
#[derive(Debug, Clone, Subcommand)]
pub enum CodexMethod {
    /// Direct OpenAI API key.
    #[command(name = "api-key")]
    ApiKey(OpenAiApiKeyArgs),
}

/// Arguments for creating an OpenAI API key secret used by the Codex harness.
#[derive(Debug, Clone, Args)]
pub struct OpenAiApiKeyArgs {
    #[clap(flatten)]
    pub common: CommonSecretCreateArgs,

    #[clap(flatten)]
    pub value: ValueArgs,

    /// Optional base URL for the OpenAI API (e.g. a regional endpoint like
    /// `https://us.api.openai.com/v1`). When omitted in interactive mode the
    /// CLI prompts for it; pressing Enter at the prompt skips it. When omitted
    /// in non-interactive mode the harness uses the provider's default endpoint.
    #[arg(long = "base-url")]
    pub base_url: Option<String>,
}

/// Arguments for creating an Anthropic Bedrock access key secret.
#[derive(Debug, Clone, Args)]
pub struct BedrockAccessKeyArgs {
    #[clap(flatten)]
    pub common: CommonSecretCreateArgs,

    /// AWS access key ID. If not provided, prompts interactively.
    #[arg(long = "access-key-id")]
    pub access_key_id: Option<String>,

    /// AWS secret access key. If not provided, prompts interactively.
    #[arg(long = "secret-access-key")]
    pub secret_access_key: Option<String>,

    /// AWS session token. If not provided, prompts interactively.
    #[arg(long = "session-token")]
    pub session_token: Option<String>,

    /// AWS region for the Bedrock endpoint. If not provided, prompts interactively.
    #[arg(long = "region")]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteSecretArgs {
    /// Name of the secret to delete.
    pub name: String,

    /// Delete without asking for confirmation.
    #[arg(long, default_value_t = false)]
    pub force: bool,

    #[clap(flatten)]
    pub scope: ObjectScope,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateSecretArgs {
    /// Name of the secret to update.
    pub name: String,

    /// Prompt for a new value for the secret.
    #[arg(long = "value", conflicts_with = "value_file")]
    pub value: bool,

    #[clap(flatten)]
    pub value_args: ValueArgs,
    /// New description for the secret. If omitted, the description is not changed.
    #[arg(long = "description", short = 'd')]
    pub description: Option<String>,

    #[clap(flatten)]
    pub scope: ObjectScope,
}

#[derive(Debug, Clone, Args)]
pub struct ListSecretsArgs {
    // TODO: consider flags to filter secrets.
}

#[derive(Debug, Clone, Args)]
pub struct ValueArgs {
    /// File to read the secret value from. If not provided, the secret value will be read from
    /// standard input.
    #[arg(long = "value-file", short = 'f')]
    pub value_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
#[value(rename_all = "kebab-case")]
pub enum SecretType {
    #[default]
    RawValue,
    AnthropicApiKey,
    // Not exposed via the CLI `--type` flag; constructed internally for provider subcommands.
    #[value(skip)]
    AnthropicBedrockApiKey,
    // Not exposed via the CLI `--type` flag; constructed internally for provider subcommands.
    #[value(skip)]
    OpenaiApiKey,
}

impl fmt::Display for SecretType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretType::RawValue => write!(f, "raw-value"),
            SecretType::AnthropicApiKey => write!(f, "anthropic-api-key"),
            SecretType::AnthropicBedrockApiKey => write!(f, "anthropic-bedrock-api-key"),
            SecretType::OpenaiApiKey => write!(f, "openai-api-key"),
        }
    }
}
