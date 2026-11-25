use warp_core::telemetry::{TelemetryEvent as _, TelemetryEventDesc};

use super::TelemetryEvent;

#[test]
fn retired_terminal_events_are_not_registered() {
    let events = TelemetryEvent::event_descs()
        .map(|event| event.name())
        .filter(|name| {
            matches!(
                *name,
                "Bootstrap Slow Contents"
                    | "Agent Predict"
                    | "AgentMode.ChangedInputType"
                    | "AgentMode.PotentialAutoDetectionFalsePositive"
            )
        })
        .collect::<Vec<_>>();

    assert!(events.is_empty(), "retired events: {events:?}");
}

#[test]
fn slow_bootstrap_metadata_event_is_registered() {
    assert!(TelemetryEvent::event_descs().any(|event| event.name() == "Bootstrapping Slow"));
}

#[derive(Debug)]
enum TelemetryEventPropertyError {
    // The variant data is never directly read, but it's used for error formatting if the test
    // below fails.
    EmptyName(#[expect(dead_code)] Box<dyn TelemetryEventDesc>),
    EmptyDescription(#[expect(dead_code)] Box<dyn TelemetryEventDesc>),
}

/// Checks that all telemetry events have a non-empty name and description.
///
/// The name and description are intended to be user-facing and are used to populate
/// our [exhaustive telemetry table](https://docs.warp.dev/support-and-community/privacy-and-security/privacy#exhaustive-telemetry-table).
#[test]
#[cfg(not(target_family = "wasm"))]
fn telemetry_events_have_nonempty_name_and_description() -> Result<(), TelemetryEventPropertyError>
{
    for event in warp_core::telemetry::all_events() {
        if event.name().is_empty() {
            return Err(TelemetryEventPropertyError::EmptyName(event));
        }
        if event.description().is_empty() {
            return Err(TelemetryEventPropertyError::EmptyDescription(event));
        }
    }
    Ok(())
}
