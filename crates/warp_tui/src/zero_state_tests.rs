use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use channel_versions::{Changelog, MarkdownSection, Section};
use chrono::DateTime;
use uuid::Uuid;
use warp::tui_export::{
    TuiMcpConfigDiagnostic, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerSource,
    TuiMcpServerStatus, TuiMcpSnapshot, TuiMcpTransport, TuiUserInfoSnapshot,
    register_tui_session_view_test_singletons,
};
use warpui::{EntityIdMap, SingletonEntity};
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    Color, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize, TuiStyle, TuiText, text_width,
};
use warpui_core::{App, AppContext};

use super::{
    ANIMATION_PANEL_COLS, LEFT_COLUMN_COLS, ZeroStateSectionVisibility, autoupdate_status_label,
    build_zero_state_layout, build_zero_state_overlay, build_zero_state_stack_layout,
    changelog_bullets_from_changelog, mcp_status_label, render_first_run_top_section,
};
use crate::autoupdate::TuiAutoupdateStatus;
use crate::tui_builder::TuiUiBuilder;
use crate::zero_state_animation::{
    WarpLogoStyles, ZeroStateAnimationConfig, ZeroStateAnimationElement,
    ZeroStateInteractionHandle, ZeroStateStarfieldElement,
};

fn server(id: u64, status: TuiMcpServerStatus) -> TuiMcpServerSnapshot {
    TuiMcpServerSnapshot {
        id: TuiMcpServerId::Installation(Uuid::from_u128(id as u128)),
        installation_uuid: Some(Uuid::from_u128(id as u128)),
        name: format!("server-{id}"),
        description: None,
        source: TuiMcpServerSource::Installation,
        transport: Some(TuiMcpTransport::Stdio),
        status,
        tool_count: 2,
        resource_count: 0,
        can_log_out: false,
        authorization_url: None,
    }
}

fn changelog(tui_updates: Vec<&str>) -> Changelog {
    Changelog {
        date: DateTime::parse_from_rfc3339("2026-07-30T12:00:00+00:00").unwrap(),
        sections: vec![Section {
            title: "Improvements".to_owned(),
            items: vec!["Unrelated GUI improvement".to_owned()],
        }],
        markdown_sections: vec![MarkdownSection {
            title: "Improvements".to_owned(),
            markdown: "* Unrelated GUI improvement\n".to_owned(),
        }],
        image_url: None,
        oz_updates: vec!["Unrelated Oz improvement".to_owned()],
        tui_updates: tui_updates.into_iter().map(ToOwned::to_owned).collect(),
    }
}

#[test]
fn changelog_bullets_use_only_the_first_three_tui_updates() {
    let changelog = changelog(vec!["First", "Second", "Third", "Fourth"]);
    assert_eq!(
        changelog_bullets_from_changelog(&changelog),
        ["First", "Second", "Third"]
    );
}

#[test]
fn changelog_bullets_are_empty_when_only_other_surfaces_have_updates() {
    assert!(changelog_bullets_from_changelog(&changelog(Vec::new())).is_empty());
}

#[test]
fn failed_autoupdate_status_has_visible_label() {
    assert_eq!(
        autoupdate_status_label(TuiAutoupdateStatus::Failed),
        Some("automatic update failed")
    );
}

#[test]
fn homebrew_update_status_shows_the_upgrade_command() {
    assert_eq!(
        autoupdate_status_label(TuiAutoupdateStatus::UpdateAvailable),
        Some("update available — run brew upgrade --cask warp-agent-cli")
    );
}

#[test]
fn first_zero_state_matches_welcome_design_copy() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);

        let lines = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            render_element_lines(
                render_first_run_top_section(&builder, ZeroStateSectionVisibility::default(), ctx)
                    .finish(),
                ctx,
                LEFT_COLUMN_COLS,
                16,
            )
        });
        let rendered = lines.join("\n");
        for expected in [
            "Welcome to Warp",
            "What’s different about Warp",
            "✶ State of the art coding agents",
            "✶ Frontier and open-weight models",
            "✶ Fully customizable model routers",
            "✶ Orchestration for fleets of agents",
            "✶ Better shell command support",
        ] {
            assert!(
                rendered.contains(expected),
                "first zero state should contain {expected:?}:\n{rendered}"
            );
        }
        for unexpected in [
            "/natural-language-detection",
            "/modify-settings",
            "/orchestrate",
            "Run full-screen terminal apps",
            "Orchestrate fleets of agents",
            "Work with shell commands like in a native terminal",
        ] {
            assert!(
                !rendered.contains(unexpected),
                "first zero state should not contain {unexpected:?}:\n{rendered}"
            );
        }
        assert!(!rendered.contains("What's new"));
        assert!(!rendered.contains("████"));
    });
}

