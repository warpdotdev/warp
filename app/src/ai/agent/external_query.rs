//! Display helpers for [`AIAgentInput::ExternalQuery`](super::AIAgentInput::ExternalQuery):
//! the platform, sender, and container labels shared by the GUI, TUI, and conversation search.

use anyhow::{Context, anyhow};
use base64::Engine as _;
use prost::Message as _;
use warp_multi_agent_api::external_message::Platform;
use warp_multi_agent_api::request::input::user_inputs::ExternalQueryToken;
use warp_multi_agent_api::{BodyFormat, ExternalMessage, ExternalQuery};

/// The raw platform message text behind an external query; empty when the server sent no
/// message.
pub fn external_query_body(query: &ExternalQuery) -> &str {
    query
        .message
        .as_ref()
        .map(|message| message.body.as_str())
        .unwrap_or_default()
}

/// Human-readable name of the platform the message was posted on.
pub fn platform_name(message: &ExternalMessage) -> &'static str {
    match &message.platform {
        Some(Platform::Slack(_)) => "Slack",
        Some(Platform::Github(_)) => "GitHub",
        Some(Platform::Gitlab(_)) => "GitLab",
        Some(Platform::Linear(_)) => "Linear",
        Some(Platform::Jira(_)) => "Jira",
        Some(Platform::CustomWebhook(_)) => "Webhook",
        None => "External",
    }
}

/// Label for the platform-specific container the message lives in (`#channel`,
/// `owner/repo#123`, `group/project!7`, an issue key, ...), when the platform provides one.
pub fn container_label(message: &ExternalMessage) -> Option<String> {
    let label = match message.platform.as_ref()? {
        Platform::Slack(slack) => {
            if !slack.channel_name.is_empty() {
                format!("#{}", slack.channel_name)
            } else if !slack.channel_id.is_empty() {
                slack.channel_id.clone()
            } else {
                return None;
            }
        }
        Platform::Github(github) => {
            if github.owner.is_empty() && github.repo.is_empty() {
                return None;
            }
            format!("{}/{}#{}", github.owner, github.repo, github.number)
        }
        Platform::Gitlab(gitlab) => {
            if gitlab.project_path.is_empty() {
                return None;
            }
            format!("{}!{}", gitlab.project_path, gitlab.merge_request_iid)
        }
        Platform::Linear(linear) => linear.issue_identifier.clone(),
        Platform::Jira(jira) => jira.issue_key.clone(),
        Platform::CustomWebhook(_) => return None,
    };
    (!label.is_empty()).then_some(label)
}

/// Best available name for the sender: display name, then handle, then platform id, then the
/// platform name itself.
pub fn sender_display_name(message: &ExternalMessage) -> String {
    message
        .sender
        .as_ref()
        .and_then(|sender| {
            [&sender.display_name, &sender.handle, &sender.id]
                .into_iter()
                .find(|field| !field.is_empty())
                .cloned()
        })
        .unwrap_or_else(|| platform_name(message).to_owned())
}

/// Whether the body should be rendered as markdown. Slack mrkdwn, HTML, and Atlassian Document
/// Format have no client renderer yet and fall back to plain text.
pub fn body_is_markdown(message: &ExternalMessage) -> bool {
    matches!(message.body_format(), BodyFormat::Markdown)
}

/// Decodes the `ExternalQuery` carried by a server-issued `ExternalQueryToken` string
/// (`base64url_nopad(payload) "." base64url_nopad(signature)`). The signature is not checked:
/// the client only needs the payload for local display and echoes the token verbatim for the
/// server to verify.
pub fn decode_external_query_token(token: &str) -> anyhow::Result<ExternalQuery> {
    let payload = token
        .split('.')
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| anyhow!("external query token has no payload segment"))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("external query token payload is not base64url")?;
    let decoded = ExternalQueryToken::decode(bytes.as_slice())
        .context("external query token payload is not a valid ExternalQueryToken")?;
    decoded
        .query
        .ok_or_else(|| anyhow!("external query token carries no query"))
}

#[cfg(test)]
#[path = "external_query_tests.rs"]
mod tests;
