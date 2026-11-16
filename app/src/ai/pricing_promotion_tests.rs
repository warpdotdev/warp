use serde_json::json;
use warp_core::telemetry::TelemetryEvent;

use super::{PricingPromotionState, PricingPromotionSurface, PricingPromotionTelemetryEvent};

#[test]
fn promotion_telemetry_payload_includes_surface() {
    for (event, surface) in [
        (
            PricingPromotionTelemetryEvent::Shown {
                surface: PricingPromotionSurface::AgentMessageBar,
            },
            "agent_message_bar",
        ),
        (
            PricingPromotionTelemetryEvent::Clicked {
                surface: PricingPromotionSurface::AgentMessageBar,
            },
            "agent_message_bar",
        ),
        (
            PricingPromotionTelemetryEvent::Dismissed {
                surface: PricingPromotionSurface::TerminalMessageBar,
            },
            "terminal_message_bar",
        ),
    ] {
        assert_eq!(
            event.payload(),
            Some(json!({
                "surface": surface,
            }))
        );
    }
}

#[test]
fn agent_and_terminal_dismissals_are_independent() {
    let mut state = PricingPromotionState {
        agent_dismissed: true,
        terminal_dismissed: false,
        displayed_surfaces: Default::default(),
    };
    assert!(state.is_dismissed(PricingPromotionSurface::AgentMessageBar));
    assert!(!state.is_dismissed(PricingPromotionSurface::TerminalMessageBar));

    state.terminal_dismissed = true;
    assert!(state.is_dismissed(PricingPromotionSurface::TerminalMessageBar));
    assert!(state.is_dismissed(PricingPromotionSurface::AgentMessageBar));
}