#[test]
fn mcp_summary_keeps_empty_catalog_action_short() {
    let snapshot = TuiMcpSnapshot {
        diagnostics: Vec::new(),
        servers: Vec::new(),
    };

    assert_eq!(
        mcp_status_label(&snapshot),
        ("No servers available · run /mcp".to_string(), false)
    );
}

#[test]
fn mcp_summary_reports_mixed_runtime_states() {
    let snapshot = TuiMcpSnapshot {
        diagnostics: Vec::new(),
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
            server(7, TuiMcpServerStatus::Available),
        ],
    };

    assert_eq!(
        mcp_status_label(&snapshot),
        (
            "1 connected · 1 starting · 1 needs auth · 1 stopping · 1 failed · 1 offline · 1 available · /mcp"
                .to_string(),
            false
        )
    );
}

#[test]
fn mcp_summary_marks_config_errors() {
    let snapshot = TuiMcpSnapshot {
        diagnostics: vec![
            TuiMcpConfigDiagnostic {
                provider: "Claude".to_owned(),
                config_path: PathBuf::from("/tmp/.claude.json"),
                message: "invalid JSON".to_owned(),
            },
            TuiMcpConfigDiagnostic {
                provider: "Codex".to_owned(),
                config_path: PathBuf::from("/tmp/config.toml"),
                message: "invalid TOML".to_owned(),
            },
        ],
        servers: Vec::new(),
    };

    assert_eq!(
        mcp_status_label(&snapshot),
        ("2 config errors · /mcp".to_string(), true)
    );
}

// ---------------------------------------------------------------------------
// Render tests for the path-header fix (APP-5009)
//
// Both tests call `build_zero_state_overlay` — the same function that
// `TuiZeroStateView::render` uses to compose the overlay column.  Any change
// to how `render` places the path header (e.g. moving it back inside the
// LEFT_COLUMN_COLS constrained box) goes through `build_zero_state_overlay`
// and is therefore caught here.
//
// Verified empirically: wrapping `path_header` back in a TuiConstrainedBox
// with min=max=LEFT_COLUMN_COLS inside `build_zero_state_overlay` causes the
// wide-terminal test to fail because the buffer is only 48 cols wide and the
// 60-char path is clipped — no row matches `header_text`.
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

fn render_element_lines(
    element: Box<dyn TuiElement>,
    ctx: &AppContext,
    width: u16,
    height: u16,
) -> Vec<String> {
    render_to_buffer(element, ctx, width, height).to_lines()
}

fn static_zero_state_layers(
    width: u16,
    height: u16,
) -> (
    Box<dyn TuiElement>,
    Box<dyn TuiElement>,
    Box<dyn TuiElement>,
) {
    let stars = (0..height)
        .map(|_| "*".repeat(usize::from(width)))
        .collect::<Vec<_>>()
        .join("\n");
    (
        TuiText::new(stars).finish(),
        TuiText::new("animation").finish(),
        TuiText::new("copy here\n\nsecond line").finish(),
    )
}

#[test]
fn direct_zero_state_layers_match_scratch_composition() {
    App::test((), |app| async move {
        app.read(|ctx| {
            for (width, height) in [(60, 12), (80, 24), (120, 40), (240, 80)] {
                let (stars, animation, overlay) = static_zero_state_layers(width, height);
                let direct = render_to_buffer(
                    build_zero_state_layout(stars, animation, overlay),
                    ctx,
                    width,
                    height,
                );
                let (stars, animation, overlay) = static_zero_state_layers(width, height);
                let scratch = render_to_buffer(
                    build_zero_state_stack_layout(stars, animation, overlay),
                    ctx,
                    width,
                    height,
                );
                assert_eq!(direct, scratch, "layout differs at {width}x{height}");
            }
        });
    });
}

