use std::path::PathBuf;

use uuid::Uuid;
use warp::tui_export::{
    TuiMcpConfigState, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerStatus, TuiMcpSnapshot,
    TuiMcpTransport,
};
use warpui::EntityIdMap;
use warpui_core::App;
use warpui_core::elements::tui::{
    TuiConstrainedBox, TuiConstraint, TuiElement, TuiFlex, TuiLayoutContext, TuiSize, TuiText,
};

use super::{LEFT_COLUMN_COLS, mcp_status_label};

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

// ---------------------------------------------------------------------------
// Layout tests for the path-header fix (APP-5009)
//
// These tests construct the same outer-column structure that
// `TuiZeroStateView::render` builds and assert on layout widths. They use the
// element library directly (no full singleton setup required) so they run fast
// without a TUI window or view presenter.
// ---------------------------------------------------------------------------

/// Helper: lay out `element` inside a loose constraint of `(w, h)` columns/rows
/// and return the resulting `TuiSize`.
fn layout_at(
    element: &mut dyn TuiElement,
    w: u16,
    h: u16,
    app: &warpui_core::AppContext,
) -> TuiSize {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.layout(TuiConstraint::loose(TuiSize::new(w, h)), &mut ctx, app)
}

/// The project path header must NOT be capped at `LEFT_COLUMN_COLS` when the
/// terminal is wider — that was the bug reported in APP-5009.
#[test]
fn path_header_outside_constrained_box_uses_full_terminal_width() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // A path longer than LEFT_COLUMN_COLS.
            let long_path =
                "~/work/projects/my-organisation/very-long-repository-name-that-exceeds-48-cols";
            assert!(
                long_path.len() as u16 > LEFT_COLUMN_COLS,
                "test path must exceed LEFT_COLUMN_COLS"
            );

            // Simulate the overlay column the new render() builds:
            //   constrained_top (48 cols max) + blank + path_header (no cap) + constrained_bottom
            let constrained_top = TuiConstrainedBox::new(TuiText::new("Warp Agent CLI").finish())
                .with_min_cols(LEFT_COLUMN_COLS)
                .with_max_cols(LEFT_COLUMN_COLS);
            let path_header = TuiText::new(long_path).truncate();
            let constrained_bottom =
                TuiConstrainedBox::new(TuiText::new("Not configured · /mcp").finish())
                    .with_min_cols(LEFT_COLUMN_COLS)
                    .with_max_cols(LEFT_COLUMN_COLS);

            let mut outer = TuiFlex::column()
                .child(constrained_top.finish())
                .child(path_header.finish())
                .child(constrained_bottom.finish());

            let size = layout_at(&mut outer, 200, 10, app_ctx);

            assert!(
                size.width > LEFT_COLUMN_COLS,
                "outer column should be wider than LEFT_COLUMN_COLS ({LEFT_COLUMN_COLS}) \
                 when the path header exceeds it; got width = {}",
                size.width
            );
            assert_eq!(
                size.width,
                long_path.len() as u16,
                "outer column width should equal the path's natural width at a wide terminal"
            );
        });
    });
}

/// At a genuinely narrow terminal the path header should clip gracefully rather
/// than overflow or panic, and the outer column should stay within the available
/// width.
#[test]
fn path_header_outside_constrained_box_degrades_gracefully_at_narrow_terminal() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let long_path =
                "~/work/projects/my-organisation/very-long-repository-name-that-exceeds-48-cols";
            let narrow_width: u16 = 30;
            assert!(
                long_path.len() as u16 > narrow_width,
                "test path must exceed the narrow terminal width"
            );

            let constrained_top = TuiConstrainedBox::new(TuiText::new("Warp Agent CLI").finish())
                .with_min_cols(LEFT_COLUMN_COLS)
                .with_max_cols(LEFT_COLUMN_COLS);
            let path_header = TuiText::new(long_path).truncate();
            let constrained_bottom =
                TuiConstrainedBox::new(TuiText::new("Not configured · /mcp").finish())
                    .with_min_cols(LEFT_COLUMN_COLS)
                    .with_max_cols(LEFT_COLUMN_COLS);

            let mut outer = TuiFlex::column()
                .child(constrained_top.finish())
                .child(path_header.finish())
                .child(constrained_bottom.finish());

            // narrow_width < LEFT_COLUMN_COLS so the constrained boxes clamp to
            // narrow_width, and so does the path header.
            let size = layout_at(&mut outer, narrow_width, 10, app_ctx);

            assert_eq!(
                size.width, narrow_width,
                "outer column should fit within the available {narrow_width} cols at a \
                 narrow terminal"
            );
        });
    });
}
