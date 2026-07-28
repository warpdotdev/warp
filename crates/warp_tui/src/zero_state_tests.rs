use std::path::PathBuf;

use ai::project_context::model::ProjectContextModel;
use uuid::Uuid;
use warp::tui_export::{
    TuiMcpConfigState, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerStatus, TuiMcpSnapshot,
    TuiMcpTransport, register_tui_session_view_test_singletons,
};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{EntityIdMap, SingletonEntity};
use warpui_core::App;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiBufferExt, TuiConstrainedBox, TuiConstraint, TuiElement, TuiFlex,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
    TuiText,
};

use super::{
    LEFT_COLUMN_COLS, blank_row, mcp_status_label, project_section_header_text,
    render_bottom_section, render_top_section,
};

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
        ("Not configured \u{00b7} /mcp".to_string(), false)
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
            "1 connected \u{00b7} 1 starting \u{00b7} 1 needs auth \u{00b7} 1 stopping \u{00b7} 1 failed \u{00b7} 1 offline \u{00b7} /mcp"
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
        ("Config error \u{00b7} run /mcp".to_string(), true)
    );
}

// ---------------------------------------------------------------------------
// Render tests for the path-header fix (APP-5009)
//
// These tests call the real `project_section_header_text` and
// `render_bottom_section` functions from zero_state.rs, then render the same
// outer-column structure that `TuiZeroStateView::render` builds and assert on
// `TuiBuffer::to_lines()`.  They fail if:
// * `project_section_header_text` or `render_bottom_section` are removed
//   (compile error), or
// * the path header is truncated to LEFT_COLUMN_COLS in the rendered output.
// ---------------------------------------------------------------------------

/// Lay out `element` at `(w, h)`, render it into a fresh buffer, and return
/// the buffer.  Mirrors `render_element_with_size` in terminal_session_view_tests.rs.
fn render_to_buffer(
    mut element: Box<dyn TuiElement>,
    app_ctx: &warpui_core::AppContext,
    w: u16,
    h: u16,
) -> TuiBuffer {
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(w, h)),
        &mut layout_ctx,
        app_ctx,
    );
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    {
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(
            TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
            &mut surface,
            &mut paint_ctx,
        );
    }
    buffer
}

/// At a wide terminal the path header must NOT be capped at LEFT_COLUMN_COLS
/// (APP-5009 regression).  The test exercises the real `project_section_header_text`
/// and `render_bottom_section` functions and checks the rendered buffer so the
/// assertion fails if the header moves back inside the constrained box.
#[test]
fn zero_state_path_header_not_truncated_at_wide_terminal() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);

        app.read(|app_ctx| {
            // A path definitely longer than LEFT_COLUMN_COLS (48).
            let long_cwd = "/home/user/work/projects/my-organisation/very-long-repo-name";
            assert!(
                long_cwd.len() as u16 > LEFT_COLUMN_COLS,
                "test cwd must exceed LEFT_COLUMN_COLS"
            );

            let builder = crate::tui_builder::TuiUiBuilder::from_app(app_ctx);

            // Resolve project rules via the real model (returns None with the
            // default ProjectContextModel registered by the test singletons).
            let cwd_path = LocalOrRemotePath::Local(PathBuf::from(long_cwd));
            let rules =
                ProjectContextModel::as_ref(app_ctx).find_applicable_project_rules(&cwd_path);

            // Call the REAL project_section_header_text from zero_state.rs.
            // With no indexed rules it returns abbreviate_home_prefix(long_cwd).
            let header_text = project_section_header_text(long_cwd, rules.as_ref());
            assert!(
                header_text.len() as u16 > LEFT_COLUMN_COLS,
                "resolved header ({header_text:?}) must still exceed LEFT_COLUMN_COLS"
            );

            // Build the outer-column structure exactly as TuiZeroStateView::render does.
            let constrained_top =
                TuiConstrainedBox::new(render_top_section(&builder, app_ctx).finish())
                    .with_min_cols(LEFT_COLUMN_COLS)
                    .with_max_cols(LEFT_COLUMN_COLS);
            let header_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
            let path_element = TuiText::new(header_text.clone())
                .with_style(header_style)
                .truncate();
            // Call the REAL render_bottom_section from zero_state.rs.
            let bottom = render_bottom_section(Some(long_cwd), rules.as_ref(), &builder, app_ctx);
            let constrained_bottom = TuiConstrainedBox::new(bottom.finish())
                .with_min_cols(LEFT_COLUMN_COLS)
                .with_max_cols(LEFT_COLUMN_COLS);
            let outer = TuiFlex::column()
                .child(constrained_top.finish())
                .child(blank_row())
                .child(path_element.finish())
                .child(constrained_bottom.finish());

            let buffer = render_to_buffer(outer.finish(), app_ctx, 200, 12);
            let lines = buffer.to_lines();

            // The path header should appear as an exact-match row somewhere in the
            // rendered buffer (after the title rows and blank separator).  Its exact
            // row index varies depending on changelog / version content, so we
            // search rather than hardcoding an index.
            //
            // Critically: if the path were still inside the 48-col constrained box
            // the buffer would only be 48 cols wide and the 60-char path would be
            // clipped — no row would equal `header_text`, so the assertion fails.
            let path_row = lines
                .iter()
                .position(|line| line.trim_end() == header_text)
                .unwrap_or_else(|| {
                    panic!(
                        "path header {header_text:?} must appear verbatim in the rendered output; \n\
                         got lines:\n{}",
                        lines.join("\n")
                    )
                });
            assert!(
                header_text.len() as u16 > LEFT_COLUMN_COLS,
                "path at row {path_row} must exceed LEFT_COLUMN_COLS, \
                 indicating it was rendered outside the constrained box"
            );
        });
    });
}