#[test]
fn zero_state_copy_rectangle_is_opaque_without_changing_the_background_color() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let stars = (0..9)
                .map(|_| "*".repeat(80))
                .collect::<Vec<_>>()
                .join("\n");
            let layout = build_zero_state_layout(
                TuiText::new(stars).finish(),
                TuiText::new("").finish(),
                TuiText::new("copy here\n\nline").finish(),
            );
            let buffer = render_to_buffer(layout, ctx, 80, 9);
            let lines = buffer.to_lines();
            assert_eq!(&lines[3][..9], "copy here");
            assert_eq!(&lines[5][..4], "line");
            for y in 3..=5 {
                for x in 0..9 {
                    assert_ne!(buffer[(x, y)].symbol(), "*");
                    assert_eq!(buffer[(x, y)].bg, Color::Reset);
                }
            }
            assert_eq!(buffer[(1, 2)].symbol(), "*");
            assert_eq!(buffer[(1, 6)].symbol(), "*");
            assert_eq!(buffer[(9, 3)].symbol(), "*");
            assert_eq!(buffer[(1, 2)].bg, Color::Reset);
            assert_eq!(buffer[(1, 6)].bg, Color::Reset);
            assert_eq!(buffer[(9, 3)].bg, Color::Reset);
        });
    });
}
#[test]
fn zero_state_starfield_spans_the_full_width() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout = build_zero_state_layout(
                ZeroStateStarfieldElement::new(
                    AnimationClock::starting_at(Duration::ZERO),
                    TuiStyle::default(),
                    LEFT_COLUMN_COLS,
                    ANIMATION_PANEL_COLS,
                )
                .finish(),
                TuiText::new("").finish(),
                TuiText::new("").finish(),
            );
            let buffer = render_to_buffer(layout, ctx, 120, 20);
            let occupied_columns = buffer
                .content
                .iter()
                .enumerate()
                .filter_map(|(index, cell)| {
                    (cell.symbol() != " ").then_some(index % usize::from(buffer.area.width))
                })
                .collect::<Vec<_>>();

            assert!(occupied_columns.iter().any(|column| *column < 30));
            assert!(occupied_columns.iter().any(|column| *column >= 90));
        });
    });
}

#[test]
fn zero_state_animation_is_centered_in_remaining_space_and_hidden_when_space_is_tight() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let animation = || {
                let style = TuiStyle::default();
                ZeroStateAnimationElement::new(
                    AnimationClock::starting_at(Duration::ZERO),
                    Arc::new(ZeroStateAnimationConfig::default()),
                    ZeroStateInteractionHandle::default(),
                    WarpLogoStyles {
                        front: style,
                        back: style,
                        side: style,
                        background: style,
                    },
                )
                .without_background_stars()
                .finish()
            };
            let layout = build_zero_state_layout(
                TuiText::new("").finish(),
                animation(),
                TuiText::new("").finish(),
            );
            let wide_width = 120;
            let wide = render_to_buffer(layout, ctx, wide_width, 20);
            let occupied = wide
                .content
                .iter()
                .enumerate()
                .filter_map(|(index, cell)| {
                    (cell.symbol() != " ").then_some(index % usize::from(wide.area.width))
                })
                .collect::<Vec<_>>();
            let remaining_cols = wide_width - LEFT_COLUMN_COLS;
            let animation_start = LEFT_COLUMN_COLS + (remaining_cols - ANIMATION_PANEL_COLS) / 2;
            let animation_end = animation_start + ANIMATION_PANEL_COLS;

            assert!(!occupied.is_empty());
            assert!(
                occupied
                    .iter()
                    .all(|column| *column >= usize::from(animation_start)
                        && *column < usize::from(animation_end))
            );

            let layout = build_zero_state_layout(
                TuiText::new("").finish(),
                animation(),
                TuiText::new("").finish(),
            );
            assert!(
                render_to_buffer(layout, ctx, 60, 20)
                    .content
                    .iter()
                    .all(|cell| cell.symbol() == " ")
            );
        });
    });
}
/// When the terminal is wide enough, the path header must stay on one row and
/// must not be capped at LEFT_COLUMN_COLS.
///
/// Calls the real `build_zero_state_overlay` (the same function used by
/// `TuiZeroStateView::render`) and asserts the path appears verbatim in the
/// rendered `TuiBuffer`.  Any regression that moves the path back inside the
/// 48-col constrained box causes this test to fail: the buffer would be only
/// 48 cols wide and the 60-char path would be clipped — no row would equal
/// `header_text`.
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

            // project_section_header_text returns abbreviate_home_prefix(long_cwd)
            // when no rules are indexed; with the sandbox HOME=/root the path is
            // returned unchanged.
            let header_text = {
                use ai::project_context::model::ProjectContextModel;
                use warp_util::local_or_remote_path::LocalOrRemotePath;
                let cwd_path = LocalOrRemotePath::Local(PathBuf::from(long_cwd));
                let rules =
                    ProjectContextModel::as_ref(app_ctx).find_applicable_project_rules(&cwd_path);
                super::project_section_header_text(long_cwd, rules.as_ref())
            };
            assert!(
                header_text.len() as u16 > LEFT_COLUMN_COLS,
                "resolved header ({header_text:?}) must still exceed LEFT_COLUMN_COLS"
            );

            // Give the overlay exactly enough width for the displayed path.
            // Call build_zero_state_overlay -- the same function render() calls.
            let overlay = build_zero_state_overlay(Some(long_cwd), &builder, app_ctx);
            let buffer = render_to_buffer(overlay, app_ctx, text_width(&header_text), 12);
            let lines = buffer.to_lines();

            // The path header should appear as an exact-match row somewhere in the
            // rendered buffer.  Its row index varies by title/version content so we
            // search.  If the path were inside the 48-col box the buffer would be 48
            // cols wide and the 60-char path would be clipped -- the assertion fails.
            let _ = lines
                .iter()
                .position(|line| line.trim_end() == header_text)
                .unwrap_or_else(|| {
                    panic!(
                        "path header {header_text:?} must appear verbatim in the rendered output;\n\
                         got lines:\n{}",
                        lines.join("\n")
                    )
                });
            assert!(
                header_text.len() as u16 > LEFT_COLUMN_COLS,
                "path header length {} should exceed LEFT_COLUMN_COLS ({})",
                header_text.len(),
                LEFT_COLUMN_COLS
            );
        });
    });
}

