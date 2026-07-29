use std::path::PathBuf;

use uuid::Uuid;
use warp::tui_export::{
    TuiMcpAction, TuiMcpConfigState, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerStatus,
    TuiMcpSnapshot, TuiMcpTransport,
};

use super::{menu_rows, row_is_selectable};

fn server(
    id: u64,
    name: &str,
    status: TuiMcpServerStatus,
    can_log_out: bool,
) -> TuiMcpServerSnapshot {
    TuiMcpServerSnapshot {
        id: TuiMcpServerId(id),
        installation_uuid: Uuid::from_u128(u128::from(id)),
        name: name.to_owned(),
        transport: TuiMcpTransport::Stdio,
        status,
        tool_count: 2,
        resource_count: 0,
        can_log_out,
        authorization_url: None,
    }
}

fn snapshot(servers: Vec<TuiMcpServerSnapshot>) -> TuiMcpSnapshot {
    TuiMcpSnapshot {
        config_path: PathBuf::from("/tmp/mcp.json"),
        config_state: TuiMcpConfigState::Ready,
        servers,
    }
}

#[test]
fn query_filters_server_rows_case_insensitively() {
    let snapshot = snapshot(vec![
        server(1, "GitHub", TuiMcpServerStatus::Running, true),
        server(2, "Linear", TuiMcpServerStatus::Offline, false),
    ]);

    let rows = menu_rows(&snapshot, "hub");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "GitHub");
    assert_eq!(
        rows[0].primary_action,
        Some(TuiMcpAction::Stop(TuiMcpServerId(1)))
    );
}

#[test]
fn logout_capability_adds_a_secondary_action_without_adding_a_logout_row() {
    let snapshot = snapshot(vec![
        server(1, "GitHub", TuiMcpServerStatus::Running, true),
        server(2, "Linear", TuiMcpServerStatus::Offline, false),
    ]);

    let rows = menu_rows(&snapshot, "");

    assert_eq!(
        rows.len(),
        2,
        "each MCP server should render exactly one row"
    );
    assert_eq!(
        rows[0].logout_action,
        Some(TuiMcpAction::LogOut(TuiMcpServerId(1)))
    );
    assert_eq!(rows[1].logout_action, None);
    assert!(rows.iter().all(|row| !row.title.starts_with("Log out ")));
}

#[test]
fn logout_capability_keeps_a_transitional_server_selectable() {
    let snapshot = snapshot(vec![server(
        1,
        "GitHub",
        TuiMcpServerStatus::Starting,
        true,
    )]);

    let rows = menu_rows(&snapshot, "");

    assert_eq!(rows[0].primary_action, None);
    assert!(row_is_selectable(&rows[0]));
    assert_eq!(
        rows[0].logout_action,
        Some(TuiMcpAction::LogOut(TuiMcpServerId(1)))
    );
}

#[test]
fn actionless_server_transition_remains_selectable() {
    let snapshot = snapshot(vec![server(
        1,
        "GitHub",
        TuiMcpServerStatus::Stopping,
        false,
    )]);

    let rows = menu_rows(&snapshot, "");

    assert_eq!(rows[0].primary_action, None);
    assert_eq!(rows[0].logout_action, None);
    assert!(row_is_selectable(&rows[0]));
}
