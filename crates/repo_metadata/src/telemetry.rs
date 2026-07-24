use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::register_telemetry_event;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

#[derive(Clone, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum RepoMetadataTelemetryEvent {
    BuildTreeFailed { error: String },
}

impl TelemetryEvent for RepoMetadataTelemetryEvent {
    fn name(&self) -> &'static str {
        RepoMetadataTelemetryEventDiscriminants::from(self).name()
    }

    fn description(&self) -> &'static str {
        RepoMetadataTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        RepoMetadataTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::BuildTreeFailed { error } => Some(json!({
                "error": error
            })),
        }
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::BuildTreeFailed { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for RepoMetadataTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::BuildTreeFailed => "RepoMetadata.BuildTree.Failed",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::BuildTreeFailed => "Failed to build file tree for repo metadata",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::BuildTreeFailed => EnablementState::Always,
        }
    }
}

register_telemetry_event!(RepoMetadataTelemetryEvent);
