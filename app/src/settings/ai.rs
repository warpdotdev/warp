//! Settings for Blocklist AI.
//!
//! These settings are currently used to configure the underlying model/API used to power the AI
//! UX, as well as small UX configurations.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
pub use cloud_object_models::{
    AgentModeCommandExecutionPredicate, DEFAULT_COMMAND_EXECUTION_ALLOWLIST,
    DEFAULT_COMMAND_EXECUTION_DENYLIST,
};
use indexmap::IndexMap;
use regex::Regex;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use settings::{
    define_settings_group, RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use warp_core::execution_mode::AppExecutionMode;
use warp_core::features::FeatureFlag;
use warpui::platform::keyboard::KeyCode;
use warpui::platform::OperatingSystem;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, UpdateModel};

use crate::ai::request_usage_model::RequestLimitInfo;
use crate::auth::AuthStateProvider;
use crate::report_if_error;
use crate::settings::PrivacySettings;
use crate::terminal::CLIAgent;
use crate::workspaces::user_workspaces::UserWorkspaces;

pub enum FocusedTerminalInfoEvent {
    TerminalInfoUpdated,
}

/// Singleton model that is used to track the remote sessions in the terminal.
/// Useful for organizations that have restrictions on using AI in sessions in
/// remote sessions.
#[derive(Default, Clone, Debug)]
pub struct FocusedTerminalInfo {
    contains_any_remote_blocks: bool,
    contains_any_restored_remote_blocks: bool,
}

impl FocusedTerminalInfo {
    pub fn new(_: &mut ModelContext<Self>) -> Self {
        Self {
            contains_any_remote_blocks: false,
            contains_any_restored_remote_blocks: false,
        }
    }

    pub fn contains_any_remote_blocks(&self) -> bool {
        self.contains_any_remote_blocks
    }

    pub fn contains_any_restored_remote_blocks(&self) -> bool {
        self.contains_any_restored_remote_blocks
    }

    /// Updates both remote blocks and restored blocks status in a single atomic operation.
    /// Only emits a TerminalInfoUpdated event if either value changes.
    /// Returns true if the event was emitted.
    pub fn update(
        &mut self,
        contains_any_remote_blocks: bool,
        contains_any_restored_remote_blocks: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let remote_changed = self.contains_any_remote_blocks != contains_any_remote_blocks;
        let restored_changed =
            self.contains_any_restored_remote_blocks != contains_any_restored_remote_blocks;

        if remote_changed || restored_changed {
            self.contains_any_remote_blocks = contains_any_remote_blocks;
            self.contains_any_restored_remote_blocks = contains_any_restored_remote_blocks;
            ctx.emit(FocusedTerminalInfoEvent::TerminalInfoUpdated);
            return true;
        }

        false
    }
}

impl Entity for FocusedTerminalInfo {
    type Event = FocusedTerminalInfoEvent;
}

impl SingletonEntity for FocusedTerminalInfo {}

#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    EnumIter,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Physical key used to toggle voice input.",
    rename_all = "snake_case"
)]
pub enum VoiceInputToggleKey {
    #[default]
    #[schemars(description = "No toggle key assigned.")]
    None,
    /// Fn key is default toggle key for Mac, when the feature is toggled on.
    #[schemars(description = "Fn key.")]
    Fn,
    /// Alt or Option key (left side).
    #[schemars(description = "Alt or Option key (left side).")]
    AltLeft,
    /// Alt or Option key (right side). Used as default toggle
    /// key for Windows and Linux, , when the feature is toggled on.
    #[schemars(description = "Alt or Option key (right side).")]
    AltRight,
    #[schemars(description = "Control key (left side).")]
    ControlLeft,
    #[schemars(description = "Control key (right side).")]
    ControlRight,
    /// The Windows, ⌘, Command, or other OS symbol key.
    #[schemars(description = "Super, Windows, or Command key (left side).")]
    SuperLeft,
    /// The Windows, ⌘, Command, or other OS symbol key.
    #[schemars(description = "Super, Windows, or Command key (right side).")]
    SuperRight,
    #[schemars(description = "Shift key (left side).")]
    ShiftLeft,
    #[schemars(description = "Shift key (right side).")]
    ShiftRight,
}

settings::macros::implement_setting_for_enum!(
    VoiceInputToggleKey,
    AISettings,
    SupportedPlatforms::DESKTOP,
    // Never sync to cloud to allow users to use different toggle keys on different devices,
    // especially given platform differences.
    SyncToCloud::Never,
    private: false,
    toml_path: "agents.voice.voice_input_toggle_key",
    description: "The key used to toggle voice input.",
);

impl VoiceInputToggleKey {
    pub fn all_possible_values() -> Vec<VoiceInputToggleKey> {
        let all_keys = VoiceInputToggleKey::iter().collect();
        match OperatingSystem::get() {
            OperatingSystem::Mac => all_keys,
            // For non-Mac platforms, we exclude the `Fn` key since it may not be correctly identified by winit.
            // In particular, we saw it is unidentified for a ThinkPad with a standard keyboard.
            OperatingSystem::Windows | OperatingSystem::Linux | OperatingSystem::Other(_) => {
                all_keys
                    .into_iter()
                    .filter(|key| *key != VoiceInputToggleKey::Fn)
                    .collect()
            }
        }
    }

    /// Display name for choosing key from the AI settings page.
    pub fn display_name(&self) -> &'static str {
        // We use the underlying host OS to determine the correct key name to display.
        let (super_key_name, alt_key_name): (&'static str, &'static str) =
            match OperatingSystem::get() {
                OperatingSystem::Mac => ("Command", "Option"),
                OperatingSystem::Windows => ("Windows", "Alt"),
                OperatingSystem::Linux | OperatingSystem::Other(_) => ("Super", "Alt"),
            };

        match self {
            VoiceInputToggleKey::None => "None",
            VoiceInputToggleKey::Fn => "Fn",
            VoiceInputToggleKey::AltLeft => {
                Box::leak(format!("{alt_key_name} (Left)").into_boxed_str())
            }
            VoiceInputToggleKey::AltRight => {
                Box::leak(format!("{alt_key_name} (Right)").into_boxed_str())
            }
            VoiceInputToggleKey::ControlLeft => "Control (Left)",
            VoiceInputToggleKey::ControlRight => "Control (Right)",
            VoiceInputToggleKey::SuperLeft => {
                Box::leak(format!("{super_key_name} (Left)").into_boxed_str())
            }
            VoiceInputToggleKey::SuperRight => {
                Box::leak(format!("{super_key_name} (Right)").into_boxed_str())
            }
            VoiceInputToggleKey::ShiftLeft => "Shift (Left)",
            VoiceInputToggleKey::ShiftRight => "Shift (Right)",
        }
    }

    pub fn to_key_code(&self) -> Option<KeyCode> {
        match self {
            VoiceInputToggleKey::None => None,
            VoiceInputToggleKey::Fn => Some(KeyCode::Fn),
            VoiceInputToggleKey::AltLeft => Some(KeyCode::AltLeft),
            VoiceInputToggleKey::AltRight => Some(KeyCode::AltRight),
            VoiceInputToggleKey::ControlLeft => Some(KeyCode::ControlLeft),
            VoiceInputToggleKey::ControlRight => Some(KeyCode::ControlRight),
            VoiceInputToggleKey::SuperLeft => Some(KeyCode::SuperLeft),
            VoiceInputToggleKey::SuperRight => Some(KeyCode::SuperRight),
            VoiceInputToggleKey::ShiftLeft => Some(KeyCode::ShiftLeft),
            VoiceInputToggleKey::ShiftRight => Some(KeyCode::ShiftRight),
        }
    }

    /// Converts the voice input toggle key to a Keystroke representation.
    /// Since these are standalone modifier keys, we construct the Keystroke directly
    /// rather than using `parse()` (which always requires a non-modifier key to be included).
    pub fn keystroke(&self) -> Option<warpui::keymap::Keystroke> {
        use warpui::keymap::Keystroke;

        let keystroke = match self {
            VoiceInputToggleKey::None => return None,
            VoiceInputToggleKey::Fn => Keystroke {
                key: "fn".to_string(),
                ..Default::default()
            },
            VoiceInputToggleKey::AltLeft | VoiceInputToggleKey::AltRight => Keystroke {
                alt: true,
                ..Default::default()
            },
            VoiceInputToggleKey::ControlLeft | VoiceInputToggleKey::ControlRight => Keystroke {
                ctrl: true,
                ..Default::default()
            },
            VoiceInputToggleKey::SuperLeft | VoiceInputToggleKey::SuperRight => Keystroke {
                cmd: true,
                ..Default::default()
            },
            VoiceInputToggleKey::ShiftLeft | VoiceInputToggleKey::ShiftRight => Keystroke {
                shift: true,
                ..Default::default()
            },
        };
        Some(keystroke)
    }

    pub fn tooltip_message(&self) -> String {
        match self.keystroke() {
            Some(keystroke) => {
                let symbol = keystroke.displayed();
                let side = match self {
                    VoiceInputToggleKey::AltLeft
                    | VoiceInputToggleKey::ControlLeft
                    | VoiceInputToggleKey::SuperLeft
                    | VoiceInputToggleKey::ShiftLeft => Some("Left"),
                    VoiceInputToggleKey::AltRight
                    | VoiceInputToggleKey::ControlRight
                    | VoiceInputToggleKey::SuperRight
                    | VoiceInputToggleKey::ShiftRight => Some("Right"),
                    VoiceInputToggleKey::None | VoiceInputToggleKey::Fn => None,
                };
                let key_name = match side {
                    Some(side) => format!("{side} {symbol}"),
                    None => symbol,
                };
                format!("Voice input (hold {key_name} key)")
            }
            None => "Voice input".to_string(),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, VoiceInputToggleKey::None)
    }
}

