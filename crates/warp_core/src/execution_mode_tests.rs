use super::ExecutionMode;

#[test]
fn desktop_app_requires_an_explicit_execution_path() {
    assert!(!ExecutionMode::App.can_inherit_process_path_for_mcp());
}

#[test]
fn tui_inherits_the_process_path() {
    assert!(ExecutionMode::Tui.can_inherit_process_path_for_mcp());
}

#[test]
fn sdk_inherits_the_process_path() {
    assert!(ExecutionMode::Sdk.can_inherit_process_path_for_mcp());
}

#[test]
fn remote_server_daemon_inherits_the_process_path() {
    assert!(ExecutionMode::RemoteServerDaemon.can_inherit_process_path_for_mcp());
}
