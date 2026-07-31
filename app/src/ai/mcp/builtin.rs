//! Built-in Warp-hosted MCP servers.
//!
//! Built-in servers are attached automatically for logged-in users: their
//! definitions are constructed in code and authenticated with the user's
//! existing session credentials (warp-server accepts both session ID tokens
//! and API keys as `Bearer` credentials), so they require no MCP
//! configuration and no manually minted API key.
//!
//! Lifecycle is owned by [`TemplatableMCPServerManager::sync_builtin_servers`]
//! (attach on login, re-attach on token rotation, detach on logout), gated by
//! [`warp_core::features::FeatureFlag::FactoryMcp`].
//!
//! [`TemplatableMCPServerManager::sync_builtin_servers`]: super::TemplatableMCPServerManager::sync_builtin_servers

use std::collections::HashMap;

use uuid::Uuid;
use warp_core::channel::ChannelState;

use super::templatable::{JsonTemplate, TemplatableMCPServer};
use super::templatable_installation::TemplatableMCPServerInstallation;
use crate::auth::credentials::Credentials;

/// Stable installation UUID for the built-in Factory MCP server, so lifecycle
/// state, request grouping, and respawns are keyed consistently.
pub const FACTORY_MCP_INSTALLATION_UUID: Uuid =
    Uuid::from_u128(0xfac70a11_a55e_4bde_9c3a_1c0ffee0f001);

/// Stable template UUID for the built-in Factory MCP server. Log files are
/// keyed by template UUID, so keeping it constant keeps one log per server
/// across respawns.
const FACTORY_MCP_TEMPLATE_UUID: Uuid = Uuid::from_u128(0xfac70a11_a55e_4bde_9c3a_1c0ffee0f002);

/// The server name under which the Factory MCP's tools are grouped.
pub const FACTORY_MCP_SERVER_NAME: &str = "warp-factory";

/// Returns the bearer token built-in servers should authenticate with, or
/// `None` when the current credentials cannot be used for one.
///
/// A Firebase token that expires within the next couple of minutes is treated
/// as unusable: spawning with it would race expiry, and a 401 on the
/// connection preflight would misroute the built-in server into the
/// interactive MCP OAuth flow. The app's request layer refreshes tokens
/// within a five-minute window (see `AuthSession::get_or_refresh_access_token`),
/// and the manager respawns on the resulting `AccessTokenRefreshed` event.
pub fn builtin_bearer_token(credentials: &Credentials) -> Option<String> {
    if let Some(tokens) = credentials.as_firebase() {
        let min_validity = chrono::Duration::minutes(2);
        if chrono::Local::now().fixed_offset() + min_validity >= tokens.expiration_time {
            return None;
        }
    }
    credentials.bearer_token().bearer_token()
}

/// Builds the ephemeral installation for the built-in Factory MCP server: a
/// streamable-HTTP MCP server hosted by warp-server at `/api/v1/mcp/factory`,
/// pre-authenticated via the `Authorization` header.
pub fn factory_mcp_installation(bearer_token: &str) -> TemplatableMCPServerInstallation {
    factory_mcp_installation_for_server_root(&ChannelState::server_root_url(), bearer_token)
}

/// Like [`factory_mcp_installation`], with an explicit server root for tests.
fn factory_mcp_installation_for_server_root(
    server_root: &str,
    bearer_token: &str,
) -> TemplatableMCPServerInstallation {
    let server_config = serde_json::json!({
        "url": factory_mcp_url(server_root),
        "headers": {
            "Authorization": format!("Bearer {bearer_token}"),
        },
    });
    let mut root = serde_json::Map::new();
    root.insert(FACTORY_MCP_SERVER_NAME.to_string(), server_config);
    let template_json = serde_json::Value::Object(root).to_string();

    let templatable_mcp_server = TemplatableMCPServer {
        uuid: FACTORY_MCP_TEMPLATE_UUID,
        name: FACTORY_MCP_SERVER_NAME.to_string(),
        description: Some(
            "Warp's hosted Factory MCP server. Work with your team's software factories: \
             list factories and their tasks, inspect a task's status and outputs, and send \
             work in or hand it back."
                .to_string(),
        ),
        template: JsonTemplate {
            json: template_json,
            // Fully resolved: the token is baked into the header, so there
            // are no variables to prompt for.
            variables: Vec::new(),
        },
        // Constant version: the definition is code-managed, not user-editable.
        version: 0,
        gallery_data: None,
    };

    TemplatableMCPServerInstallation::new(
        FACTORY_MCP_INSTALLATION_UUID,
        templatable_mcp_server,
        HashMap::new(),
    )
}

/// Joins the Factory MCP endpoint path onto a server root URL.
fn factory_mcp_url(server_root: &str) -> String {
    format!("{}/api/v1/mcp/factory", server_root.trim_end_matches('/'))
}

#[cfg(test)]
#[path = "builtin_tests.rs"]
mod tests;