/// The default mode for new terminal sessions.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    EnumIter,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Default mode for new sessions.",
    rename_all = "snake_case"
)]
pub enum DefaultSessionMode {
    /// New sessions start in the terminal mode (default).
    #[default]
    Terminal,
    /// New sessions start in agent view.
    Agent,
    /// New sessions start in cloud (ambient) agent mode.
    CloudAgent,
    /// New sessions open a user-defined tab config.
    /// The specific config is identified by the companion `default_tab_config_path` setting.
    TabConfig,
    /// New sessions open in a local Docker sandbox.
    /// Requires the `LocalDockerSandbox` feature flag; falls back to `Terminal` when disabled.
    DockerSandbox,
}

settings::macros::implement_setting_for_enum!(
    DefaultSessionMode,
    AISettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "general.default_session_mode",
    description: "The default mode for new terminal sessions.",
);

impl DefaultSessionMode {
    /// Display name for the settings dropdown.
    pub fn display_name(&self) -> &'static str {
        match self {
            DefaultSessionMode::Terminal => "Terminal",
            DefaultSessionMode::Agent => "Agent",
            DefaultSessionMode::CloudAgent => "Cloud Oz",
            DefaultSessionMode::TabConfig => "Tab Config",
            DefaultSessionMode::DockerSandbox => "Local Docker Sandbox",
        }
    }
}

/// Controls how agent thinking/reasoning traces are displayed after streaming.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    EnumIter,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Controls how agent thinking is displayed after streaming.",
    rename_all = "snake_case"
)]
pub enum ThinkingDisplayMode {
    /// Show reasoning blocks while streaming, then collapse them when complete (default).
    #[default]
    ShowAndCollapse,
    /// Always keep reasoning blocks expanded, even after streaming finishes.
    AlwaysShow,
    /// Never show reasoning blocks.
    NeverShow,
}

settings::macros::implement_setting_for_enum!(
    ThinkingDisplayMode,
    AISettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "agents.warp_agent.other.thinking_display_mode",
    description: "Controls how agent thinking traces are displayed after streaming.",
);

impl ThinkingDisplayMode {
    /// Display name for the settings dropdown.
    pub fn display_name(&self) -> &'static str {
        match self {
            ThinkingDisplayMode::ShowAndCollapse => "Show & collapse",
            ThinkingDisplayMode::AlwaysShow => "Always show",
            ThinkingDisplayMode::NeverShow => "Never show",
        }
    }

    pub fn command_palette_description(&self) -> &'static str {
        match self {
            ThinkingDisplayMode::ShowAndCollapse => "Set agent thinking display: show & collapse",
            ThinkingDisplayMode::AlwaysShow => "Set agent thinking display: always show",
            ThinkingDisplayMode::NeverShow => "Set agent thinking display: never show",
        }
    }

    pub fn should_render(&self) -> bool {
        !matches!(self, ThinkingDisplayMode::NeverShow)
    }

    pub fn should_keep_expanded(&self) -> bool {
        matches!(self, ThinkingDisplayMode::AlwaysShow)
    }
}

/// Controls how child-agent message bodies are displayed.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    EnumIter,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Controls how child-agent messages are displayed.",
    rename_all = "snake_case"
)]
pub enum OrchestrationMessageDisplayMode {
    /// Show child-agent messages while streaming, then collapse them.
    ShowAndCollapse,
    /// Keep child-agent message bodies expanded.
    AlwaysShow,
    /// Keep child-agent message bodies collapsed.
    #[default]
    AlwaysCollapse,
}

settings::macros::implement_setting_for_enum!(
    OrchestrationMessageDisplayMode,
    AISettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "agents.warp_agent.other.orchestration_message_display_mode",
    description: "Controls how child-agent messages are displayed.",
);

impl OrchestrationMessageDisplayMode {
    /// Display name for the settings dropdown.
    pub fn display_name(&self) -> &'static str {
        match self {
            OrchestrationMessageDisplayMode::ShowAndCollapse => "Show & collapse",
            OrchestrationMessageDisplayMode::AlwaysShow => "Always show",
            OrchestrationMessageDisplayMode::AlwaysCollapse => "Always collapse",
        }
    }

    pub fn command_palette_description(&self) -> &'static str {
        match self {
            OrchestrationMessageDisplayMode::ShowAndCollapse => {
                "Set child-agent message display: show & collapse"
            }
            OrchestrationMessageDisplayMode::AlwaysShow => {
                "Set child-agent message display: always show"
            }
            OrchestrationMessageDisplayMode::AlwaysCollapse => {
                "Set child-agent message display: always collapse"
            }
        }
    }

    /// Whether child-agent message bodies should expand while streaming.
    pub fn should_expand_agent_message_body(&self) -> bool {
        matches!(
            self,
            OrchestrationMessageDisplayMode::ShowAndCollapse
                | OrchestrationMessageDisplayMode::AlwaysShow
        )
    }

    /// Whether child-agent message bodies should collapse after streaming.
    pub fn should_collapse_agent_message_body_on_finish(&self) -> bool {
        matches!(self, OrchestrationMessageDisplayMode::ShowAndCollapse)
    }
}

/// Controls what happens when a user submits a new prompt while the agent is
/// still responding to an earlier prompt.
///
/// This is the *default* used when a conversation has no explicit auto-queue
/// override. Per-conversation overrides live on `QueuedQueryModel` and take
/// precedence over this setting.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    EnumIter,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Default behavior when submitting a new prompt while the agent is still responding.",
    rename_all = "snake_case"
)]
pub enum PromptSubmissionMode {
    /// Cancel the in-flight response and submit the new prompt immediately
    /// (default).
    #[default]
    Interrupt,
    /// Hold the new prompt until the in-flight response finishes, then submit.
    Queue,
}

settings::macros::implement_setting_for_enum!(
    PromptSubmissionMode,
    AISettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "agents.warp_agent.other.default_prompt_submission_mode",
    description: "Default behavior when submitting a new prompt while the agent is still responding.",
    feature_flag: FeatureFlag::QueueSlashCommand,
);

impl PromptSubmissionMode {
    /// Display name for the settings dropdown.
    pub fn display_name(&self) -> &'static str {
        match self {
            PromptSubmissionMode::Interrupt => "Interrupt response",
            PromptSubmissionMode::Queue => "Queue until response finishes",
        }
    }

    pub fn command_palette_description(&self) -> &'static str {
        match self {
            PromptSubmissionMode::Interrupt => "Set default prompt submission: interrupt response",
            PromptSubmissionMode::Queue => {
                "Set default prompt submission: queue until response finishes"
            }
        }
    }
}

