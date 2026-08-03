use warp_core::telemetry::TelemetryEvent as _;

use super::*;

#[test]
fn provider_credential_payload_contains_only_classification_metadata() {
    let event = AITelemetryEvent::ProviderCredentialChanged {
        provider: ProviderCredentialTelemetryProvider::Anthropic,
        credential_kind: ProviderCredentialTelemetryKind::PastedKey,
        action: ProviderCredentialTelemetryAction::Added,
    };

    assert_eq!(event.name(), "AI.ProviderCredential.Changed");
    assert_eq!(
        event.payload(),
        Some(json!({
            "provider": "anthropic",
            "credential_kind": "pasted_key",
            "action": "added",
        }))
    );
    assert!(!event.contains_ugc());
}
