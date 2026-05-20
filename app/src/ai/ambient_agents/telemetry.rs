use crate::server::ids::ServerId;
use serde::Serialize;
use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::features::FeatureFlag;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

/// The entry point through which Cloud Mode was entered.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudModeEntryPoint {
    /// User clicked "New Cloud Agent Tab" or similar action to create a dedicated Cloud Mode tab.
    NewTab,
    /// User entered Cloud Mode from an existing local terminal session (e.g., via keyboard shortcut or command).
    LocalSession,
    /// User entered Cloud Mode through the Oz launch modal.
    OzLaunchModal,
    /// User re-entered Cloud Mode by clicking on an ambient agent entry block.
    EntryBlock,
}

/// The entry point through which a local-to-cloud handoff was initiated.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffEntryPoint {
    /// User typed `&` in the input to enter handoff compose mode.
    #[default]
    Ampersand,
    /// User used the `/handoff` slash command.
    SlashCommand,
    /// User clicked the "Hand off to cloud" chip in the footer toolbar.
    FooterChip,
}

/// Telemetry events for client interactions with cloud agents.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum CloudAgentTelemetryEvent {
    /// User entered Cloud Mode.
    EnteredCloudMode { entry_point: CloudModeEntryPoint },
    /// User opened the environment selector menu.
    EnvironmentSelectorOpened,
    /// User selected an environment from the environment selector.
    EnvironmentSelected {
        /// The ID of the selected environment, if available.
        environment_id: Option<ServerId>,
    },
    /// User opened the environment management pane from the environment selector.
    OpenedEnvironmentManagementPane,
    /// User created a new environment.
    EnvironmentCreated,
    /// User updated an existing environment.
    EnvironmentUpdated {
        /// The server ID of the updated environment, if available.
        environment_id: Option<ServerId>,
    },
    /// User deleted an environment.
    EnvironmentDeleted {
        /// The server ID of the deleted environment, if available.
        environment_id: Option<ServerId>,
    },
    /// Docker image was successfully suggested for an environment.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    ImageSuggested {
        /// The suggested Docker image string.
        image: String,
        /// Whether the user needs to create a custom image.
        needs_custom_image: bool,
    },
    /// Docker image suggestion failed.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    ImageSuggestionFailed {
        /// Error message describing why the suggestion failed.
        error: String,
    },
    /// User launched an environment setup agent from the environment form.
    LaunchedAgentFromEnvironmentForm,
    /// User started GitHub authentication from the environment form.
    GitHubAuthFromEnvironmentForm,
    /// Ambient agent failed to dispatch or encountered an error during subscription.
    DispatchFailed {
        /// Error message describing the failure.
        error: String,
    },
    /// User initiated a local-to-cloud handoff.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    HandoffInitiated {
        /// How the handoff was triggered.
        entry_point: HandoffEntryPoint,
        /// Whether the handoff forked an existing conversation.
        forked_existing_conversation: bool,
    },
}

impl TelemetryEvent for CloudAgentTelemetryEvent {
    fn name(&self) -> &'static str {
        CloudAgentTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            CloudAgentTelemetryEvent::EnteredCloudMode { entry_point } => Some(json!({
                "entry_point": entry_point,
            })),
            CloudAgentTelemetryEvent::EnvironmentSelectorOpened => None,
            CloudAgentTelemetryEvent::EnvironmentSelected { environment_id } => Some(json!({
                "environment_id": environment_id.map(|id| id.to_string()),
            })),
            CloudAgentTelemetryEvent::OpenedEnvironmentManagementPane => None,
            CloudAgentTelemetryEvent::EnvironmentCreated => None,
            CloudAgentTelemetryEvent::EnvironmentUpdated { environment_id } => Some(json!({
                "environment_id": environment_id.map(|id| id.to_string()),
            })),
            CloudAgentTelemetryEvent::EnvironmentDeleted { environment_id } => Some(json!({
                "environment_id": environment_id.map(|id| id.to_string()),
            })),
            CloudAgentTelemetryEvent::ImageSuggested {
                image,
                needs_custom_image,
            } => Some(json!({
                "image": image,
                "needs_custom_image": needs_custom_image,
            })),
            CloudAgentTelemetryEvent::ImageSuggestionFailed { error } => Some(json!({
                "error": error,
            })),
            CloudAgentTelemetryEvent::LaunchedAgentFromEnvironmentForm => None,
            CloudAgentTelemetryEvent::GitHubAuthFromEnvironmentForm => None,
            CloudAgentTelemetryEvent::DispatchFailed { error } => Some(json!({
                "error": error,
            })),
            CloudAgentTelemetryEvent::HandoffInitiated {
                entry_point,
                forked_existing_conversation,
            } => Some(json!({
                "entry_point": entry_point,
                "forked_existing_conversation": forked_existing_conversation,
            })),
        }
    }

    fn description(&self) -> &'static str {
        CloudAgentTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        CloudAgentTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for CloudAgentTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::EnteredCloudMode => "AmbientAgent.CloudMode.Entered",
            Self::EnvironmentSelectorOpened => "AmbientAgent.CloudMode.EnvironmentSelector.Opened",
            Self::EnvironmentSelected => "AmbientAgent.CloudMode.EnvironmentSelector.Selected",
            Self::OpenedEnvironmentManagementPane => "AmbientAgent.EnvironmentSettings.Opened",
            Self::EnvironmentCreated => "AmbientAgent.EnvironmentSettings.CreatedEnvironment",
            Self::EnvironmentUpdated => "AmbientAgent.EnvironmentSettings.UpdatedEnvironment",
            Self::EnvironmentDeleted => "AmbientAgent.EnvironmentSettings.DeletedEnvironment",
            Self::ImageSuggested => "AmbientAgent.EnvironmentSettings.Image.Suggested",
            Self::ImageSuggestionFailed => {
                "AmbientAgent.EnvironmentSettings.Image.SuggestionFailed"
            }
            Self::LaunchedAgentFromEnvironmentForm => {
                "AmbientAgent.CloudMode.EnvironmentSettings.LaunchedAgent"
            }
            Self::GitHubAuthFromEnvironmentForm => {
                "AmbientAgent.CloudMode.EnvironmentSettings.GitHubAuth"
            }
            Self::DispatchFailed => "AmbientAgent.DispatchFailed",
            Self::HandoffInitiated => "AmbientAgent.Handoff.Initiated",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::EnteredCloudMode => "User entered cloud agent view",
            Self::EnvironmentSelectorOpened => "User opened the environment selector menu",
            Self::EnvironmentSelected => "User selected an environment from the selector",
            Self::OpenedEnvironmentManagementPane => "User opened the environment management pane",
            Self::EnvironmentCreated => "User created a new environment",
            Self::EnvironmentUpdated => "User updated an existing environment",
            Self::EnvironmentDeleted => "User deleted an environment",
            Self::ImageSuggested => "Docker image was suggested for an environment",
            Self::ImageSuggestionFailed => "Docker image suggestion failed",
            Self::LaunchedAgentFromEnvironmentForm => {
                "User launched an environment setup agent from the environment form"
            }
            Self::GitHubAuthFromEnvironmentForm => {
                "User started GitHub authentication from the environment form"
            }
            Self::DispatchFailed => "Ambient agent failed to dispatch or encountered an error",
            Self::HandoffInitiated => "User initiated a local-to-cloud handoff",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Flag(FeatureFlag::CloudMode)
    }
}

warp_core::register_telemetry_event!(CloudAgentTelemetryEvent);
