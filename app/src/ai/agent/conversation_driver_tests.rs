use super::ConversationDriver;

#[test]
fn native_owns_everything() {
    let driver = ConversationDriver::Native;
    assert!(driver.owns_turn_lifecycle());
    assert!(driver.executes_tool_calls_locally());
    assert!(!driver.reconstructs_inputs_from_messages());
    assert!(driver.reports_task_status());
    assert!(!driver.is_read_only_ui());
    assert!(driver.is_persisted_locally());
}

#[test]
fn shared_session_viewer_only_mirrors_the_stream() {
    let driver = ConversationDriver::SharedSessionViewer;
    assert!(!driver.owns_turn_lifecycle());
    assert!(!driver.executes_tool_calls_locally());
    assert!(driver.reconstructs_inputs_from_messages());
    assert!(!driver.reports_task_status());
    assert!(driver.is_read_only_ui());
    assert!(!driver.is_persisted_locally());
}

#[test]
fn cli_agent_transcript_is_a_read_only_vehicle() {
    let driver = ConversationDriver::CliAgentTranscript;
    assert!(!driver.owns_turn_lifecycle());
    assert!(!driver.executes_tool_calls_locally());
    assert!(!driver.reconstructs_inputs_from_messages());
    assert!(driver.reports_task_status());
    assert!(driver.is_read_only_ui());
    assert!(driver.is_persisted_locally());
}

#[test]
fn remote_child_is_driven_by_its_worker() {
    let driver = ConversationDriver::RemoteChild;
    assert!(!driver.owns_turn_lifecycle());
    assert!(!driver.executes_tool_calls_locally());
    assert!(!driver.reconstructs_inputs_from_messages());
    assert!(!driver.reports_task_status());
    assert!(!driver.is_read_only_ui());
    assert!(!driver.is_persisted_locally());
}

#[test]
fn default_driver_is_native() {
    assert_eq!(ConversationDriver::default(), ConversationDriver::Native);
}
