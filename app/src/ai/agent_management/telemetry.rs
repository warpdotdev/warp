use serde::Serialize;
use serde_json::json;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::ai::agent_management::cloud_setup_guide_view::SetupGuideDocs;

/// Which setup guide workflow step the user interacted with
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupGuideStep {
    /// Quick start banner: Visit Oz
    VisitOz,
    /// Step 1: Create environment (slash command)
    CreateEnvironment,
    /// Step 1: Create environment (CLI command)
    CreateEnvironmentCli,
    /// Step 2: Create Slack integration
    CreateSlackIntegration,
    /// Step 2: Create Linear integration
    CreateLinearIntegration,
}

/// Where the item was opened from
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenedFrom {
    ManagementView,
    ConversationList,
    DetailsPanel,
}

/// Type of artifact clicked
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    Plan,
    Branch,
    PullRequest,
    File,
}

/// Type of filter changed
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterType {
    Status,
    Source,
    CreatedOn,
    Creator,
    Owner,
    Harness,
}

/// Telemetry events for the agent management view
#[derive(Serialize, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum AgentManagementTelemetryEvent {
    /// User toggled the agent management view open or closed
    ViewToggled { is_open: bool },
    /// User opened the setup guide
    OpenSetupGuide,
    /// User dismissed the setup guide
    DismissSetupGuide,
    /// User spawned a new local agent
    SpawnNewLocalAgent,
    /// User spawned a new cloud agent
    SpawnNewCloudAgent,
    /// User opened the agent type selector modal
    AgentTypeSelectorOpened,
    /// User ran a workflow step from the setup guide
    SetupGuideStepRun { step: SetupGuideStep },
    /// User copied a workflow step from the setup guide
    SetupGuideStepCopy { step: SetupGuideStep },
    /// User clicked a URL in the setup guide
    SetupGuideDocsLink { docs: SetupGuideDocs },
    /// User opened a conversation
    ConversationOpened {
        conversation_id: String,
        opened_from: OpenedFrom,
    },
    /// User opened a cloud run
    CloudRunOpened {
        task_id: String,
        opened_from: OpenedFrom,
    },
    /// User clicked an artifact button
    ArtifactClicked { artifact_type: ArtifactType },
    /// User changed a filter
    FilterChanged { filter_type: FilterType },
    /// User clicked an item details button
    DetailsViewed {
        item_id: String,
        viewed_from: OpenedFrom,
    },
    /// User copied a conversation link
    ConversationLinkCopied {
        conversation_id: String,
        copied_from: OpenedFrom,
    },
    /// User copied a session link
    SessionLinkCopied {
        task_id: String,
        copied_from: OpenedFrom,
    },
    /// User clicked an artifact in the tombstone view
    TombstoneArtifactClicked { artifact_type: ArtifactType },
    /// User clicked "Continue locally" in the tombstone
    #[cfg(not(target_family = "wasm"))]
    TombstoneContinueLocally,
    /// User clicked "Continue" in the tombstone to start a cloud follow-up.
    TombstoneContinueInCloud { task_id: String },
    /// User clicked "Continue locally" in the details panel
    #[cfg(not(target_family = "wasm"))]
    DetailsPanelContinueLocally,
    /// User invoked the /continue-locally slash command
    #[cfg(not(target_family = "wasm"))]
    SlashCommandContinueLocally,
    /// User clicked "Open in Warp" in the tombstone (wasm)
    #[cfg(target_family = "wasm")]
    TombstoneOpenInWarp,
    /// User cancelled a cloud run
    CloudRunCancelled { task_id: String },
    /// User forked a conversation
    ConversationForked { conversation_id: String },
}

