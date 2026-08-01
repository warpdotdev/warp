use super::ExecutionMode;

#[test]
fn execution_modes_report_distinct_agent_and_sdk_client_ids() {
    assert_eq!(ExecutionMode::Tui.client_id(), "warp-agent-cli");
    assert_eq!(ExecutionMode::Sdk.client_id(), "warp-cli");
}
