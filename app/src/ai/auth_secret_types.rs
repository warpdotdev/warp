use anyhow::{Result, anyhow};
use warp_cli::agent::Harness;
use warp_graphql::managed_secrets::ManagedSecretType;
use warp_managed_secrets::ManagedSecretValue;

pub struct AuthSecretTypeField {
    pub label: &'static str,
    /// Placeholder text shown inside the input editor. Falls back to `label` if `None`.
    pub placeholder: Option<&'static str>,
    pub optional: bool,
    /// Whether the field contains sensitive data and should be password-masked.
    pub sensitive: bool,
}

pub struct AuthSecretTypeInfo {
    pub display_name: &'static str,
    pub secret_type: ManagedSecretType,
    pub fields: &'static [AuthSecretTypeField],
    pub learn_more_url: &'static str,
}

pub fn auth_secret_types_for_harness(harness: Harness) -> &'static [AuthSecretTypeInfo] {
    match harness {
        Harness::Claude => &CLAUDE_AUTH_SECRET_TYPES,
        Harness::Codex => &CODEX_AUTH_SECRET_TYPES,
        _ => &[],
    }
}

pub fn learn_more_url_for_harness(harness: Harness) -> &'static str {
    match harness {
        Harness::Claude => CLAUDE_LEARN_MORE_URL,
        Harness::Codex => CODEX_LEARN_MORE_URL,
        _ => DEFAULT_LEARN_MORE_URL,
    }
}

const DEFAULT_LEARN_MORE_URL: &str =
    "https://docs.warp.dev/agent-platform/cloud-agents/harnesses/authentication/";
const CODEX_LEARN_MORE_URL: &str = "https://docs.warp.dev/agent-platform/cloud-agents/harnesses/authentication/#connecting-codex-credentials";
const CLAUDE_LEARN_MORE_URL: &str = "https://docs.warp.dev/agent-platform/cloud-agents/harnesses/authentication/#connecting-claude-code-credentials";

pub fn build_managed_secret_value(
    info: &AuthSecretTypeInfo,
    field_values: &[String],
) -> Result<ManagedSecretValue> {
    if field_values.len() != info.fields.len() {
        return Err(anyhow!(
            "Expected {} field values, got {}",
            info.fields.len(),
            field_values.len()
        ));
    }
    for (field, value) in info.fields.iter().zip(field_values.iter()) {
        if !field.optional && value.trim().is_empty() {
            return Err(anyhow!("Field '{}' is required", field.label));
        }
    }
    match info.secret_type {
        ManagedSecretType::AnthropicApiKey => Ok(ManagedSecretValue::anthropic_api_key(
            field_values[0].clone(),
        )),
        ManagedSecretType::AnthropicBedrockApiKey => {
            Ok(ManagedSecretValue::anthropic_bedrock_api_key(
                field_values[0].clone(),
                field_values[1].clone(),
            ))
        }
        ManagedSecretType::AnthropicBedrockAccessKey => {
            let session_token = if field_values[2].trim().is_empty() {
                None
            } else {
                Some(field_values[2].clone())
            };
            Ok(ManagedSecretValue::anthropic_bedrock_access_key(
                field_values[0].clone(),
                field_values[1].clone(),
                session_token,
                field_values[3].clone(),
            ))
        }
        ManagedSecretType::OpenaiApiKey => {
            let base_url = field_values
                .get(1)
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            Ok(ManagedSecretValue::openai_api_key(
                field_values[0].clone(),
                base_url,
            ))
        }
        ManagedSecretType::RawValue | ManagedSecretType::Dotenvx => Err(anyhow!(
            "Auth secret type {:?} is not supported via the harness FTUX flow",
            info.secret_type
        )),
    }
}

static CODEX_AUTH_SECRET_TYPES: [AuthSecretTypeInfo; 1] = [AuthSecretTypeInfo {
    display_name: "OpenAI API Key",
    secret_type: ManagedSecretType::OpenaiApiKey,
    learn_more_url: CODEX_LEARN_MORE_URL,
    fields: &[
        AuthSecretTypeField {
            label: "OPENAI_API_KEY",
            placeholder: Some("sk-..."),
            optional: false,
            sensitive: true,
        },
        AuthSecretTypeField {
            label: "BASE_URL",
            placeholder: Some("https://us.api.openai.com/v1"),
            optional: true,
            sensitive: false,
        },
    ],
}];

static CLAUDE_AUTH_SECRET_TYPES: [AuthSecretTypeInfo; 3] = [
    AuthSecretTypeInfo {
        display_name: "Anthropic API Key",
        secret_type: ManagedSecretType::AnthropicApiKey,
        learn_more_url: CLAUDE_LEARN_MORE_URL,
        fields: &[AuthSecretTypeField {
            label: "ANTHROPIC_API_KEY",
            placeholder: Some("sk-ant-..."),
            optional: false,
            sensitive: true,
        }],
    },
    AuthSecretTypeInfo {
        display_name: "Bedrock API Key",
        secret_type: ManagedSecretType::AnthropicBedrockApiKey,
        learn_more_url: CLAUDE_LEARN_MORE_URL,
        fields: &[
            AuthSecretTypeField {
                label: "AWS_BEARER_TOKEN_BEDROCK",
                placeholder: Some("Bearer token"),
                optional: false,
                sensitive: true,
            },
            AuthSecretTypeField {
                label: "AWS_REGION",
                placeholder: Some("us-east-1"),
                optional: false,
                sensitive: false,
            },
        ],
    },
    AuthSecretTypeInfo {
        display_name: "Bedrock Access Key",
        secret_type: ManagedSecretType::AnthropicBedrockAccessKey,
        learn_more_url: CLAUDE_LEARN_MORE_URL,
        fields: &[
            AuthSecretTypeField {
                label: "AWS_ACCESS_KEY_ID",
                placeholder: Some("AKIA..."),
                optional: false,
                sensitive: true,
            },
            AuthSecretTypeField {
                label: "AWS_SECRET_ACCESS_KEY",
                placeholder: Some("Secret access key"),
                optional: false,
                sensitive: true,
            },
            AuthSecretTypeField {
                label: "AWS_SESSION_TOKEN",
                placeholder: Some("Session token (temporary credentials only)"),
                optional: true,
                sensitive: true,
            },
            AuthSecretTypeField {
                label: "AWS_REGION",
                placeholder: Some("us-east-1"),
                optional: false,
                sensitive: false,
            },
        ],
    },
];