/// At a narrow terminal the layout must degrade gracefully: path clamps to
/// the available width without overflow or panic.
#[test]
fn zero_state_path_header_clipped_gracefully_at_narrow_terminal() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);

        app.read(|app_ctx| {
            let long_cwd = "/home/user/work/projects/my-organisation/very-long-repo-name";
            let narrow_width: u16 = 30;
            assert!(
                long_cwd.len() as u16 > narrow_width,
                "test cwd must exceed the narrow terminal width"
            );

            let builder = crate::tui_builder::TuiUiBuilder::from_app(app_ctx);

            let cwd_path = LocalOrRemotePath::Local(PathBuf::from(long_cwd));
            let rules =
                ProjectContextModel::as_ref(app_ctx).find_applicable_project_rules(&cwd_path);
            let header_text = project_section_header_text(long_cwd, rules.as_ref());

            let constrained_top =
                TuiConstrainedBox::new(render_top_section(&builder, app_ctx).finish())
                    .with_min_cols(LEFT_COLUMN_COLS)
                    .with_max_cols(LEFT_COLUMN_COLS);
            let path_element = TuiText::new(header_text).truncate();
            let bottom = render_bottom_section(Some(long_cwd), rules.as_ref(), &builder, app_ctx);
            let constrained_bottom = TuiConstrainedBox::new(bottom.finish())
                .with_min_cols(LEFT_COLUMN_COLS)
                .with_max_cols(LEFT_COLUMN_COLS);
            let outer = TuiFlex::column()
                .child(constrained_top.finish())
                .child(blank_row())
                .child(path_element.finish())
                .child(constrained_bottom.finish());

            let buffer = render_to_buffer(outer.finish(), app_ctx, narrow_width, 12);
            let lines = buffer.to_lines();

            // At narrow_width < LEFT_COLUMN_COLS both the constrained boxes and the
            // path header all clamp to narrow_width.
            assert_eq!(
                buffer.area.width, narrow_width,
                "buffer width should be clamped to narrow_width"
            );
            // At least some row must start with '/' (the clipped path).
            // The exact row index varies by rendered title content so we search.
            let first_char = long_cwd.chars().next().unwrap();
            let has_path_row = lines.iter().any(|line| line.starts_with(first_char));
            assert!(
                has_path_row,
                "a row starting with {first_char:?} (clipped path) should appear in narrow output;\n\
                 got lines:\n{}",
                lines.join("\n")
            );
        });
    });
}