#[test]
fn login_line_shows_signed_in_account_email() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);

        let lines = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            render_element_lines(super::render_login_line(&builder, ctx), ctx, 48, 1)
        });
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Signed in as test_user@warp.dev")),
            "zero-state login line should show the signed-in email:\n{}",
            lines.join("\n")
        );
    });
}

#[test]
fn login_line_never_claims_signed_in_without_an_identity() {
    let snapshot = TuiUserInfoSnapshot {
        is_logged_in: true,
        ..Default::default()
    };

    assert_eq!(super::login_line_label("Signed in as", snapshot), None);
}

#[test]
fn login_line_falls_back_to_validated_user_id() {
    let snapshot = TuiUserInfoSnapshot {
        is_logged_in: true,
        user_id: Some("user-123".to_owned()),
        ..Default::default()
    };

    assert_eq!(
        super::login_line_label("Signed in as", snapshot),
        Some("Signed in as user-123".to_owned())
    );
}

/// At a narrow terminal the complete displayed path must wrap across rows
/// without losing content.
#[test]
fn zero_state_path_header_wraps_without_losing_content_at_narrow_terminal() {
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

            // Derive expected wrapped rows from header_text (the abbreviated path),
            // not from long_cwd -- so the assertion is correct even if $HOME changes.
            let header_text = {
                use ai::project_context::model::ProjectContextModel;
                use warp_util::local_or_remote_path::LocalOrRemotePath;
                let cwd_path = LocalOrRemotePath::Local(PathBuf::from(long_cwd));
                let rules =
                    ProjectContextModel::as_ref(app_ctx).find_applicable_project_rules(&cwd_path);
                super::project_section_header_text(long_cwd, rules.as_ref())
            };
            let header_chars = header_text.chars().collect::<Vec<_>>();
            let expected_wrapped = header_chars
                .chunks(usize::from(narrow_width))
                .map(|chunk| chunk.iter().collect::<String>())
                .collect::<Vec<_>>();
            assert!(
                expected_wrapped.len() > 1,
                "test path must wrap at the narrow terminal width"
            );

            let overlay = build_zero_state_overlay(Some(long_cwd), &builder, app_ctx);
            let buffer = render_to_buffer(overlay, app_ctx, narrow_width, 12);
            let lines = buffer.to_lines();

            // Buffer width must clamp to narrow_width.
            assert_eq!(
                buffer.area.width, narrow_width,
                "buffer width should be clamped to narrow_width"
            );
            // The wrapped path rows must appear consecutively so joining them
            // reconstructs the complete displayed path.
            let has_wrapped_rows = lines.windows(expected_wrapped.len()).any(|rows| {
                rows.iter()
                    .map(|row| row.trim_end())
                    .eq(expected_wrapped.iter().map(String::as_str))
            });
            assert!(
                has_wrapped_rows,
                "wrapped path rows {expected_wrapped:?} must appear consecutively \
                 in narrow output;\ngot lines:\n{}",
                lines.join("\n")
            );
        });
    });
}
