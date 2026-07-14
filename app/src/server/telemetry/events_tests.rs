use warp_core::telemetry::{EnablementState, TelemetryEventDesc};

use super::TelemetryEventDiscriminants;
use crate::channel::Channel;

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

/// Emits `AgentMode.NaturalLanguageDetection.InputBufferSubmitted` only in dogfood channels for
/// cost control. Remove this test if the event is re-enabled across channels (e.g. a new NLD change).
#[test]
fn input_buffer_submitted_only_emits_on_dogfood_channels() {
    let EnablementState::ChannelSpecific { channels } =
        TelemetryEventDiscriminants::InputBufferSubmitted.enablement_state()
    else {
        panic!("InputBufferSubmitted should be gated to specific channels, not Always/Flag");
    };

    for channel in [Channel::Dev, Channel::Local] {
        assert!(
            channels.contains(&channel),
            "expected {channel:?} to emit InputBufferSubmitted"
        );
    }
    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Oss,
        Channel::Integration,
    ] {
        assert!(
            !channels.contains(&channel),
            "expected {channel:?} to NOT emit InputBufferSubmitted"
        );
    }
}
