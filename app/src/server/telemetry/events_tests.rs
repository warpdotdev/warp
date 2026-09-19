use super::TelemetryEvent;
use crate::ai::agent::conversation::AIConversationId;
use warp_core::telemetry::TelemetryEventDesc;

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

#[test]
fn agent_mode_created_ai_block_payload_drops_out_of_range_duration_ms() {
    let payload = TelemetryEvent::AgentModeCreatedAIBlock {
        client_exchange_id: "client-exchange-id".to_string(),
        server_output_id: None,
        was_autodetected_ai_query: false,
        time_to_first_token_ms: Some(u128::MAX),
        time_to_last_token_ms: Some(u64::MAX as u128),
        was_user_facing_error: true,
        cancelled: false,
        conversation_id: AIConversationId::new(),
        is_udi_enabled: false,
    }
    .payload()
    .expect("event should have a payload");

    assert!(payload["time_to_first_token_ms"].is_null());
    assert_eq!(
        payload["time_to_last_token_ms"],
        serde_json::json!(u64::MAX),
    );
}
