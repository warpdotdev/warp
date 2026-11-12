use std::collections::HashMap;
use std::ffi::OsString;

use anyhow::{Result, anyhow};
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_managed_secrets::ManagedSecretValue;

use crate::ai::ambient_agents::task::HarnessModelConfig;

pub(super) fn resolve(
    harness: Harness,
    secrets: &HashMap<String, ManagedSecretValue>,
    model_config: &mut Option<HarnessModelConfig>,
) -> Result<HashMap<OsString, OsString>> {
    let mut env = HashMap::new();
    if harness != Harness::Claude {
        return Ok(env);
    }
    let Some(config) = model_config.as_mut() else {
        return Ok(env);
    };
    if Uuid::parse_str(&config.model_id).is_err() {
        return Ok(env);
    }
    let raw = |name: &str| match secrets.get(name) {
        Some(ManagedSecretValue::RawValue { value }) if !value.is_empty() => Some(value.as_str()),
        _ => None,
    };
    let model = raw("WARP_CLAUDE_BYOE_MODEL")
        .ok_or_else(|| anyhow!("Missing custom Claude model credentials"))?;
    let base_url =
        raw("ANTHROPIC_BASE_URL").ok_or_else(|| anyhow!("Missing custom Claude endpoint"))?;
    let api_key =
        raw("ANTHROPIC_API_KEY").ok_or_else(|| anyhow!("Missing custom Claude credential"))?;
    config.model_id = model.to_owned();
    for (name, value) in [
        ("WARP_CLAUDE_BYOE_MODEL", model),
        ("ANTHROPIC_BASE_URL", base_url),
        ("ANTHROPIC_API_KEY", api_key),
        ("ANTHROPIC_MODEL", model),
        ("ANTHROPIC_CUSTOM_MODEL_OPTION", model),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL", model),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL", model),
        ("ANTHROPIC_DEFAULT_OPUS_MODEL", model),
        ("ANTHROPIC_AUTH_TOKEN", ""),
        ("CLAUDE_CODE_OAUTH_TOKEN", ""),
        ("CLAUDE_CODE_USE_BEDROCK", ""),
        ("CLAUDE_CODE_USE_VERTEX", ""),
        ("CLAUDE_CODE_USE_FOUNDRY", ""),
    ] {
        env.insert(OsString::from(name), OsString::from(value));
    }
    Ok(env)
}

#[cfg(test)]
#[path = "claude_byo_tests.rs"]
mod tests;