/// Tracks the state of the quota reset banner
#[derive(
    Debug,
    Serialize,
    Deserialize,
    Clone,
    PartialEq,
    Default,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "State of the quota reset banner.")]
pub struct BannerState {
    #[serde(default)]
    #[schemars(description = "Whether the banner has been dismissed.")]
    pub dismissed: bool,
}

/// Tracks information about a single billing cycle for AI request usage
#[derive(
    Debug,
    Serialize,
    Deserialize,
    Clone,
    PartialEq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Information about a single billing cycle.")]
pub struct CycleInfo {
    /// End date of the billing cycle
    #[schemars(description = "End date of the billing cycle.")]
    pub end_date: DateTime<Utc>,
    /// Whether the quota was exceeded in this cycle
    #[schemars(description = "Whether the usage quota was exceeded in this cycle.")]
    pub was_quota_exceeded: bool,
    /// State of the quota reset banner
    #[schemars(description = "State of the quota reset banner for this cycle.")]
    pub banner_state: BannerState,
}

#[derive(
    Debug,
    Serialize,
    Deserialize,
    Clone,
    Default,
    PartialEq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "AI usage quota information across billing cycles.")]
pub struct AIRequestQuotaInfo {
    /// History of billing cycles and their usage.
    ///
    /// Note that these are only populated going forward from when this setting
    /// was introduced.
    #[schemars(description = "History of billing cycles and their quota usage.")]
    pub cycle_history: Vec<CycleInfo>,
}

#[derive(
    Debug,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Default,
    PartialEq,
    EnumIter,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "File read permission level for the agent.",
    rename_all = "snake_case"
)]
pub enum AgentModeCodingPermissionsType {
    /// Agent Mode must ask for explicit permission for any type of file read.
    #[default]
    AlwaysAskBeforeReading,
    /// Agent Mode can always read files without explicit consent.
    AlwaysAllowReading,
    /// Agent Mode can only read certain files without explicit consent.
    ///
    /// The specific filepaths are backed by the
    /// [`AISettings::agent_mode_coding_file_read_allowlist`] setting.
    AllowReadingSpecificFiles,
}

/// Maps custom toolbar command regex patterns to CLI agent names.
/// Keys are regex patterns (insertion-ordered), values are serialized CLIAgent names (e.g. "Claude").
/// An empty string value means "Any CLI Agent" (CLIAgent::Unknown).
///
/// Uses `IndexMap` to preserve insertion order so the settings UI list is deterministic.
/// Supports backward-compatible deserialization from the legacy `Vec<String>` format,
/// where each string is converted to a key with an empty agent value.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ToolbarCommandMap(IndexMap<String, String>);

impl ToolbarCommandMap {
    pub(crate) fn new(map: IndexMap<String, String>) -> Self {
        Self(map)
    }
}

impl<'de> Deserialize<'de> for ToolbarCommandMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum MapOrVec {
            Map(IndexMap<String, String>),
            Vec(Vec<String>),
        }

        match MapOrVec::deserialize(deserializer) {
            Ok(MapOrVec::Map(map)) => Ok(ToolbarCommandMap::new(map)),
            Ok(MapOrVec::Vec(vec)) => {
                let map = vec
                    .into_iter()
                    .map(|pattern| (pattern, String::new()))
                    .collect();
                Ok(ToolbarCommandMap::new(map))
            }
            Err(e) => Err(e),
        }
    }
}

impl schemars::JsonSchema for ToolbarCommandMap {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("ToolbarCommandMap")
    }

    fn json_schema(gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        gen.subschema_for::<HashMap<String, String>>()
    }
}

impl std::ops::Deref for ToolbarCommandMap {
    type Target = IndexMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl settings_value::SettingsValue for ToolbarCommandMap {
    fn to_file_value(&self) -> serde_json::Value {
        serde_json::to_value(&self.0).unwrap_or_default()
    }

    fn from_file_value(value: &serde_json::Value) -> Option<Self> {
        // Try map format first (using from_value to preserve insertion order), then legacy array format.
        if value.is_object() {
            if let Ok(map) = serde_json::from_value::<IndexMap<String, String>>(value.clone()) {
                return Some(ToolbarCommandMap::new(map));
            }
        }
        if let Some(arr) = value.as_array() {
            let result: IndexMap<String, String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| (s.to_string(), String::new())))
                .collect();
            return Some(ToolbarCommandMap::new(result));
        }
        None
    }
}

