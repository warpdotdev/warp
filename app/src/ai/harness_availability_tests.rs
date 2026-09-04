use super::{CloudAgentStartBlocker, cloud_agent_start_blocker};

#[test]
fn team_required_takes_precedence_over_empty_harnesses() {
    assert_eq!(
        cloud_agent_start_blocker(true, false),
        Some(CloudAgentStartBlocker::TeamRequired)
    );
}

#[test]
fn teamed_user_without_enabled_harnesses_keeps_existing_blocker() {
    assert_eq!(
        cloud_agent_start_blocker(false, false),
        Some(CloudAgentStartBlocker::NoEnabledHarnesses)
    );
}

#[test]
fn teamed_user_with_enabled_harness_can_start_cloud_agent() {
    assert_eq!(cloud_agent_start_blocker(false, true), None);
}
