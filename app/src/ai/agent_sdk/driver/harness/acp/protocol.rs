//! Wire types for the Agent Client Protocol (<https://agentclientprotocol.com>) subset this
//! harness speaks. Field names follow ACP's `camelCase` keys and `snake_case` discriminators.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(super) const PROTOCOL_VERSION: u16 = 1;

pub(super) const METHOD_INITIALIZE: &str = "initialize";
pub(super) const METHOD_SESSION_NEW: &str = "session/new";
pub(super) const METHOD_SESSION_SET_CONFIG_OPTION: &str = "session/set_config_option";
pub(super) const METHOD_SESSION_PROMPT: &str = "session/prompt";
pub(super) const METHOD_SESSION_CANCEL: &str = "session/cancel";
pub(super) const METHOD_SESSION_UPDATE: &str = "session/update";
pub(super) const METHOD_REQUEST_PERMISSION: &str = "session/request_permission";
pub(super) const METHOD_FS_READ_TEXT_FILE: &str = "fs/read_text_file";
pub(super) const METHOD_FS_WRITE_TEXT_FILE: &str = "fs/write_text_file";
/// Extension shared by the Claude, Codex, and (pending) Pi adapters that adds a prompt to the
/// turn in flight instead of starting a new one. Not part of ACP v1; the agent advertises it in
/// the `_meta` of its `initialize` response.
pub(super) const METHOD_SESSION_STEERING: &str = "_session/steering";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeParams {
    pub protocol_version: u16,
    pub client_capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClientCapabilities {
    pub fs: FsCapabilities,
    pub terminal: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FsCapabilities {
    pub read_text_file: bool,
    pub write_text_file: bool,
}

#[derive(Serialize)]
pub(super) struct ClientInfo {
    pub name: String,
    pub title: String,
    pub version: String,
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct InitializeResponse {
    pub protocol_version: u16,
    pub agent_info: Option<AgentInfo>,
    pub agent_capabilities: AgentCapabilities,
    pub auth_methods: Vec<Value>,
    /// Extension capabilities, which adapters announce here rather than in `agentCapabilities`.
    #[serde(rename = "_meta")]
    pub meta: Value,
}

impl InitializeResponse {
    /// Whether the agent accepts [`METHOD_SESSION_STEERING`].
    pub(super) fn supports_steering(&self) -> bool {
        self.meta
            .pointer("/steering/supported")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// What the agent announced it supports at `initialize`. Every field defaults to "unsupported"
/// so an agent that omits the whole object is treated as a baseline ACP agent.
#[derive(Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct AgentCapabilities {
    pub load_session: bool,
    pub prompt_capabilities: PromptCapabilities,
    /// Capabilities this client does not model (e.g. `mcpCapabilities`), kept for logging.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// The prompt content kinds the agent accepts beyond text and resource links, which every ACP
/// agent must take.
#[derive(Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub(super) struct AgentInfo {
    pub name: String,
    pub version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NewSessionParams {
    pub cwd: String,
    pub mcp_servers: Vec<McpServer>,
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
pub(super) enum McpServer {
    #[serde(rename_all = "camelCase")]
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        env: Vec<EnvVariable>,
    },
    #[serde(rename_all = "camelCase")]
    Remote {
        #[serde(rename = "type")]
        kind: &'static str,
        name: String,
        url: String,
        headers: Vec<HttpHeader>,
    },
}

#[derive(Serialize, Clone)]
pub(super) struct EnvVariable {
    pub name: String,
    pub value: String,
}

#[derive(Serialize, Clone)]
pub(super) struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct NewSessionResponse {
    pub session_id: String,
    #[serde(default)]
    pub config_options: Vec<AcpAdvertisedSetting>,
    /// Adapter-specific extension data.
    #[serde(rename = "_meta", default)]
    pub meta: Value,
}

/// An adapter-advertised ACP setting. Model and thinking settings share this wire representation.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct AcpAdvertisedSetting {
    /// Adapter-specific identifier passed to `session/set_config_option`.
    pub id: String,
    pub category: Option<String>,
    #[serde(rename = "type")]
    pub input_type: String,
    #[serde(rename = "options", default)]
    pub choices: Vec<AcpSettingChoice>,
}

/// Selectable values advertised by an ACP adapter.
#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub(super) enum AcpSettingChoice {
    Value {
        value: String,
    },
    Group {
        #[serde(rename = "options")]
        choices: Vec<AcpSettingChoice>,
    },
    Unsupported {},
}

impl AcpSettingChoice {
    pub(super) fn supports_value(&self, requested: &str) -> bool {
        match self {
            Self::Value { value } => value == requested,
            Self::Group { choices } => choices
                .iter()
                .any(|choice| choice.supports_value(requested)),
            Self::Unsupported {} => false,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AcpSetSettingParams {
    pub session_id: String,
    pub config_id: String,
    pub value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AcpSetSettingResponse {
    pub config_options: Vec<AcpAdvertisedSetting>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PromptParams {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SteeringParams {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
    #[serde(rename = "_meta")]
    pub meta: SteeringMeta,
}

/// Request options for [`METHOD_SESSION_STEERING`]; adapters that do not know them ignore them.
#[derive(Serialize)]
pub(super) struct SteeringMeta {
    pub steering: SteeringOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SteeringOptions {
    /// What the agent does when no turn is running. `promptRequired` asks it to leave the
    /// prompt unconsumed and say so, rather than start a turn no `session/prompt` owns.
    pub idle_behavior: SteeringIdleBehavior,
}

#[derive(Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) enum SteeringIdleBehavior {
    PromptRequired,
}

#[derive(Deserialize, Debug)]
pub(super) struct SteeringResponse {
    pub outcome: SteeringOutcome,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) enum SteeringOutcome {
    /// The prompt joined the running turn, whose `session/prompt` keeps owning the output.
    Injected,
    /// No turn was running, so the agent started one that no `session/prompt` owns.
    StartedNewTurn,
    /// No turn was running and the prompt was left unconsumed for the client to send itself.
    PromptRequired,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ContentBlock {
    Text {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Image {
        #[serde(default)]
        data: String,
        #[serde(default)]
        mime_type: String,
    },
    ResourceLink {
        uri: String,
        #[serde(default)]
        name: Option<String>,
    },
    Resource {
        resource: EmbeddedResource,
    },
    #[serde(other)]
    Unsupported,
}

/// The contents of a `resource` block, carried inline as text or base64 `blob`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct EmbeddedResource {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

impl ContentBlock {
    /// Best-effort plain-text rendering of the block.
    pub(super) fn as_text(&self) -> String {
        match self {
            ContentBlock::Text { text } => text.clone(),
            ContentBlock::Image { mime_type, .. } => format!("[image {mime_type}]"),
            ContentBlock::ResourceLink { uri, name } => match name {
                Some(name) => format!("[{name}]({uri})"),
                None => uri.clone(),
            },
            ContentBlock::Resource { resource } => resource
                .text
                .clone()
                .unwrap_or_else(|| resource.uri.clone()),
            ContentBlock::Unsupported => String::new(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct PromptResponse {
    pub stop_reason: StopReason,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionUpdateParams {
    pub update: SessionUpdate,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub(super) enum SessionUpdate {
    UserMessageChunk {},
    #[serde(rename_all = "camelCase")]
    AgentMessageChunk {
        content: ContentBlock,
        #[serde(default)]
        message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    AgentThoughtChunk {
        content: ContentBlock,
        #[serde(default)]
        message_id: Option<String>,
    },
    ToolCall {
        #[serde(flatten)]
        call: ToolCallFields,
    },
    ToolCallUpdate {
        #[serde(flatten)]
        call: ToolCallFields,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    UsageUpdate {},
    #[serde(other)]
    Unsupported,
}

/// Fields shared by `tool_call` and `tool_call_update`; everything except the id is optional
/// in updates.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct ToolCallFields {
    pub tool_call_id: String,
    pub title: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: Option<ToolCallStatus>,
    pub content: Option<Vec<ToolCallContent>>,
    pub locations: Option<Vec<ToolCallLocation>>,
    pub raw_input: Option<Value>,
    pub raw_output: Option<Value>,
}

/// A file the tool call touches, as reported by the agent for "follow along" UIs.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub(super) struct ToolCallLocation {
    pub path: String,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ToolCallContent {
    Content {
        content: ContentBlock,
    },
    #[serde(rename_all = "camelCase")]
    Diff {
        path: String,
        #[serde(default)]
        old_text: Option<String>,
        new_text: String,
    },
    #[serde(rename_all = "camelCase")]
    Terminal {
        terminal_id: String,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Deserialize, Clone, Debug)]
pub(super) struct PlanEntry {
    pub content: String,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct RequestPermissionParams {
    /// The call the agent wants permission for; only the fields it chose to include.
    #[serde(default)]
    pub tool_call: ToolCallFields,
    pub options: Vec<PermissionOption>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct PermissionOption {
    pub option_id: String,
    #[serde(default)]
    pub name: String,
    pub kind: PermissionOptionKind,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    #[serde(other)]
    Unknown,
}

#[derive(Serialize)]
pub(super) struct RequestPermissionResponse {
    pub outcome: PermissionOutcome,
}

#[derive(Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(super) enum PermissionOutcome {
    #[serde(rename_all = "camelCase")]
    Selected {
        option_id: String,
    },
    Cancelled,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReadTextFileParams {
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct ReadTextFileResponse {
    pub content: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct WriteTextFileParams {
    pub path: String,
    pub content: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CancelParams {
    pub session_id: String,
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
