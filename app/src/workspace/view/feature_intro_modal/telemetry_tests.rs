use warp_core::telemetry::TelemetryEvent;

use super::FactoriesLaunchModalTelemetryEvent;

#[test]
fn factories_launch_telemetry_has_stable_names_and_no_explicit_payload() {
    let cases = [
        (
            FactoriesLaunchModalTelemetryEvent::Shown,
            "FactoriesLaunchModal.Shown",
        ),
        (
            FactoriesLaunchModalTelemetryEvent::Dismissed,
            "FactoriesLaunchModal.Dismissed",
        ),
        (
            FactoriesLaunchModalTelemetryEvent::CtaClicked,
            "FactoriesLaunchModal.CtaClicked",
        ),
    ];

    for (event, expected_name) in cases {
        assert_eq!(event.name(), expected_name);
        assert_eq!(event.payload(), None);
        assert!(!event.contains_ugc());
    }
}