define_settings_group!(AISettings, settings: [
    // If `false`, all AI features are disabled.
    is_any_ai_enabled: IsAnyAIEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: false,
        toml_path: "agents.warp_agent.is_any_ai_enabled",
        description: "Controls whether all AI features are enabled.",
    },
    // This field should not be referenced directly to lookup active AI enablement -- use the
    // `is_active_ai_enabled()` getter.
    is_active_ai_enabled_internal: IsActiveAIEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: false,
        toml_path: "agents.warp_agent.active_ai.enabled",
        description: "Controls whether proactive AI features like suggestions are enabled.",
    },
    // This field should not be referenced directly to lookup autodetection enablement -- use the
    // `is_ai_autodetection_enabled()` getter.
    ai_autodetection_enabled_internal: AIAutoDetectionEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.input.ai_auto_detection_enabled",
        description: "Controls whether AI automatically detects natural language input.",
    },
    // This field should not be referenced directly -- use the
    // `is_nld_in_terminal_enabled()` getter.
    // Controls whether natural language detection is enabled in the terminal input.
    //
    // This is only used when `FeatureFlag::AgentView` is enabled.
    nld_in_terminal_enabled_internal: NLDInTerminalEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.input.nld_in_terminal_enabled",
        description: "Controls whether natural language detection is enabled in the terminal input.",
    },
    autodetection_command_denylist: AICommandDenylist {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.input.ai_command_denylist",
        description: "Commands to exclude from AI natural language autodetection.",
    },
    // This field should not be referenced directly to lookup intelligent autosuggestion enablement
    // -- use the `is_intelligent_autosuggestions_enabled()` getter.
    intelligent_autosuggestions_enabled_internal: IntelligentAutosuggestionsEnabled {
        type: bool,
        default: true, // TODO(roland): revisit this when launched to stable
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.active_ai.intelligent_autosuggestions_enabled",
        description: "Controls whether AI-powered intelligent autosuggestions are enabled.",
    }
    // This field should not be referenced directly to lookup Prompt Suggestions
    // enablement -- use the `is_prompt_suggestions_enabled()` getter.
    // Note that AgentModeQuerySuggestionsEnabled is a legacy name (the feature was initially named Agent
    // Mode Query Suggestions), however, we do not want to change the name of the setting key to avoid
    // breaking existing user settings.
    prompt_suggestions_enabled_internal: AgentModeQuerySuggestionsEnabled {
        type: bool,
        default: true, // TODO(advait): revisit this when launched to stable
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.active_ai.agent_mode_query_suggestions_enabled",
        description: "Controls whether prompt suggestions are shown in agent mode.",
    }

    // This field should not be referenced directly to lookup Code Suggestions
    // enablement -- use the `is_code_suggestions_enabled()` getter.
    code_suggestions_enabled_internal: CodeSuggestionsEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.active_ai.code_suggestions_enabled",
        description: "Controls whether AI code suggestions are enabled.",
    }
    // This field should not be referenced directly to lookup natural language autosuggestions
    // enablement -- use the `is_natural_language_autosuggestions_enabled()` getter.
    // This feature refers to ghosted text for AI input queries.
    natural_language_autosuggestions_enabled_internal: NaturalLanguageAutosuggestionsEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.active_ai.natural_language_autosuggestions_enabled",
        description: "Controls whether ghosted text autosuggestions are shown for AI input queries.",
        feature_flag: FeatureFlag::PredictAMQueries,
    }
    // This field should not be referenced directly to lookup shared block title generations
    // enablement -- use the `is_shared_block_title_generation_enabled()` getter.
    // This feature refers to the auto title generation when the user opens the shared block dialog.
    shared_block_title_generation_enabled_internal: SharedBlockTitleGenerationEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.active_ai.shared_block_title_generation_enabled",
        description: "Controls whether titles are auto-generated when sharing blocks.",
    }
    // This field should not be referenced directly to lookup git operations AI autogen
    // enablement -- use the `is_git_operations_autogen_enabled()` getter.
    git_operations_autogen_enabled_internal: GitOperationsAutogenEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.active_ai.git_operations_autogen_enabled",
        description: "Controls whether AI auto-generates commit messages and PR title/body in the code review dialogs.",
    }
    // This field should not be referenced directly to lookup Rule Suggestions
    // enablement -- use the `is_rule_suggestions_enabled()` getter.
    rule_suggestions_enabled_internal: RuleSuggestionsEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.active_ai.rule_suggestions_enabled",
        description: "Controls whether the agent suggests rules to save after responses.",
        feature_flag: FeatureFlag::SuggestedRules,
    }
    // This field should not be referenced directly to lookup Voice AI enablement -- use the
    // `is_voice_input_enabled()` getter.
    voice_input_enabled_internal: VoiceInputEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.voice.voice_input_enabled",
        description: "Controls whether voice input is enabled for AI interactions.",
    },
    // The number of times the user has entered Agent Mode.
    // Not a user-visible setting. We model it so we can show the voice input new feature popup
    // the correct number of times.
    entered_agent_mode_num_times: EnteredAgentModeNumTimes {
        type: usize,
        default: 0,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    // Whether or not the user has manually dismissed the voice input new feature popup.
    dismissed_voice_input_new_feature_popup: DismissedVoiceInputNewFeaturePopup {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    // This field is used to store the key used for voice input toggling.
    // Note this is not the named key, but rather corresponds to the physical key.
    voice_input_toggle_key: VoiceInputToggleKey,
    // This is not a user-visible setting - it's merely a one-time flag to track if the user has
    // explicitly interacted with voice input. We use this to determine whether we should show a toast
    // to inform the user about voice input and auto-set the keybinding.
    explicitly_interacted_with_voice: ExplicitlyInteractedWithVoice {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        // Never sync to cloud to keep state separate across devices, since microphone access is per-device.
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    },
    // Predicates that Agent Mode can use to decide if it can execute
    // a command without explicit user consent.
    //
    // Prefer [`BlocklistAIPermissions::can_autoexecute_command`] to
    // interpret this allowlist.
    agent_mode_command_execution_allowlist: AgentModeCommandExecutionAllowlist {
        type: Vec<AgentModeCommandExecutionPredicate>,
        default: DEFAULT_COMMAND_EXECUTION_ALLOWLIST.clone(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.profiles.agent_mode_command_execution_allowlist",
        description: "Commands that the agent can execute without explicit permission.",
    },
    // Predicates that Agent Mode can use to decide if a command must
    // be executed by the user.
    //
    // Prefer [`BlocklistAIPermissions::can_autoexecute_command`] to
    // interpret this denylist.
    agent_mode_command_execution_denylist: AgentModeCommandExecutionDenylist {
        type: Vec<AgentModeCommandExecutionPredicate>,
        default: DEFAULT_COMMAND_EXECUTION_DENYLIST.clone(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.profiles.agent_mode_command_execution_denylist",
        description: "Commands that the agent must always ask before executing.",
    },
    // Enabled iff Agent Mode can execute readonly commands without explicit user consent.
    //
    // Prefer [`BlocklistAIPermissions::can_autoexecute_command`] to
    // interpret this setting.
    agent_mode_execute_read_only_commands: AgentModeExecuteReadonlyCommands {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.profiles.agent_mode_execute_readonly_commands",
        description: "Whether the agent can auto-execute read-only commands without asking.",
    },
    // Determines coding permissions that Agent Mode has.
    // Note that if Agent Mode has permissions to execute readonly commands,
    // that automatically gives Agent Mode the ability to also _read_ files for coding
    // tasks, including codebase search.
    //
    // Prefer [`BlocklistAIPermissions::can_read_file`] to interpret this setting.
    agent_mode_coding_permissions: AgentModeCodingPermissions {
        type: AgentModeCodingPermissionsType,
        default: AgentModeCodingPermissionsType::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.profiles.agent_mode_coding_permissions",
        description: "The file read permission level for the agent.",
    }
    // Specific filepaths that Agent Mode can read without asking for additional permissions.
    // These should be persisted as absolute filepaths to avoid ambiguity.
    //
    // This is used in conjunction with [`AgentModeCodingPermissionsType::AllowReadingSpecificFiles`]
    // but modelled as a separate setting because it is not cloud-synced.
    //
    // Prefer [`BlocklistAIPermissions::can_read_file`] to interpret this setting.
    agent_mode_coding_file_read_allowlist: AgentModeCodingFileReadAllowlist {
        type: Vec<PathBuf>,
        default: vec![],
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agents.profiles.agent_mode_coding_file_read_allowlist",
        description: "File paths the agent can read without asking for permission.",
    }
    // Whether or not the profile-level command autoexecution speedbump has been shown.
    //
    // Not a user-visible setting - we model it as a setting so we can track how often
    // it's shown across devices.
    has_shown_agent_mode_profile_command_autoexecution_speedbump: HasShownAgentModeProfileCommandAutoexecutionSpeedbump {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }
    // Whether or not we should show the speedbump for auto-executing readonly cmds.
    //
    // Not a user-visible settings - we model it as a setting so we can track how often
    // it's shown across devices.
    should_show_agent_mode_autoexecute_readonly_commands_speedbump: ShouldShowAgentModeModelExecuteReadonlyCommandsSpeedbump {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }
    // Whether or not we should show the speedbump for auto-writing to the PTY.
    //
    // Not a user-visible settings - we model it as a setting so we can track how often
    // it's shown across devices.
    should_show_agent_mode_write_to_pty_speedbump: ShouldShowAgentModeWriteToPtySpeedbump {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }
    // Whether or not we should show the speedbump for auto-reading files.
    //
    // Not a user-visible settings - we model it as a setting so we can track how often
    // it's shown across devices.
    should_show_agent_mode_autoread_files_speedbump: ShouldShowAgentModeCodingReadPermissionsNudge {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }
    // Whether or not we should show the one-shot speedbump on Ask-User-Question cards.
    //
    // Not a user-visible setting - we model it as a setting so we can track state.
    // Intentionally NOT cloud-synced: we want users to see the first-time nudge on
    // each fresh device, and we avoid a cloud-sync race that would make the flag
    // silently stay `false` on new devices after being consumed once elsewhere.
    should_show_agent_mode_ask_user_question_speedbump: ShouldShowAgentModeAskUserQuestionSpeedbump {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    }
    // Whether to use locally loaded AWS credentials for Bedrock-enabled requests.
    aws_bedrock_credentials_enabled: AwsBedrockCredentialsEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "cloud_platform.third_party_api_keys.aws_bedrock_credentials_enabled",
        description: "Whether Warp should use your local AWS credentials for Bedrock-enabled requests.",
    }
    // Whether to automatically run the AWS login command when Bedrock credentials are expired.
    //
    // When true, the configured login command will be run automatically without asking.
    // When false (default), a prompt will be shown asking for permission.
    aws_bedrock_auto_login: AwsBedrockAutoLogin {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "cloud_platform.third_party_api_keys.aws_bedrock_auto_login",
        description: "Whether to automatically run the AWS login command when Bedrock credentials expire.",
    }
    // Command to run to refresh AWS credentials when using Bedrock auto-login.
    aws_bedrock_auth_refresh_command: AwsBedrockAuthRefreshCommand {
        type: String,
        default: "aws login".to_string(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "cloud_platform.third_party_api_keys.aws_bedrock_auth_refresh_command",
        description: "The command to run to refresh AWS credentials for Bedrock.",
    }
    // AWS profile name to use when loading credentials from the local AWS credential/config chain.
    aws_bedrock_profile: AwsBedrockProfile {
        type: String,
        default: "default".to_string(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "cloud_platform.third_party_api_keys.aws_bedrock_profile",
        description: "The AWS profile name to use for Bedrock credentials.",
    }
    // Whether the AWS Bedrock login banner has been permanently dismissed.
    //
    // Not a user-visible setting - we model it as a setting so we can track state.
    aws_bedrock_login_banner_dismissed: AwsBedrockLoginBannerDismissed {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }
    // Whether or not the user wants agent mode requests to use their saved rules.
    memory_enabled: MemoryEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.knowledge.rules_enabled",
        description: "Whether the agent uses your saved rules during requests.",
    }
    // Whether warp drive context should be included in AI requests
    warp_drive_context_enabled: WarpDriveContextEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.knowledge.warp_drive_context_enabled",
        description: "Whether Warp Drive context is included in AI requests.",
    }

    // Whether the codebase speedbump banner has been permanently dismissed for a given repo path.
    //
    // Not a user-visible settings - we model it as a setting so we can track state.
    codebase_index_speedbump_banner_dismissed_for_repo_paths: CodebaseIndexSpeedbumpBannerDismissedForRepoPaths {
        type: Vec<PathBuf>,
        default: vec![],
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    }

    // Whether the agent mode setup banner has been shown for a given repo path.
    // Once shown, it will not be shown again for that repo.
    //
    // Not a user-visible settings - we model it as a setting so we can track state.
    agent_mode_setup_banner_shown_for_repo_paths: AgentModeSetupBannerShownForRepoPaths {
        type: Vec<PathBuf>,
        default: vec![],
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    }

    // Whether the codebase speedbump banner has been globally dismissed ("Don't show again").
    //
    // Not a user-visible settings - we model it as a setting so we can track state.
    codebase_index_speedbump_banner_globally_dismissed: CodebaseIndexSpeedbumpBannerGloballyDismissed {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }

    // Information about AI request quotas and usage across billing cycles
    ai_request_quota_info: AIRequestQuotaInfoSetting {
        type: AIRequestQuotaInfo,
        default: AIRequestQuotaInfo::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },

    // Whether or not we should show the speedbump for showing code suggestion banners.
    // This includes both passive code diffs and suggested prompts (passive unit tests).
    //
    // Not a user-visible settings - we model it as a setting so we can track if the speedbump has already been shown or not.
    show_code_suggestion_speedbump: ShouldShowCodeSuggestionSpeedbump {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }

    mcp_execution_path: MCPExecutionPath {
        type: Option<String>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    },

    // This is not a user-visible setting - its merely a one-time flag to track if the agents 3 launch modal
    // has been shown to the user.
    //
    // We model it as a setting so it's only shown once to a given user regardless of the number of
    // devices they use.
    did_check_to_trigger_agents_3_launch_modal: DidShowAgents3LaunchModal {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: true,
    }

    // This is not a user-visible setting - it's merely a one-time flag to track if the Oz launch modal
    // has been shown to the user.
    //
    // We model it as a setting so it's only shown once to a given user regardless of the number of
    // devices they use.
    did_check_to_trigger_oz_launch_modal: DidShowOzLaunchModal {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: true,
    }

    // This is not a user-visible setting - it's merely a one-time flag to track if the
    // orchestration launch modal has been shown to the user.
    //
    // We model it as a setting so it's only shown once to a given user regardless of the number of
    // devices they use.
    did_check_to_trigger_orchestration_launch_modal: DidShowOrchestrationLaunchModal {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: true,
    }

    // Used to determine whether the "What's new in Oz" section of the agent view
    // zero state is expanded or collapsed by default.
    should_expand_oz_updates: ShouldExpandOzUpdates {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    }

    // Used to determine whether the "What's new in Oz" section of the agent view
    // zero state is shown or hidden.
    should_show_oz_updates_in_zero_state: ShouldShowOzUpdatesInZeroState {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.other.should_show_oz_updates_in_zero_state",
        description: "Whether the \"What's new\" section is shown in the agent view.",
    }

    // Whether or not the user has enabled fallback to Warp credits for user-provided models.
    can_use_warp_credits_for_fallback: CanUseWarpCreditsForFallback {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "CanUseWarpCreditsWithByok",
        toml_path: "cloud_platform.third_party_api_keys.can_use_warp_credits_with_byok",
        description: "Whether Warp credits can be used as a fallback for user-provided models.",
    }

    should_render_use_agent_footer_for_user_commands: ShouldRenderUseAgentToolbarForUserCommands {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.other.should_render_use_agent_toolbar_for_user_commands",
        description: "Whether to show the \"Use Agent\" footer for terminal commands.",
    }

    // Whether to render the CLI agent footer for commands like Claude, Codex, Gemini, etc.
    // This is independent of the "Use Agent" footer setting.
    should_render_cli_agent_footer: ShouldRenderCLIAgentToolbar {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.should_render_cli_agent_toolbar",
        description: "Whether to show the CLI agent footer for coding agent commands.",
    }
    // When enabled and a CLI agent session has a plugin listener, rich input
    // auto-closes when the session enters a Blocked state (the agent requires
    // direct keyboard interaction) and auto-opens when it leaves Blocked.
    auto_toggle_rich_input: AutoToggleRichInput {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.auto_toggle_composer",
        description: "Whether CLI agent Rich Input automatically closes and reopens based on the agent's blocked state.",
    }

    // When enabled and a CLI agent session has a plugin listener, rich input
    // auto-opens once when the session starts or when the listener is registered.
    auto_open_rich_input_on_cli_agent_start: AutoOpenRichInputOnCLIAgentStart {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.auto_open_composer_on_cli_agent_start",
        description: "Whether CLI agent Rich Input automatically opens when a CLI agent session starts.",
    }

    // When enabled and a CLI agent session does NOT have a plugin listener,
    // rich input auto-closes after the user submits a prompt.
    // When the plugin IS present, this setting has no effect (auto-show/hide
    // from auto_toggle_rich_input handles rich input lifecycle).
    auto_dismiss_rich_input_after_submit: AutoDismissRichInputAfterSubmit {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.auto_dismiss_composer_after_submit",
        description: "Whether CLI agent Rich Input automatically closes after the user submits a prompt.",
    }

    // When enabled, the Rich Input editor submits on Ctrl+Enter instead of Enter.
    // Enter inserts a newline; Ctrl+Enter submits.
    submit_on_ctrl_enter: SubmitRichInputOnCtrlEnter {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.submit_on_ctrl_enter",
        description: "When enabled, the Rich Input editor submits on Ctrl+Enter instead of Enter. Enter inserts a newline.",
    }

    // Maps custom toolbar command regex patterns to specific CLI agents.
    // Keys are regex patterns matched against the full command string.
    // Values are serialized CLIAgent names (empty string = any agent).
    // Supports migration from the legacy Vec<String> format.
    cli_agent_footer_enabled_commands: CLIAgentToolbarEnabledCommands {
        type: ToolbarCommandMap,
        default: ToolbarCommandMap::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.cli_agent_toolbar_enabled_commands",
        max_table_depth: 1,
        description: "Maps custom toolbar command patterns to specific CLI agents.",
    }

    // This is not a user-visible setting - it tracks whether a paid user has dismissed the
    // agent management help page by clicking "View Agents".
    //
    // When false and user is on a paid plan, the help page is shown.
    // When true, the help page is hidden (user dismissed it).
    // Free users never see the help page by default regardless of this setting.
    did_dismiss_cloud_setup_guide: DidDismissAgentManagementHelpPage {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }

    // This is not a user-visible setting - it tracks whether the FTU model picker callout
    // has been shown to the user. We set this to `true` as soon as the callout is first
    // displayed (not when it's dismissed), so it never re-appears.
    //
    // Note: this setting was originally named "dismissed" but we now use it to mean "shown".
    // We kept the same setting key so that users who already dismissed the callout on an
    // older client don't see it again.
    ftu_model_callout_dismissed: FtuModelCalloutDismissed {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }

    // Whether the ambient agent trial widget has been dismissed by the user.
    //
    // Not a user-visible setting - we model it as a setting so we can track state.
    ambient_agent_trial_widget_dismissed: AmbientAgentTrialWidgetDismissed {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }

    // The raw stored default mode for new sessions. Use `default_session_mode()` to retrieve the
    // effective value, which is gated on AI availability.
    default_session_mode_internal: DefaultSessionMode,

    // The file path of the tab config used when default_session_mode_internal is TabConfig.
    // Only read when mode is TabConfig; ignored for all other modes.
    // Machine-local (tab config paths vary per machine), so never synced to cloud.
    default_tab_config_path: DefaultTabConfigPath {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "general.default_tab_config_path",
    }

    // Whether computer use is enabled for cloud agent conversations started from the Warp app.
    // This setting is only used when the AI autonomy setting is AlwaysAsk or not set.
    cloud_agent_computer_use_enabled: CloudAgentComputerUseEnabled {
        type: bool,
        default: warp_core::channel::ChannelState::channel().is_dogfood(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.other.cloud_agent_computer_use_enabled",
        description: "Whether computer use is enabled for cloud agent conversations.",
    }


    // Whether file-based MCP servers from third-party AI tools (e.g. Claude, Codex) should
    // be automatically detected and spawned. Warp-native config files (.warp/.mcp.json) are
    // always detected and spawned, regardless of this setting.
    file_based_mcp_enabled: FileBasedMcpEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.mcp_servers.file_based_mcp_enabled",
        description: "Whether third-party file-based MCP servers are automatically detected.",
    }

    // Controls how agent thinking/reasoning traces are displayed.
    thinking_display_mode: ThinkingDisplayMode,

    // Controls how orchestration message bodies are expanded by default.
    orchestration_message_display_mode: OrchestrationMessageDisplayMode,

    // Default behavior when the user submits a new prompt while the agent is still
    // responding. Per-conversation overrides live on `QueuedQueryModel`; this
    // setting is the fallback used when a conversation has no explicit override.
    default_prompt_submission_mode: PromptSubmissionMode,

    // Whether agent-executed shell commands should be included in command history
    // (up-arrow, Ctrl-R search, inline history menu).
    // When false, commands run by the AI agent are excluded from history.
    include_agent_commands_in_history: IncludeAgentCommandsInHistory {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.input.include_agent_commands_in_history",
        description: "Whether agent-executed commands are included in command history.",
    }

    // Controls whether the conversation history view appears in the tools panel.
    show_conversation_history: ShowConversationHistory {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.other.show_conversation_history",
        description: "Whether conversation history appears in the tools panel.",
    }


    // Controls whether agent notifications (mailbox button, toasts, notification items) are shown.
    show_agent_notifications: ShowAgentNotifications {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.other.show_agent_notifications",
        description: "Whether agent notifications are shown.",
    }

    // Per-agent, per-host tracking of whether the user dismissed the plugin install chip.
    // Keys are "<agent_prefix>" for local sessions or "<agent_prefix>@<host>" for remote.
    // Local-only so dismissal doesn't sync across devices.
    plugin_install_chip_dismissed_map: PluginInstallChipDismissedMap {
        type: HashMap<String, bool>,
        default: HashMap::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    }

    // Per-agent, per-host tracking of the MINIMUM_PLUGIN_VERSION for which the user
    // dismissed the plugin update chip. Empty/absent means not dismissed.
    // Keys are "<agent_prefix>" for local sessions or "<agent_prefix>@<host>" for remote.
    // Local-only so dismissal doesn't sync across devices.
    plugin_update_chip_dismissed_for_version_map: PluginUpdateChipDismissedForVersionMap {
        type: HashMap<String, String>,
        default: HashMap::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    }

    // Whether Oz should add attribution (co-author line) to commit messages and PRs.
    // This is the user-level preference; it may be overridden by the team-level
    // `enable_warp_attribution` AdminEnablementSetting (see
    // `UserWorkspaces::get_agent_attribution_setting`).
    agent_attribution_enabled: AgentAttributionEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: false,
        toml_path: "agents.warp_agent.other.agent_attribution_enabled",
        description: "Whether the Warp Agent adds an attribution co-author line to commit messages and pull requests it creates.",
    }

    should_force_disable_cloud_handoff: ShouldForceDisableCloudHandoff {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.other.should_force_disable_cloud_handoff",
        description: "Whether to force-disable local-to-cloud handoff.",
    }

    should_force_disable_ampersand_handoff: ShouldForceDisableAmpersandHandoff {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.other.should_force_disable_ampersand_handoff",
        description: "Whether to force-disable the & prefix for cloud handoff compose mode.",
    }

    auto_handoff_on_sleep_enabled: AutoHandoffOnSleepEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.other.auto_handoff_on_sleep_enabled",
        description: "Whether Warp automatically hands off local agent conversations to cloud when the computer is about to sleep.",
    }
]);

