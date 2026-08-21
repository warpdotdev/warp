use warp_core::execution_mode::ExecutionMode;

use super::can_spawn_cli_mcp_server;

#[test]
fn explicit_execution_path_can_always_spawn() {
    assert!(can_spawn_cli_mcp_server(Some("/usr/bin"), false));
    assert!(can_spawn_cli_mcp_server(Some("/usr/bin"), true));
}

#[test]
fn fresh_sdk_process_without_execution_path_inherits_process_path() {
    let can_inherit = ExecutionMode::Sdk.can_inherit_process_path_for_mcp();
    assert!(can_spawn_cli_mcp_server(None, can_inherit));
}

#[test]
fn tui_without_execution_path_inherits_process_path() {
    let can_inherit = ExecutionMode::Tui.can_inherit_process_path_for_mcp();
    assert!(can_spawn_cli_mcp_server(None, can_inherit));
}

#[test]
fn desktop_app_without_execution_path_cannot_spawn() {
    let can_inherit = ExecutionMode::App.can_inherit_process_path_for_mcp();
    assert!(!can_spawn_cli_mcp_server(None, can_inherit));
}

#[test]
fn remote_server_daemon_without_execution_path_inherits_process_path() {
    let can_inherit = ExecutionMode::RemoteServerDaemon.can_inherit_process_path_for_mcp();
    assert!(can_spawn_cli_mcp_server(None, can_inherit));
}
