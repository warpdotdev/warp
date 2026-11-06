//! Configures model and thinking settings through the ACP bridge.
//!
//! Every ACP session selects its model this way, from the settings the adapter advertises at
//! `session/new`; the PTY transport keeps each CLI's own file/environment contract instead.
//! A requested value the adapter does not offer fails the session rather than silently
//! running on the adapter's default.
use anyhow::{Context as _, Result, anyhow};

use super::connection::AcpConnection;
use super::protocol::{
    AcpAdvertisedSetting, AcpSetSettingParams, AcpSetSettingResponse,
    METHOD_SESSION_SET_CONFIG_OPTION, NewSessionResponse,
};
use crate::ai::ambient_agents::task::HarnessModelConfig;

/// Applies requested model and thinking settings, updating the session's advertised settings.
pub(super) async fn configure_model_and_thinking_through_acp(
    connection: &AcpConnection,
    session: &mut NewSessionResponse,
    model_config: Option<&HarnessModelConfig>,
) -> Result<()> {
    let Some(config) = model_config else {
        return Ok(());
    };
    // Changing the model can change which thinking levels the adapter offers.
    for (category, value) in [
        (
            "model",
            Some(config.model_id.as_str()).filter(|id| !id.is_empty() && *id != "default"),
        ),
        (
            "thought_level",
            config
                .reasoning_level
                .as_deref()
                .filter(|level| !level.is_empty()),
        ),
    ] {
        let Some(value) = value else {
            continue;
        };
        let setting_id = find_supported_setting_id(&session.config_options, category, value)?;
        let response: AcpSetSettingResponse = connection
            .request(
                METHOD_SESSION_SET_CONFIG_OPTION,
                AcpSetSettingParams {
                    session_id: session.session_id.clone(),
                    config_id: setting_id.to_owned(),
                    value: value.to_owned(),
                },
            )
            .await
            .context("Failed to configure ACP session")?;
        session.config_options = response.config_options;
    }
    Ok(())
}

/// Finds the adapter's selectable setting ID for a supported category/value pair.
/// Returns an error if no advertised setting accepts the value.
fn find_supported_setting_id<'a>(
    settings: &'a [AcpAdvertisedSetting],
    category: &str,
    value: &str,
) -> Result<&'a str> {
    settings
        .iter()
        .find(|setting| {
            setting.category.as_deref() == Some(category)
                && setting.input_type == "select"
                && setting
                    .choices
                    .iter()
                    .any(|choice| choice.supports_value(value))
        })
        .map(|setting| setting.id.as_str())
        .ok_or_else(|| anyhow!("ACP session does not offer {category} value '{value}'"))
}

#[cfg(test)]
#[path = "model_config_tests.rs"]
mod tests;
