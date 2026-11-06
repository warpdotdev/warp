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
//! CLI agent runs (`oz agent run`, including the cloud workers behind
//! `oz agent run-cloud`) bypass that manager path; the agent driver attaches
//! the same installation per-run instead (see
//! `AgentDriver::resolve_mcp_specs_to_json`), with the token pinned for the
//! duration of the run.
//!
//! The preview URLs server is only useful inside a cloud run, so only the agent
//! driver attaches it, gated by [`warp_core::features::FeatureFlag::PreviewUrlsMcp`].
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

/// The server name under which the Factory MCP's tools are grouped.
pub const FACTORY_MCP_SERVER_NAME: &str = "warp-factory";

/// The server name under which the preview URLs MCP's tools are grouped.
pub const PREVIEW_URLS_MCP_SERVER_NAME: &str = "warp-preview-urls";

/// Code-owned definition of a streamable-HTTP MCP server hosted by warp-server.
struct BuiltinMcpServer {
    name: &'static str,
    /// Endpoint path under the warp-server root.
    path: &'static str,
    description: &'static str,
    /// Stable installation UUID, so lifecycle state, request grouping, and
    /// respawns are keyed consistently.
    installation_uuid: Uuid,
    /// Stable template UUID. Log files are keyed by template UUID, so keeping
    /// it constant keeps one log per server across respawns.
    template_uuid: Uuid,
}

const FACTORY_MCP: BuiltinMcpServer = BuiltinMcpServer {
    name: FACTORY_MCP_SERVER_NAME,
    path: "/api/v1/mcp/factory",
    description: "Warp's hosted Factory MCP server. Work with your team's software factories: \
                  list factories and their tasks, inspect a task's status and outputs, and send \
                  work in or hand it back.",
    installation_uuid: FACTORY_MCP_INSTALLATION_UUID,
    template_uuid: Uuid::from_u128(0xfac70a11_a55e_4bde_9c3a_1c0ffee0f002),
};

const PREVIEW_URLS_MCP: BuiltinMcpServer = BuiltinMcpServer {
    name: PREVIEW_URLS_MCP_SERVER_NAME,
    path: "/api/v1/mcp/preview-urls",
    description: "Warp's hosted preview URLs MCP server. Expose a port from this cloud agent \
                  run's sandbox as a shareable preview URL.",
    installation_uuid: Uuid::from_u128(0xb6b375a8_1ba5_4a51_a407_22563d4f7e81),
    template_uuid: Uuid::from_u128(0xb6b375a8_1ba5_4a51_a407_22563d4f7e82),
};

impl BuiltinMcpServer {
    /// Joins the endpoint path onto a server root URL.
    fn url(&self, server_root: &str) -> String {
        format!("{}{}", server_root.trim_end_matches('/'), self.path)
    }
}

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
///
/// `ambient_headers` (workload token, cloud-agent ID) are attached alongside it
/// when this run has an active ambient task, so warp-server can verify the
/// caller is that task's own worker rather than soft-failing the check on a
/// missing token.
pub fn factory_mcp_installation(
    bearer_token: &str,
    ambient_headers: &[(String, String)],
) -> TemplatableMCPServerInstallation {
    builtin_mcp_installation(
        &FACTORY_MCP,
        &ChannelState::server_root_url(),
        bearer_token,
        ambient_headers,
    )
}

/// Like [`factory_mcp_installation`], for the preview URLs MCP server at
/// `/api/v1/mcp/preview-urls`. warp-server binds its calls to this run's
/// execution through the ambient workload token, so it is only useful for
/// cloud runs.
pub fn preview_urls_mcp_installation(
    bearer_token: &str,
    ambient_headers: &[(String, String)],
) -> TemplatableMCPServerInstallation {
    builtin_mcp_installation(
        &PREVIEW_URLS_MCP,
        &ChannelState::server_root_url(),
        bearer_token,
        ambient_headers,
    )
}

/// Builds a fully resolved installation for `server`, pre-authenticated via the
/// `Authorization` header plus any `ambient_headers`.
fn builtin_mcp_installation(
    server: &BuiltinMcpServer,
    server_root: &str,
    bearer_token: &str,
    ambient_headers: &[(String, String)],
) -> TemplatableMCPServerInstallation {
    let mut headers = serde_json::Map::new();
    headers.insert(
        "Authorization".to_string(),
        serde_json::Value::String(format!("Bearer {bearer_token}")),
    );
    for (name, value) in ambient_headers {
        headers.insert(name.clone(), serde_json::Value::String(value.clone()));
    }
    let server_config = serde_json::json!({
        "url": server.url(server_root),
        "headers": headers,
    });
    let mut root = serde_json::Map::new();
    root.insert(server.name.to_string(), server_config);
    let template_json = serde_json::Value::Object(root).to_string();

    let templatable_mcp_server = TemplatableMCPServer {
        uuid: server.template_uuid,
        name: server.name.to_string(),
        description: Some(server.description.to_string()),
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
        server.installation_uuid,
        templatable_mcp_server,
        HashMap::new(),
    )
}

#[cfg(test)]
#[path = "builtin_tests.rs"]
mod tests;
