use serde::{Deserialize, Serialize};
use warp_core::ui::icons::Icon;
use warp_errors::report_error;

use crate::api_keys::ApiKeys;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LLMProvider {
    OpenAI,
    Anthropic,
    Google,
    Xai,
    Unknown,
}

impl LLMProvider {
    pub const API_KEY_PROVIDERS: [Self; 4] =
        [Self::OpenAI, Self::Anthropic, Self::Google, Self::Xai];
    pub const API_KEY_PROVIDER_VALUE_NAME: &'static str = "openai|anthropic|google|grok";

    pub fn icon(self) -> Option<Icon> {
        match self {
            Self::OpenAI => Some(Icon::OpenAILogo),
            Self::Anthropic => Some(Icon::ClaudeLogo),
            Self::Google => Some(Icon::GeminiLogo),
            Self::Xai | Self::Unknown => None,
        }
    }
    pub fn supports_pasted_api_key(self) -> bool {
        matches!(self, Self::OpenAI | Self::Anthropic | Self::Google)
    }

    pub fn api_key_placeholder(self) -> Option<&'static str> {
        match self {
            Self::OpenAI => Some("sk-..."),
            Self::Anthropic => Some("sk-ant-..."),
            Self::Google => Some("AIzaSy..."),
            Self::Xai | Self::Unknown => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Google => "Google",
            Self::Xai => "xAI",
            Self::Unknown => "this provider",
        }
    }

    pub fn api_key_slug(self) -> Option<&'static str> {
        match self {
            Self::OpenAI => Some("openai"),
            Self::Anthropic => Some("anthropic"),
            Self::Google => Some("google"),
            Self::Xai => Some("grok"),
            Self::Unknown => None,
        }
    }

    pub fn from_api_key_slug(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" | "open-ai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "google" => Ok(Self::Google),
            "grok" | "xai" | "x-ai" => Ok(Self::Xai),
            _ => Err("provider must be one of: anthropic, openai, google, grok".to_owned()),
        }
    }

    pub fn api_key(self, keys: &ApiKeys) -> Option<&str> {
        match self {
            Self::OpenAI => keys.openai.as_deref(),
            Self::Anthropic => keys.anthropic.as_deref(),
            Self::Google => keys.google.as_deref(),
            Self::Xai | Self::Unknown => None,
        }
    }

    pub(crate) fn set_api_key(self, keys: &mut ApiKeys, key: Option<String>) -> bool {
        match self {
            Self::OpenAI => keys.openai = key,
            Self::Anthropic => keys.anthropic = key,
            Self::Google => keys.google = key,
            Self::Xai | Self::Unknown => return false,
        }
        true
    }
}

impl From<warp_graphql::queries::get_feature_model_choices::LlmProvider> for LLMProvider {
    fn from(value: warp_graphql::queries::get_feature_model_choices::LlmProvider) -> Self {
        match value {
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Openai => Self::OpenAI,
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Anthropic => {
                Self::Anthropic
            }
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Google => Self::Google,
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Xai => Self::Xai,
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Unknown => Self::Unknown,
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Other(value) => {
                report_error!(
                    "Invalid LlmProvider; update client GraphQL types",
                    extra: { "provider" => %value },
                    warp_errors::ReportErrorLogMode::OncePerRun
                );
                Self::Unknown
            }
        }
    }
}

impl From<warp_graphql::workspace::LlmProvider> for LLMProvider {
    fn from(value: warp_graphql::workspace::LlmProvider) -> Self {
        match value {
            warp_graphql::workspace::LlmProvider::Openai => Self::OpenAI,
            warp_graphql::workspace::LlmProvider::Anthropic => Self::Anthropic,
            warp_graphql::workspace::LlmProvider::Google => Self::Google,
            warp_graphql::workspace::LlmProvider::Xai => Self::Xai,
            warp_graphql::workspace::LlmProvider::Unknown => Self::Unknown,
            warp_graphql::workspace::LlmProvider::Other(value) => {
                report_error!(
                    "Invalid LlmProvider; update client GraphQL types",
                    extra: { "provider" => %value },
                    warp_errors::ReportErrorLogMode::OncePerRun
                );
                Self::Unknown
            }
        }
    }
}

#[cfg(test)]
#[path = "llm_provider_tests.rs"]
mod tests;
