use super::network_state_event;
use crate::windowing::winit::app::CustomEvent;

#[test]
fn unknown_and_connected_states_report_online() {
    for state in [0, 50, 60, 70] {
        assert!(
            matches!(network_state_event(state), CustomEvent::InternetConnected),
            "NetworkManager state {state} should report online"
        );
    }
}

#[test]
fn inactive_and_transitioning_states_report_offline() {
    for state in [10, 20, 30, 40] {
        assert!(
            matches!(
                network_state_event(state),
                CustomEvent::InternetDisconnected
            ),
            "NetworkManager state {state} should report offline"
        );
    }
}

#[test]
fn unrecognized_states_report_offline() {
    for state in [1, 49, 51, 71, u32::MAX] {
        assert!(
            matches!(
                network_state_event(state),
                CustomEvent::InternetDisconnected
            ),
            "Unrecognized NetworkManager state {state} should report offline"
        );
    }
}
