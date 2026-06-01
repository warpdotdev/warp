use super::OzRunTimelineEvent;

#[test]
fn setup_timeline_event_names_match_server_contract() {
    assert_eq!(
        OzRunTimelineEvent::AgentStarted.as_event_name(),
        "agent_started"
    );
    assert_eq!(
        OzRunTimelineEvent::WorkerContainerReady.as_event_name(),
        "worker_container_ready"
    );
}
