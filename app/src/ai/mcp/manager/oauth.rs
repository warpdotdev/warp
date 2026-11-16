/// Secure storage key for legacy MCP server OAuth credentials.
///
/// Kept so that `TemplatableMCPServerManager::copy_oauth_from_legacy_to_templatable`
/// can read credentials stored by the (now-removed) legacy `MCPServerManager` and
/// migrate them into the templatable credential store.
#[cfg(not(target_family = "wasm"))]
pub const LEGACY_MCP_CREDENTIALS_KEY: &str = "McpCredentials";
