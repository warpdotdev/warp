use serde::Serialize;
use serde_json::json;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::features::FeatureFlag;

/// Telemetry events for the computer-use video recording lifecycle.
///
/// Emitted through the same client-side RudderStack pipeline as
/// [`SkillTelemetryEvent`](crate::ai::skills::SkillTelemetryEvent) via
/// `send_telemetry_from_ctx!`. These are infrastructure signals for tracking
/// recording start/stop outcomes and upload success; they carry a
/// client-generated `recording_id` plus capture metadata only and contain no
/// user-generated content.
#[derive(Serialize, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
// `Stopped` is only built in the non-wasm stop/finalize path.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub enum RecordingTelemetryEvent {
    /// Emitted when a `StartRecording` action successfully begins capture.
    Started {
        /// Client-generated UUID identifying this recording, echoed back in the
        /// matching `Stopped` event.
        recording_id: String,
        /// Whether capture is scoped to a specific window (`"window"`) or the
        /// whole screen (`"screen"`).
        capture_target: String,
    },
    /// Emitted when a `StopRecording` action's finalization resolves, carrying
    /// the outcome and finalized recording metadata.
    Stopped {
        /// Client-generated UUID identifying this recording, matching the
        /// `Started` event.
        recording_id: String,
        /// Coarse outcome: `"success"` (artifact published), `"error"`, or
        /// `"cancelled"` (includes deliberate discards and cancellations).
        outcome: String,
        /// Finalized (post-cut) video duration in seconds, when available.
        duration_secs: Option<f64>,
        /// Finalized video size in bytes, when available.
        size_bytes: Option<i64>,
        /// Stable, machine-readable key identifying why finalization ran, mapped
        /// from `FinalizeReason` (e.g. `"agent_stopped"`, `"limit_reached"`).
        termination_reason: String,
    },
}

impl TelemetryEvent for RecordingTelemetryEvent {
    fn name(&self) -> &'static str {
        RecordingTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<serde_json::Value> {
        match self {
            RecordingTelemetryEvent::Started {
                recording_id,
                capture_target,
            } => Some(json!({
                "recording_id": recording_id,
                "capture_target": capture_target,
            })),
            RecordingTelemetryEvent::Stopped {
                recording_id,
                outcome,
                duration_secs,
                size_bytes,
                termination_reason,
            } => Some(json!({
                "recording_id": recording_id,
                "outcome": outcome,
                "duration_secs": duration_secs,
                "size_bytes": size_bytes,
                "termination_reason": termination_reason,
            })),
        }
    }

    fn description(&self) -> &'static str {
        RecordingTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        RecordingTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        // `recording_id` is a client-generated UUID and the remaining fields are
        // infrastructure metadata; none are user-generated content.
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for RecordingTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::Started => "Recording.Started",
            Self::Stopped => "Recording.Stopped",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Started => "A computer-use video recording was started",
            Self::Stopped => "A computer-use video recording was stopped and finalized",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        // Recording itself is gated behind this flag (see the start/stop
        // executors' `should_autoexecute`), so telemetry only fires when
        // recording is actually possible.
        EnablementState::Flag(FeatureFlag::VideoRecording)
    }
}

warp_core::register_telemetry_event!(RecordingTelemetryEvent);

#[cfg(test)]
#[path = "recording_telemetry_tests.rs"]
mod tests;
