use serde_json::{Value, json};
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

    /// One attempt in the harness exit escalation ladder driven by
    /// `AgentDriver::run_harness`: the initial `/exit` request, a follow-up
    /// retry (e.g. dismissing Claude's background-task confirmation), or a
    /// final force-kill of the harness's process group. Fires on every
    /// attempt, not just escalations, so the ratio of clean exits to
    /// escalations is measurable.
    ExitEscalation {
        /// CLI command prefix for the harness (e.g. `"claude"`, `"codex"`).
        harness: String,
        /// `"exit"`, `"exit_followup"`, or `"force_kill"`.
        method: &'static str,
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
            ThirdPartyHarnessTelemetryEvent::ExitEscalation { harness, method } => Some(json!({
                "harness": harness,
                "method": method,
            })),
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
            ThirdPartyHarnessTelemetryEvent::ExitEscalation { .. } => false,
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
            ThirdPartyHarnessTelemetryEventDiscriminants::ExitEscalation => {
                "AmbientAgents.ThirdPartyHarness.Exit.Escalation"
            }
        }
    }

    fn description(&self) -> &'static str {
        match self {
            ThirdPartyHarnessTelemetryEventDiscriminants::RuntimeErrorDetected => {
                "Runtime output scanner detected a known failure substring in a third-party \
                 harness block."
            }
            ThirdPartyHarnessTelemetryEventDiscriminants::ExitEscalation => {
                "One attempt (initial exit, follow-up retry, or force-kill) in the harness \
                 exit escalation ladder."
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            ThirdPartyHarnessTelemetryEventDiscriminants::RuntimeErrorDetected => {
                EnablementState::Always
            }
            ThirdPartyHarnessTelemetryEventDiscriminants::ExitEscalation => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(ThirdPartyHarnessTelemetryEvent);
