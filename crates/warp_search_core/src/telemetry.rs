use std::collections::HashSet;

use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::register_telemetry_event;
use warp_core::telemetry::{EnablementState, TelemetryEventDesc};

#[derive(Clone, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum TelemetryEvent {
    CommandSearchAsyncQueryCompleted {
        filters: HashSet<crate::data_source::QueryFilter>,
        error_payload: Option<Value>,
    },
}

impl warp_core::telemetry::TelemetryEvent for TelemetryEvent {
    fn name(&self) -> &'static str {
        TelemetryEventDiscriminants::from(self).name()
    }

    fn description(&self) -> &'static str {
        TelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        TelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            TelemetryEvent::CommandSearchAsyncQueryCompleted {
                filters,
                error_payload,
            } => Some(json!({ "filter": filters, "error": error_payload })),
        }
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::CommandSearchAsyncQueryCompleted { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for TelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::CommandSearchAsyncQueryCompleted => "Command Search Async Query Completed",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::CommandSearchAsyncQueryCompleted => {
                "Finished searching for a command in the background"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::CommandSearchAsyncQueryCompleted => EnablementState::Always,
        }
    }
}

register_telemetry_event!(TelemetryEvent);
