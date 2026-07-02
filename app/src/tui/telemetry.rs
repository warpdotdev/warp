//! Telemetry for the `warp-tui` background auto-updater.

use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::features::FeatureFlag;

/// Health signals for the TUI auto-updater, sent once per completed (or
/// failed) background update check. Throttled checks are not reported.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(super) enum TuiAutoupdateTelemetryEvent {
    /// A background update check completed.
    CheckCompleted {
        /// `"up_to_date"`, `"installed"`, or `"locked"`.
        outcome: &'static str,
        /// The relevant version: the running version when up to date, or the
        /// newly installed version.
        version: Option<String>,
    },
    /// A background update check failed (e.g. network or install errors).
    CheckFailed { error: String },
}

impl TelemetryEvent for TuiAutoupdateTelemetryEvent {
    fn name(&self) -> &'static str {
        TuiAutoupdateTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            TuiAutoupdateTelemetryEvent::CheckCompleted { outcome, version } => Some(json!({
                "outcome": outcome,
                "version": version,
            })),
            TuiAutoupdateTelemetryEvent::CheckFailed { error } => Some(json!({
                "error": error,
            })),
        }
    }

    fn description(&self) -> &'static str {
        TuiAutoupdateTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        TuiAutoupdateTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            TuiAutoupdateTelemetryEvent::CheckCompleted { .. } => false,
            // Error messages can embed install paths (which include the
            // user's home directory).
            TuiAutoupdateTelemetryEvent::CheckFailed { .. } => true,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for TuiAutoupdateTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            TuiAutoupdateTelemetryEventDiscriminants::CheckCompleted => {
                "TUI Autoupdate Check Completed"
            }
            TuiAutoupdateTelemetryEventDiscriminants::CheckFailed => "TUI Autoupdate Check Failed",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            TuiAutoupdateTelemetryEventDiscriminants::CheckCompleted => {
                "A warp-tui background update check completed, with its outcome"
            }
            TuiAutoupdateTelemetryEventDiscriminants::CheckFailed => {
                "A warp-tui background update check failed"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            TuiAutoupdateTelemetryEventDiscriminants::CheckCompleted
            | TuiAutoupdateTelemetryEventDiscriminants::CheckFailed => {
                EnablementState::Flag(FeatureFlag::TuiAutoupdate)
            }
        }
    }
}

warp_core::register_telemetry_event!(TuiAutoupdateTelemetryEvent);