impl AISettings {
    pub fn register_and_subscribe_to_events(app: &mut AppContext) {
        Self::register(app);
        app.add_singleton_model(FocusedTerminalInfo::new);
        CompiledCommandsForCodingAgentToolbar::register(app);

        app.update_model(&Self::handle(app), |_me, ctx| {
            ctx.subscribe_to_model(&FocusedTerminalInfo::handle(ctx), |_me, event, ctx| {
                if matches!(event, FocusedTerminalInfoEvent::TerminalInfoUpdated) {
                    // Pipe the event so that any view that listens for settings changes will be notified.
                    ctx.emit(AISettingsChangedEvent::IsAnyAIEnabled {
                        change_event_reason: ChangeEventReason::LocalChange,
                    });
                }
            });
        });
    }

    pub fn is_ai_disabled_due_to_remote_session_org_policy(&self, app: &AppContext) -> bool {
        let contains_remote_blocks = FocusedTerminalInfo::as_ref(app).contains_any_remote_blocks();

        let contains_restored_remote_blocks =
            FocusedTerminalInfo::as_ref(app).contains_any_restored_remote_blocks();

        let is_ai_allowed_in_remote_sessions =
            UserWorkspaces::as_ref(app).is_ai_allowed_in_remote_sessions();

        if is_ai_allowed_in_remote_sessions {
            return false;
        }

        contains_remote_blocks || contains_restored_remote_blocks
    }

