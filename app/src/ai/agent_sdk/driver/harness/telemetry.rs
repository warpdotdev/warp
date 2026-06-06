use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

/// Telemetry events emitted by the third-party harness runtime layer.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(crate) enum ThirdPartyHarnessTelemetryEvent {
    /// The runtime output scanner observed one of the harness's known
    /// failure substrings. Fires once per detection, before any suppression
    /// logic, so dashboards can compare raw trigger volume vs. detections
    /// that actually fail the run.
    RuntimeErrorDetected {
        /// CLI command prefix for the harness whose block was scanned
        /// (e.g. `"claude"`, `"codex"`).
        harness: String,
        /// The originating needle from `runtime_error_patterns` that hit.
        pattern: String,
    },
}

impl TelemetryEvent for ThirdPartyHarnessTelemetryEvent {
    fn name(&self) -> &'static str {
        ThirdPartyHarnessTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            ThirdPartyHarnessTelemetryEvent::RuntimeErrorDetected { harness, pattern } => {
                Some(json!({
                    "harness": harness,
                    "pattern": pattern,
                }))
            }
        }
    }

    fn description(&self) -> &'static str {
        ThirdPartyHarnessTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        ThirdPartyHarnessTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            ThirdPartyHarnessTelemetryEvent::RuntimeErrorDetected { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for ThirdPartyHarnessTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            ThirdPartyHarnessTelemetryEventDiscriminants::RuntimeErrorDetected => {
                "AmbientAgents.ThirdPartyHarness.RuntimeError.Detected"
            }
        }
    }

    fn description(&self) -> &'static str {
        match self {
            ThirdPartyHarnessTelemetryEventDiscriminants::RuntimeErrorDetected => {
                "Runtime output scanner detected a known failure substring in a third-party \
                 harness block."
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            ThirdPartyHarnessTelemetryEventDiscriminants::RuntimeErrorDetected => {
                EnablementState::Always
            }
        }
    }
}

warp_core::register_telemetry_event!(ThirdPartyHarnessTelemetryEvent);