impl TelemetryEvent for AgentManagementTelemetryEvent {
    fn name(&self) -> &'static str {
        AgentManagementTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<serde_json::Value> {
        match self {
            AgentManagementTelemetryEvent::ViewToggled { is_open } => {
                Some(json!({ "is_open": is_open }))
            }
            AgentManagementTelemetryEvent::OpenSetupGuide => None,
            AgentManagementTelemetryEvent::DismissSetupGuide => None,
            AgentManagementTelemetryEvent::SpawnNewLocalAgent => None,
            AgentManagementTelemetryEvent::SpawnNewCloudAgent => None,
            AgentManagementTelemetryEvent::AgentTypeSelectorOpened => None,
            AgentManagementTelemetryEvent::SetupGuideStepRun { step } => {
                Some(json!({ "step": step }))
            }
            AgentManagementTelemetryEvent::SetupGuideStepCopy { step } => {
                Some(json!({ "step": step }))
            }
            AgentManagementTelemetryEvent::SetupGuideDocsLink { docs } => {
                Some(json!({ "docs": docs }))
            }
            AgentManagementTelemetryEvent::ConversationOpened {
                conversation_id,
                opened_from,
            } => Some(json!({
                "conversation_id": conversation_id,
                "opened_from": opened_from,
            })),
            AgentManagementTelemetryEvent::CloudRunOpened {
                task_id,
                opened_from,
            } => Some(json!({
                "task_id": task_id,
                "opened_from": opened_from,
            })),
            AgentManagementTelemetryEvent::ArtifactClicked { artifact_type } => {
                Some(json!({ "artifact_type": artifact_type }))
            }
            AgentManagementTelemetryEvent::FilterChanged { filter_type } => {
                Some(json!({ "filter_type": filter_type }))
            }
            AgentManagementTelemetryEvent::DetailsViewed {
                item_id,
                viewed_from,
            } => Some(json!({
                "item_id": item_id,
                "viewed_from": viewed_from,
            })),
            AgentManagementTelemetryEvent::ConversationLinkCopied {
                conversation_id,
                copied_from,
            } => Some(json!({
                "conversation_id": conversation_id,
                "copied_from": copied_from,
            })),
            AgentManagementTelemetryEvent::SessionLinkCopied {
                task_id,
                copied_from,
            } => Some(json!({
                "task_id": task_id,
                "copied_from": copied_from,
            })),
            AgentManagementTelemetryEvent::TombstoneArtifactClicked { artifact_type } => {
                Some(json!({ "artifact_type": artifact_type }))
            }
            #[cfg(not(target_family = "wasm"))]
            AgentManagementTelemetryEvent::TombstoneContinueLocally => None,
            AgentManagementTelemetryEvent::TombstoneContinueInCloud { task_id } => Some(json!({
                "task_id": task_id,
            })),
            #[cfg(not(target_family = "wasm"))]
            AgentManagementTelemetryEvent::DetailsPanelContinueLocally => None,
            #[cfg(not(target_family = "wasm"))]
            AgentManagementTelemetryEvent::SlashCommandContinueLocally => None,
            #[cfg(target_family = "wasm")]
            AgentManagementTelemetryEvent::TombstoneOpenInWarp => None,
            AgentManagementTelemetryEvent::CloudRunCancelled { task_id } => {
                Some(json!({ "task_id": task_id }))
            }
            AgentManagementTelemetryEvent::ConversationForked { conversation_id } => {
                Some(json!({ "conversation_id": conversation_id }))
            }
        }
    }

