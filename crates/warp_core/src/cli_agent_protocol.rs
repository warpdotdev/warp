use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Sentinel title that identifies structured CLI-agent events sent via OSC 777.
pub const CLI_AGENT_NOTIFICATION_SENTINEL: &str = "warp://cli-agent";

/// Schema version emitted by the current CLI-agent notification protocol.
pub const CLI_AGENT_PROTOCOL_VERSION: u32 = 1;

/// Environment variable that advertises the host's CLI-agent protocol version.
pub const WARP_CLI_AGENT_PROTOCOL_VERSION_ENV: &str = "WARP_CLI_AGENT_PROTOCOL_VERSION";

/// Environment variable that identifies the hosting Warp client version.
pub const WARP_CLIENT_VERSION_ENV: &str = "WARP_CLIENT_VERSION";

/// Schema version emitted on the Warp-to-agent control channel.
pub const CLI_AGENT_CONTROL_PROTOCOL_VERSION: u32 = 1;

/// Environment variable that advertises the pane-scoped Warp-to-agent control endpoint address.
pub const WARP_CLI_AGENT_CONTROL_SOCKET_ENV: &str = "WARP_CLI_AGENT_CONTROL_SOCKET";

/// Kinds of Warp-to-agent control events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CLIAgentControlEventKind {
    RichInput,
}

/// Wire representation of a Warp-to-agent control event, sent as one JSON line.
///
/// Kept separate from [`CLIAgentNotification`] so the two directions can evolve independently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CLIAgentControlEvent {
    pub v: u32,
    pub event: CLIAgentControlEventKind,
    pub active: bool,
}

impl CLIAgentControlEvent {
    pub fn rich_input(active: bool) -> Self {
        Self {
            v: CLI_AGENT_CONTROL_PROTOCOL_VERSION,
            event: CLIAgentControlEventKind::RichInput,
            active,
        }
    }
}

/// Wire representation of a structured CLI-agent notification.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CLIAgentNotification {
    pub v: Option<u32>,
    pub agent: Option<String>,
    pub event: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
    pub transcript_path: Option<String>,
    pub summary: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub plugin_version: Option<String>,
    pub error_type: Option<String>,
}

impl CLIAgentNotification {
    pub fn new(agent: impl Into<String>, event: impl Into<String>) -> Self {
        Self {
            v: Some(CLI_AGENT_PROTOCOL_VERSION),
            agent: Some(agent.into()),
            event: event.into(),
            session_id: None,
            cwd: None,
            project: None,
            query: None,
            response: None,
            transcript_path: None,
            summary: None,
            tool_name: None,
            tool_input: None,
            plugin_version: None,
            error_type: None,
        }
    }
}
