use warp_graphql::queries::managed_mcp_servers::{
    ManagedMcpOwnerScope, ManagedMcpServer, ManagedMcpStatus,
};

use super::*;

fn managed_server(status: ManagedMcpStatus) -> ManagedMcpServer {
    ManagedMcpServer {
        uid: cynic::Id::new("11111111-1111-1111-1111-111111111111"),
        display_name: "Linear".to_string(),
        owner_scope: ManagedMcpOwnerScope::Team,
        team_uid: Some(cynic::Id::new("team-uid")),
        status,
    }
}

/// Regression test for REMOTE-2474: `oz mcp list` previously only ever showed
/// local templatable installations and had no notion of a managed MCP server
/// at all. A local install must still show up, labeled `local`, with no
/// managed-only status.
#[test]
fn local_server_is_labeled_local_with_no_status() {
    let info = MCPServerInfo::local(uuid::Uuid::nil(), "My Local MCP".to_string());
    assert_eq!(info.source, MCPServerSource::Local);
    assert_eq!(info.status, None);

    let row = info.row();
    assert_eq!(row[2].content(), "local");
    assert_eq!(row[3].content(), "-");
}

/// Regression test for REMOTE-2474: a managed MCP server must now appear in
/// the listing, clearly labeled `managed` and distinct from local installs,
/// carrying its lifecycle status.
#[test]
fn managed_server_is_labeled_managed_with_status() {
    let info = MCPServerInfo::managed(managed_server(ManagedMcpStatus::Active));
    assert_eq!(info.source, MCPServerSource::Managed);
    assert_eq!(info.status, Some("active"));
    assert_eq!(info.uuid, "11111111-1111-1111-1111-111111111111");
    assert_eq!(info.name, "Linear");

    let row = info.row();
    assert_eq!(row[2].content(), "managed");
    assert_eq!(row[3].content(), "active");
}

#[test]
fn managed_server_status_is_rendered_for_every_lifecycle_state() {
    assert_eq!(
        MCPServerInfo::managed(managed_server(ManagedMcpStatus::Draft)).status,
        Some("draft")
    );
    assert_eq!(
        MCPServerInfo::managed(managed_server(ManagedMcpStatus::Active)).status,
        Some("active")
    );
    assert_eq!(
        MCPServerInfo::managed(managed_server(ManagedMcpStatus::Error)).status,
        Some("error")
    );
}

/// The header must carry a column that distinguishes local installs from
/// managed installs, since both are now merged into one listing.
#[test]
fn header_includes_source_and_status_columns() {
    let header_titles: Vec<String> = MCPServerInfo::header()
        .iter()
        .map(|cell| cell.content())
        .collect();
    assert_eq!(header_titles, vec!["UUID", "Name", "Source", "Status"]);
}