    fn description(&self) -> &'static str {
        AgentManagementTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        AgentManagementTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for AgentManagementTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::ViewToggled => "AgentManagement.ViewToggled",
            Self::OpenSetupGuide => "AgentManagement.OpenSetupGuide",
            Self::DismissSetupGuide => "AgentManagement.DismissSetupGuide",
            Self::SpawnNewLocalAgent => "AgentManagement.SpawnNewLocalAgent",
            Self::SpawnNewCloudAgent => "AgentManagement.SpawnNewCloudAgent",
            Self::AgentTypeSelectorOpened => "AgentManagement.AgentTypeSelectorOpened",
            Self::SetupGuideStepRun => "AgentManagement.SetupGuideStepRun",
            Self::SetupGuideStepCopy => "AgentManagement.SetupGuideStepCopy",
            Self::SetupGuideDocsLink => "AgentManagement.SetupGuideDocsLink",
            Self::ConversationOpened => "AgentManagement.ConversationOpened",
            Self::CloudRunOpened => "AgentManagement.CloudRunOpened",
            Self::ArtifactClicked => "AgentManagement.ArtifactClicked",
            Self::FilterChanged => "AgentManagement.FilterChanged",
            Self::DetailsViewed => "AgentManagement.DetailsViewed",
            Self::ConversationLinkCopied => "AgentManagement.ConversationLinkCopied",
            Self::SessionLinkCopied => "AgentManagement.SessionLinkCopied",
            Self::TombstoneArtifactClicked => "AgentManagement.TombstoneArtifactClicked",
            #[cfg(not(target_family = "wasm"))]
            Self::TombstoneContinueLocally => "AgentManagement.TombstoneContinueLocally",
            Self::TombstoneContinueInCloud => "AgentManagement.TombstoneContinueInCloud",
            #[cfg(not(target_family = "wasm"))]
            Self::DetailsPanelContinueLocally => "AgentManagement.DetailsPanelContinueLocally",
            #[cfg(not(target_family = "wasm"))]
            Self::SlashCommandContinueLocally => "AgentManagement.SlashCommandContinueLocally",
            #[cfg(target_family = "wasm")]
            Self::TombstoneOpenInWarp => "AgentManagement.TombstoneOpenInWarp",
            Self::CloudRunCancelled => "AgentManagement.CloudRunCancelled",
            Self::ConversationForked => "AgentManagement.ConversationForked",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::ViewToggled => "User toggled the agent management view open or closed",
            Self::OpenSetupGuide => "User opened the ambient agent setup guide",
            Self::DismissSetupGuide => "User dismissed the ambient agent setup guide",
            Self::SpawnNewLocalAgent => "User spawned a new local agent from agent management",
            Self::SpawnNewCloudAgent => "User spawned a new cloud agent from agent management",
            Self::AgentTypeSelectorOpened => {
                "User opened the agent type selector from agent management"
            }
            Self::SetupGuideStepRun => "User ran a workflow step from the setup guide",
            Self::SetupGuideStepCopy => "User copied a workflow step from the setup guide",
            Self::SetupGuideDocsLink => "User clicked a docs URL in the setup guide",
            Self::ConversationOpened => "User opened a conversation",
            Self::CloudRunOpened => "User opened a cloud run",
            Self::ArtifactClicked => "User clicked an artifact button",
            Self::FilterChanged => "User changed a filter in the management view",
            Self::DetailsViewed => "User clicked View details",
            Self::ConversationLinkCopied => "User copied a conversation link",
            Self::SessionLinkCopied => "User copied a session link",
            Self::TombstoneArtifactClicked => "User clicked an artifact in the tombstone view",
            #[cfg(not(target_family = "wasm"))]
            Self::TombstoneContinueLocally => "User clicked Continue locally in the tombstone",
            Self::TombstoneContinueInCloud => {
                "User clicked Continue in the tombstone to start a cloud follow-up"
            }
            #[cfg(not(target_family = "wasm"))]
            Self::DetailsPanelContinueLocally => {
                "User clicked Continue locally in the details panel"
            }
            #[cfg(not(target_family = "wasm"))]
            Self::SlashCommandContinueLocally => {
                "User invoked /continue-locally to fork a cloud conversation locally"
            }
            #[cfg(target_family = "wasm")]
            Self::TombstoneOpenInWarp => "User clicked Open in Warp in the tombstone",
            Self::CloudRunCancelled => "User cancelled a cloud run",
            Self::ConversationForked => "User forked a conversation",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(AgentManagementTelemetryEvent);