    pub fn is_any_ai_enabled(&self, app: &AppContext) -> bool {
        // Disable AI for anonymous and logged-out users.
        let is_anonymous_or_logged_out = AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out();

        *self.is_any_ai_enabled
            && !is_anonymous_or_logged_out
            && !self.is_ai_disabled_due_to_remote_session_org_policy(app)
    }

    pub fn default_session_mode(&self, app: &AppContext) -> DefaultSessionMode {
        let mode = *self.default_session_mode_internal.value();
        match mode {
            // Terminal and TabConfig don't require AI.
            DefaultSessionMode::Terminal | DefaultSessionMode::TabConfig => mode,
            // Agent and CloudAgent require AI to be enabled.
            DefaultSessionMode::Agent | DefaultSessionMode::CloudAgent => {
                if self.is_any_ai_enabled(app) {
                    mode
                } else {
                    DefaultSessionMode::Terminal
                }
            }
            // DockerSandbox is gated on its feature flag; fall back to Terminal
            // when disabled so a stale stored value doesn't wedge the user.
            DefaultSessionMode::DockerSandbox => {
                if FeatureFlag::LocalDockerSandbox.is_enabled() {
                    mode
                } else {
                    DefaultSessionMode::Terminal
                }
            }
        }
    }

    /// Returns the stored default tab config path (only meaningful when mode is `TabConfig`).
    pub fn default_tab_config_path(&self) -> &str {
        &self.default_tab_config_path
    }

    /// Looks up the `TabConfig` matching the stored `default_tab_config_path`.
    /// Returns `None` if the path is empty or no loaded config matches.
    pub fn resolved_default_tab_config(
        &self,
        app: &AppContext,
    ) -> Option<crate::tab_configs::TabConfig> {
        let path_str = self.default_tab_config_path.as_str();
        if path_str.is_empty() {
            return None;
        }
        let path = std::path::Path::new(path_str);
        crate::user_config::WarpConfig::as_ref(app)
            .tab_configs()
            .iter()
            .find(|config| config.source_path.as_deref().is_some_and(|p| p == path))
            .cloned()
    }

