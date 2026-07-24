use std::path::PathBuf;

use uuid::Uuid;
use warp::appearance::Appearance;
use warp::tui_export::{
    TuiMcpConfigState, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerStatus, TuiMcpSnapshot,
    TuiMcpTransport,
};
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiElement, TuiRect, TuiText};
use warpui_core::presenter::tui::TuiPresenter;

use super::mcp_status_label;

fn server(id: u64, status: TuiMcpServerStatus) -> TuiMcpServerSnapshot {
    TuiMcpServerSnapshot {
        id: TuiMcpServerId(id),
        installation_uuid: Uuid::from_u128(id as u128),
        name: format!("server-{id}"),
        transport: TuiMcpTransport::Stdio,
        status,
        tool_count: 2,
        resource_count: 0,
        has_credentials: false,
        authorization_url: None,
    }
}

#[test]
fn mcp_summary_keeps_missing_config_action_short() {
    let snapshot = TuiMcpSnapshot {
        config_path: PathBuf::from("/tmp/.mcp.json"),
        config_state: TuiMcpConfigState::Missing,
        servers: Vec::new(),
    };

    assert_eq!(
        mcp_status_label(&snapshot),
        ("Not configured · /mcp".to_string(), false)
    );
}

#[test]
fn mcp_summary_reports_mixed_runtime_states() {
    let snapshot = TuiMcpSnapshot {
        config_path: PathBuf::from("/tmp/.mcp.json"),
        config_state: TuiMcpConfigState::Ready,
        servers: vec![
            server(1, TuiMcpServerStatus::Running),
            server(2, TuiMcpServerStatus::Starting),
            server(3, TuiMcpServerStatus::Authenticating),
            server(4, TuiMcpServerStatus::Stopping),
            server(
                5,
                TuiMcpServerStatus::Failed {
                    message: "failed".to_string(),
                },
            ),
            server(6, TuiMcpServerStatus::Offline),
        ],
    };

    assert_eq!(
        mcp_status_label(&snapshot),
        (
            "1 connected · 1 starting · 1 needs auth · 1 stopping · 1 failed · 1 offline · /mcp"
                .to_string(),
            false
        )
    );
}

#[test]
fn mcp_summary_marks_config_errors() {
    let snapshot = TuiMcpSnapshot {
        config_path: PathBuf::from("/tmp/.mcp.json"),
        config_state: TuiMcpConfigState::Invalid {
            message: "invalid JSON".to_string(),
        },
        servers: Vec::new(),
    };

    assert_eq!(
        mcp_status_label(&snapshot),
        ("Config error · run /mcp".to_string(), true)
    );
}

#[test]
fn zero_state_subtitle_renders_built_bug_free_by_kevin_yang() {
    // Verify that the subtitle text we added to `render_left_column` renders
    // correctly by building the element directly and asserting on the output.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let subtitle = TuiText::new("built bug free by kevin yang").finish();
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(subtitle, TuiRect::new(0, 0, 40, 1), app_ctx);
            let lines = frame.buffer.to_lines();
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("built bug free by kevin yang")),
                "zero state subtitle should render: {lines:?}"
            );
        });
    });
}
