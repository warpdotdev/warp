use uuid::Uuid;
use warp::tui_export::{
    TuiMcpAction, TuiMcpConfigDiagnostic, TuiMcpFileScope, TuiMcpFileSource, TuiMcpServerId,
    TuiMcpServerSnapshot, TuiMcpServerSource, TuiMcpServerStatus, TuiMcpSnapshot,
    TuiMcpSyncedTemplateProvenance, TuiMcpTransport,
};

use super::{menu_rows, row_is_selectable};

fn server(
    id: TuiMcpServerId,
    name: &str,
    source: TuiMcpServerSource,
    status: TuiMcpServerStatus,
    can_log_out: bool,
) -> TuiMcpServerSnapshot {
    TuiMcpServerSnapshot {
        id,
        installation_uuid: match id {
            TuiMcpServerId::FileBased(_) | TuiMcpServerId::Installation(_) => {
                Some(Uuid::from_u128(1))
            }
            TuiMcpServerId::SyncedTemplate(_) | TuiMcpServerId::Gallery(_) => None,
        },
        name: name.to_owned(),
        description: None,
        source,
        transport: Some(TuiMcpTransport::Stdio),
        status,
        tool_count: 2,
        resource_count: 0,
        can_log_out,
        authorization_url: None,
    }
}

fn snapshot(servers: Vec<TuiMcpServerSnapshot>) -> TuiMcpSnapshot {
    TuiMcpSnapshot {
        diagnostics: Vec::new(),
        servers,
    }
}

#[test]
fn query_filters_server_rows_case_insensitively() {
    let first = TuiMcpServerId::Installation(Uuid::from_u128(1));
    let snapshot = snapshot(vec![
        server(
            first,
            "GitHub",
            TuiMcpServerSource::Installation,
            TuiMcpServerStatus::Running,
            true,
        ),
        server(
            TuiMcpServerId::FileBased(2),
            "Linear",
            TuiMcpServerSource::FileBased {
                sources: Vec::new(),
            },
            TuiMcpServerStatus::Offline,
            false,
        ),
    ]);

    let rows = menu_rows(&snapshot, "hub");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "GitHub");
    assert_eq!(
        rows[0].description.as_deref(),
        Some("CLI local · stdio · running · 2 tools")
    );
    assert_eq!(rows[0].primary_action, Some(TuiMcpAction::Stop(first)));
}

#[test]
fn available_row_is_distinct_from_offline_and_requires_enable() {
    let id = TuiMcpServerId::Gallery(Uuid::from_u128(7));
    let snapshot = snapshot(vec![server(
        id,
        "Figma",
        TuiMcpServerSource::Gallery,
        TuiMcpServerStatus::Available,
        false,
    )]);

    let rows = menu_rows(&snapshot, "");

    assert_eq!(rows[0].primary_action, Some(TuiMcpAction::Enable(id)));
    assert_eq!(
        rows[0].description.as_deref(),
        Some("shared by Warp · stdio · available")
    );
}

#[test]
fn same_named_synced_templates_explain_their_distinct_provenance() {
    let snapshot = snapshot(vec![
        server(
            TuiMcpServerId::SyncedTemplate(Uuid::from_u128(7)),
            "Figma",
            TuiMcpServerSource::SyncedTemplate {
                provenance: TuiMcpSyncedTemplateProvenance::FromAnotherDevice,
            },
            TuiMcpServerStatus::Available,
            false,
        ),
        server(
            TuiMcpServerId::SyncedTemplate(Uuid::from_u128(8)),
            "Figma",
            TuiMcpServerSource::SyncedTemplate {
                provenance: TuiMcpSyncedTemplateProvenance::Shared {
                    creator: Some("Roland Huang".to_owned()),
                },
            },
            TuiMcpServerStatus::Available,
            false,
        ),
    ]);

    let rows = menu_rows(&snapshot, "");

    assert_eq!(
        rows.iter()
            .map(|row| row.description.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("from another device · stdio · available"),
            Some("shared by Roland Huang · stdio · available"),
        ]
    );
}

#[test]
fn file_provenance_includes_provider_scope_and_repository() {
    let snapshot = snapshot(vec![server(
        TuiMcpServerId::FileBased(8),
        "Repo MCP",
        TuiMcpServerSource::FileBased {
            sources: vec![
                TuiMcpFileSource {
                    provider: "Claude".to_owned(),
                    root_path: "/home/me".into(),
                    scope: TuiMcpFileScope::Global,
                },
                TuiMcpFileSource {
                    provider: "Other Agents".to_owned(),
                    root_path: "/work/project".into(),
                    scope: TuiMcpFileScope::Project,
                },
            ],
        },
        TuiMcpServerStatus::Offline,
        false,
    )]);

    let rows = menu_rows(&snapshot, "");

    assert_eq!(
        rows[0].description.as_deref(),
        Some("Claude global, Other Agents · project · stdio · offline")
    );
}

#[test]
fn multiple_diagnostics_render_without_hiding_healthy_servers() {
    let mut snapshot = snapshot(vec![server(
        TuiMcpServerId::FileBased(2),
        "Healthy",
        TuiMcpServerSource::FileBased {
            sources: Vec::new(),
        },
        TuiMcpServerStatus::Offline,
        false,
    )]);
    snapshot.diagnostics = vec![
        TuiMcpConfigDiagnostic {
            provider: "Claude".to_owned(),
            config_path: "/tmp/.claude.json".into(),
            message: "invalid JSON".to_owned(),
        },
        TuiMcpConfigDiagnostic {
            provider: "Codex".to_owned(),
            config_path: "/tmp/config.toml".into(),
            message: "missing variable".to_owned(),
        },
    ];

    let rows = menu_rows(&snapshot, "");

    assert_eq!(rows.len(), 3);
    assert!(!row_is_selectable(&rows[0]));
    assert!(!row_is_selectable(&rows[1]));
    assert_eq!(rows[2].title, "Healthy");
    assert!(row_is_selectable(&rows[2]));
}

#[test]
fn logout_capability_adds_a_secondary_action_without_adding_a_logout_row() {
    let first = TuiMcpServerId::Installation(Uuid::from_u128(1));
    let snapshot = snapshot(vec![
        server(
            first,
            "GitHub",
            TuiMcpServerSource::Installation,
            TuiMcpServerStatus::Running,
            true,
        ),
        server(
            TuiMcpServerId::FileBased(2),
            "Linear",
            TuiMcpServerSource::FileBased {
                sources: Vec::new(),
            },
            TuiMcpServerStatus::Offline,
            false,
        ),
    ]);

    let rows = menu_rows(&snapshot, "");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].logout_action, Some(TuiMcpAction::LogOut(first)));
    assert_eq!(rows[1].logout_action, None);
}

#[test]
fn actionless_server_transition_remains_selectable() {
    let snapshot = snapshot(vec![server(
        TuiMcpServerId::FileBased(1),
        "GitHub",
        TuiMcpServerSource::FileBased {
            sources: Vec::new(),
        },
        TuiMcpServerStatus::Stopping,
        false,
    )]);

    let rows = menu_rows(&snapshot, "");

    assert_eq!(rows[0].primary_action, None);
    assert_eq!(rows[0].logout_action, None);
    assert!(row_is_selectable(&rows[0]));
}