    pub fn is_active_ai_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_any_ai_enabled(app)
            && *self.is_active_ai_enabled_internal
            && AppExecutionMode::as_ref(app).allows_active_ai()
    }

    pub fn is_prompt_suggestions_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_active_ai_enabled(app) && *self.prompt_suggestions_enabled_internal
    }

    pub fn is_rule_suggestions_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_active_ai_enabled(app) && *self.rule_suggestions_enabled_internal
    }

    pub fn is_code_suggestions_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_active_ai_enabled(app) && *self.code_suggestions_enabled_internal
    }

    pub fn is_natural_language_autosuggestions_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_active_ai_enabled(app) && *self.natural_language_autosuggestions_enabled_internal
    }

    pub fn is_shared_block_title_generation_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_active_ai_enabled(app) && *self.shared_block_title_generation_enabled_internal
    }

    pub fn is_git_operations_autogen_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_active_ai_enabled(app) && *self.git_operations_autogen_enabled_internal
    }

    pub fn is_intelligent_autosuggestions_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_active_ai_enabled(app) && *self.intelligent_autosuggestions_enabled_internal
    }

    pub fn is_voice_input_enabled(&self, app: &warpui::AppContext) -> bool {
        // Voice input is conditionally-compiled because it requires additional dependencies on some platforms.
        cfg!(feature = "voice_input")
            && self.is_any_ai_enabled(app)
            && *self.voice_input_enabled_internal
    }

    /// Returns `true` if input autodetection is enabled.
    ///
    /// If `FeatureFlag::AgentView` is enabled, this specifically gates NLD enablement in the agent
    /// view only.
    pub fn is_ai_autodetection_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_any_ai_enabled(app) && *self.ai_autodetection_enabled_internal
    }

    /// Returns `true` if NLD is enabled in the terminal.
    ///
    /// This is only used when `FeatureFlag::AgentView` is enabled.
    /// If the user has not explicitly set this setting, it defaults to the value of
    /// `ai_autodetection_enabled_internal`.
    pub fn is_nld_in_terminal_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_any_ai_enabled(app) && *self.nld_in_terminal_enabled_internal
    }

    pub fn is_memory_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_any_ai_enabled(app) && *self.memory_enabled
    }

    pub fn is_warp_drive_context_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_any_ai_enabled(app) && *self.warp_drive_context_enabled
    }

    pub fn is_file_based_mcp_enabled(&self, app: &warpui::AppContext) -> bool {
        if !FeatureFlag::FileBasedMcp.is_enabled() || !self.is_any_ai_enabled(app) {
            return false;
        }
        // NOTE: we intentionally do not force-enable this in Cloud Mode. Previously
        // we auto-spawned file-based MCPs in autonomous execution, but that bypassed
        // the user's explicit opt-in and let any MCP config checked into a repo run
        // arbitrary commands as part of a cloud agent run. Respecting the toggle
        // closes that attack surface; cloud agents that need project-scoped MCP
        // servers should surface an explicit, auditable opt-in. A more robust
        // solution (e.g. per-environment allowlisting, signed configs) should be
        // explored in the future.
        *self.file_based_mcp_enabled
    }

    pub fn is_orchestration_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_any_ai_enabled(app)
    }

    /// Returns true when local-to-cloud handoff is effectively enabled.
    /// False when the user/org has disabled it, cloud conversations are off,
    /// or AI is globally off.
    pub fn is_cloud_handoff_enabled(&self, app: &warpui::AppContext) -> bool {
        if !self.is_any_ai_enabled(app) || *self.should_force_disable_cloud_handoff {
            return false;
        }
        if !FeatureFlag::OzHandoff.is_enabled()
            || !FeatureFlag::HandoffLocalCloud.is_enabled()
            || !cfg!(all(feature = "local_fs", not(target_family = "wasm")))
        {
            return false;
        }
        let privacy = PrivacySettings::as_ref(app);
        if !privacy.is_cloud_conversation_storage_enabled {
            return false;
        }
        !matches!(
            UserWorkspaces::as_ref(app).get_cloud_conversation_storage_enablement_setting(),
            crate::workspaces::workspace::AdminEnablementSetting::Disable
        )
    }
    pub fn is_ampersand_handoff_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_cloud_handoff_enabled(app) && !*self.should_force_disable_ampersand_handoff
    }

    pub fn is_auto_handoff_on_sleep_enabled(&self, app: &warpui::AppContext) -> bool {
        self.is_cloud_handoff_enabled(app)
            && self
                .auto_handoff_on_sleep_enabled
                .is_supported_on_current_platform()
            && *self.auto_handoff_on_sleep_enabled
    }

    /// Determines whether a quota reset banner should be displayed to the user.
    ///
    /// The banner should be shown if the most recent completed billing cycle had
    /// quota exceeded and the banner was not manually dismissed.
    pub fn should_display_quota_reset_banner(&self) -> bool {
        let quota_info = &self.ai_request_quota_info;

        let most_recent_completed_cycle = quota_info
            .cycle_history
            .iter()
            .rev()
            .find(|cycle| cycle.end_date < Utc::now());

        if let Some(cycle) = most_recent_completed_cycle {
            if cycle.was_quota_exceeded && !cycle.banner_state.dismissed {
                return true;
            }
        }

        false
    }

    /// Marks the banner as dismissed for all completed cycles.
    pub fn mark_quota_banner_as_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        let mut cycle_history = self.ai_request_quota_info.cycle_history.clone();

        for cycle in cycle_history.iter_mut() {
            if cycle.end_date < Utc::now() {
                cycle.banner_state.dismissed = true;
            }
        }

        report_if_error!(self
            .ai_request_quota_info
            .set_value(AIRequestQuotaInfo { cycle_history }, ctx));
    }

    /// Updates the quota info based on the latest RequestLimitInfo.
    ///
    /// This method finds or creates the appropriate CycleInfo based on the
    /// request_limit_info's next refresh time and updates its fields accordingly.
    pub fn update_quota_info(
        &mut self,
        request_limit_info: &RequestLimitInfo,
        ctx: &mut ModelContext<Self>,
    ) {
        // Convert ServerTimestamp to DateTime<Utc>
        let next_refresh_time = request_limit_info.next_refresh_time.utc();
        let now = Utc::now();

        // Check if request_limit_info has unlimited requests
        let is_quota_exceeded = !request_limit_info.is_unlimited
            && request_limit_info.num_requests_used_since_refresh >= request_limit_info.limit;

        let mut cycle_history = self.ai_request_quota_info.cycle_history.clone();

        // Track if we updated an existing cycle
        let mut updated_existing_cycle = false;

        // Find or create a cycle that matches the current period
        if let Some(current_cycle) = cycle_history.last_mut() {
            if now <= current_cycle.end_date {
                // Update existing cycle
                current_cycle.was_quota_exceeded = is_quota_exceeded;
                updated_existing_cycle = true;
            }
        }

        // Only create a new cycle if we didn't update an existing one
        if !updated_existing_cycle {
            // Create a new cycle
            let new_cycle = CycleInfo {
                end_date: next_refresh_time,
                was_quota_exceeded: is_quota_exceeded,
                banner_state: BannerState::default(),
            };

            cycle_history.push(new_cycle);
        }

        report_if_error!(self
            .ai_request_quota_info
            .set_value(AIRequestQuotaInfo { cycle_history }, ctx));
    }

    pub fn is_command_denylist_editable(&self, app: &AppContext) -> bool {
        self.is_any_ai_enabled(app)
    }

    pub fn is_command_allowlist_editable(&self, app: &AppContext) -> bool {
        let set_by_workspace = UserWorkspaces::as_ref(app)
            .ai_autonomy_settings()
            .has_override_for_execute_commands_allowlist();

        self.is_any_ai_enabled(app) && !set_by_workspace
    }

    pub fn is_directory_allowlist_editable(&self, app: &AppContext) -> bool {
        let set_by_workspace = UserWorkspaces::as_ref(app)
            .ai_autonomy_settings()
            .has_override_for_read_files_allowlist();

        self.is_any_ai_enabled(app) && !set_by_workspace
    }

    pub fn is_execute_commands_permissions_editable(&self, app: &AppContext) -> bool {
        let set_by_workspace = UserWorkspaces::as_ref(app)
            .ai_autonomy_settings()
            .has_override_for_execute_commands();

        self.is_any_ai_enabled(app) && !set_by_workspace
    }

    pub fn is_write_to_pty_permissions_editable(&self, app: &AppContext) -> bool {
        let set_by_workspace = UserWorkspaces::as_ref(app)
            .ai_autonomy_settings()
            .has_override_for_write_to_pty();
        self.is_any_ai_enabled(app) && !set_by_workspace
    }

    pub fn is_computer_use_permissions_editable(&self, app: &AppContext) -> bool {
        let set_by_workspace = UserWorkspaces::as_ref(app)
            .ai_autonomy_settings()
            .has_override_for_computer_use();
        self.is_any_ai_enabled(app) && !set_by_workspace
    }

    pub fn is_read_files_permissions_editable(&self, app: &AppContext) -> bool {
        let set_by_workspace = UserWorkspaces::as_ref(app)
            .ai_autonomy_settings()
            .has_override_for_read_files();

        self.is_any_ai_enabled(app) && !set_by_workspace
    }

    pub fn is_code_diffs_permissions_editable(&self, app: &AppContext) -> bool {
        let set_by_workspace = UserWorkspaces::as_ref(app)
            .ai_autonomy_settings()
            .has_override_for_code_diffs();

        self.is_any_ai_enabled(app) && !set_by_workspace
    }

    pub fn is_ask_user_question_permissions_editable(&self, app: &AppContext) -> bool {
        self.is_any_ai_enabled(app)
    }

    pub fn is_mcp_permission_editable(&self, app: &AppContext) -> bool {
        // TODO: Allow workspace overrides on MCP permissions.
        self.is_any_ai_enabled(app)
    }

    pub fn is_run_agents_permissions_editable(&self, app: &AppContext) -> bool {
        self.is_orchestration_enabled(app)
    }

    pub fn show_code_suggestion_speedbump(&self, app: &AppContext) -> bool {
        self.is_any_ai_enabled(app) && *self.show_code_suggestion_speedbump
    }

    /// Handles first-time voice input setup when user clicks the voice button.
    ///
    /// If the user hasn't explicitly interacted with voice yet:
    /// - Sets the default voice input toggle key based on the OS
    /// - Marks `explicitly_interacted_with_voice` as true
    /// - Returns `Some(toggle_key)` so the caller can show a toast
    ///
    /// If the user has already interacted with voice, returns `None`.
    pub fn maybe_setup_first_time_voice(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<VoiceInputToggleKey> {
        if *self.explicitly_interacted_with_voice.value() {
            return None;
        }

        let voice_input_toggle_key = match OperatingSystem::get() {
            OperatingSystem::Mac => VoiceInputToggleKey::Fn,
            OperatingSystem::Windows | OperatingSystem::Linux | OperatingSystem::Other(_) => {
                VoiceInputToggleKey::AltRight
            }
        };

        report_if_error!(self
            .voice_input_toggle_key
            .set_value(voice_input_toggle_key, ctx));

        report_if_error!(self.explicitly_interacted_with_voice.set_value(true, ctx));

        Some(voice_input_toggle_key)
    }

    pub fn add_cli_agent_footer_enabled_command(
        &mut self,
        command: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let command = command.trim();
        if command.is_empty() {
            return;
        }
        if self
            .cli_agent_footer_enabled_commands
            .value()
            .contains_key(command)
        {
            return;
        }

        let mut map = self.cli_agent_footer_enabled_commands.value().0.clone();
        map.insert(command.to_string(), String::new());
        report_if_error!(self
            .cli_agent_footer_enabled_commands
            .set_value(ToolbarCommandMap::new(map), ctx));
    }

    pub fn remove_cli_agent_footer_enabled_command(
        &mut self,
        command: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let command = command.trim();
        let mut map = self.cli_agent_footer_enabled_commands.value().0.clone();
        map.shift_remove(command);
        report_if_error!(self
            .cli_agent_footer_enabled_commands
            .set_value(ToolbarCommandMap::new(map), ctx));
    }

    pub fn set_cli_agent_for_command(
        &mut self,
        pattern: &str,
        agent: Option<CLIAgent>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut map = self.cli_agent_footer_enabled_commands.value().0.clone();
        if !map.contains_key(pattern) {
            return;
        }
        let value = agent.map(|a| a.to_serialized_name()).unwrap_or_default();
        map.insert(pattern.to_string(), value);
        report_if_error!(self
            .cli_agent_footer_enabled_commands
            .set_value(ToolbarCommandMap::new(map), ctx));
    }

    /// Whether the plugin install chip was dismissed for the given agent/host.
    pub fn is_plugin_install_chip_dismissed(&self, key: &str) -> bool {
        self.plugin_install_chip_dismissed_map
            .get(key)
            .copied()
            .unwrap_or(false)
    }

    /// Mark the plugin install chip as dismissed for the given agent/host.
    pub fn dismiss_plugin_install_chip(&mut self, key: &str, ctx: &mut ModelContext<Self>) {
        let mut map = self.plugin_install_chip_dismissed_map.clone();
        map.insert(key.to_owned(), true);
        report_if_error!(self.plugin_install_chip_dismissed_map.set_value(map, ctx));
    }

    /// Returns the minimum plugin version for which the update chip was dismissed
    /// for the given agent/host, or an empty string if not dismissed.
    pub fn plugin_update_chip_dismissed_version(&self, key: &str) -> &str {
        self.plugin_update_chip_dismissed_for_version_map
            .get(key)
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Record that the user dismissed the update chip for the given agent/host at
    /// the specified minimum version.
    pub fn dismiss_plugin_update_chip(
        &mut self,
        key: &str,
        version: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut map = self.plugin_update_chip_dismissed_for_version_map.clone();
        map.insert(key.to_owned(), version);
        report_if_error!(self
            .plugin_update_chip_dismissed_for_version_map
            .set_value(map, ctx));
    }
}

/// Singleton model that caches compiled regexes for the `cli_agent_footer_enabled_commands`
/// setting. Each entry pairs a compiled regex with the CLI agent it maps to.
pub struct CompiledCommandsForCodingAgentToolbar {
    regexes: Vec<(Regex, CLIAgent)>,
}

impl CompiledCommandsForCodingAgentToolbar {
    fn parse(app: &AppContext) -> Vec<(Regex, CLIAgent)> {
        AISettings::as_ref(app)
            .cli_agent_footer_enabled_commands
            .value()
            .iter()
            .filter_map(|(pattern, agent_name)| {
                let regex = Regex::new(pattern).ok()?;
                let agent = CLIAgent::from_serialized_name(agent_name);
                Some((regex, agent))
            })
            .collect()
    }

    fn register(app: &mut AppContext) {
        let handle = app.add_singleton_model(|ctx| Self {
            regexes: Self::parse(ctx),
        });
        let ai_settings = AISettings::handle(app);
        app.subscribe_to_model(&ai_settings, move |_, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::CLIAgentToolbarEnabledCommands { .. }
            ) {
                let regexes = Self::parse(ctx);
                handle.update(ctx, |me, _| {
                    me.regexes = regexes;
                });
            }
        });
    }

    /// Returns the CLI agent assigned to the first matching pattern, or `None`
    /// if no pattern matches the command.
    pub fn matched_agent(app: &AppContext, command: &str) -> Option<CLIAgent> {
        Self::as_ref(app)
            .regexes
            .iter()
            .find(|(regex, _)| regex.is_match(command))
            .map(|(_, agent)| *agent)
    }
}

impl Entity for CompiledCommandsForCodingAgentToolbar {
    type Event = ();
}

impl SingletonEntity for CompiledCommandsForCodingAgentToolbar {}

#[cfg(test)]
#[path = "ai_tests.rs"]
mod tests;
