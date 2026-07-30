use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use channel_versions::{Changelog, MarkdownSection, Section};
use chrono::DateTime;
use uuid::Uuid;
use warp::settings::{
    TuiZeroStateExtrusionDepthSetting, TuiZeroStateObjectSetting,
    TuiZeroStateRotationPeriodSecondsSetting, TuiZeroStateSettings,
    TuiZeroStateShowAnimationSetting, TuiZeroStateShowChangelogSetting, TuiZeroStateShowMcpSetting,
    TuiZeroStateShowProjectInfoSetting, TuiZeroStateShowSignedInUserSetting,
};
use warp::tui_export::{
    ChangelogModel, ChangelogState, TuiMcpConfigState, TuiMcpServerId, TuiMcpServerSnapshot,
    TuiMcpServerStatus, TuiMcpSnapshot, TuiMcpTransport, register_tui_session_view_test_singletons,
};
use warp_core::settings::Setting as _;
use warpui::{EntityIdMap, SingletonEntity};
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    Color, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize, TuiStyle, TuiText, text_width,
};
use warpui_core::{App, AppContext};

use super::{
    ANIMATION_PANEL_COLS, LEFT_COLUMN_COLS, ZeroStateSectionVisibility,
    build_zero_state_copy_only_layout, build_zero_state_layout, build_zero_state_overlay,
    changelog_bullets_from_changelog, mcp_status_label,
};
use crate::tui_builder::TuiUiBuilder;
use crate::zero_state_animation::{
    WarpLogoStyles, ZeroStateAnimationConfig, ZeroStateAnimationElement, ZeroStateStarfieldElement,
};

/// Every optional zero-state section hidden.
fn all_sections_hidden() -> ZeroStateSectionVisibility {
    ZeroStateSectionVisibility {
        signed_in_user: false,
        changelog: false,
        project_info: false,
        mcp: false,
        animation: false,
    }
}

/// Installs a changelog with a single TUI bullet so the "What's new" section
/// has content to render.
fn add_test_changelog(app: &mut App) {
    app.update(|ctx| {
        ChangelogModel::handle(ctx).update(ctx, |model, _| {
            model.changelog = ChangelogState::Some(changelog(vec!["Configurable zero state"]));
        });
    });
}

fn server(id: u64, status: TuiMcpServerStatus) -> TuiMcpServerSnapshot {
    TuiMcpServerSnapshot {
        id: TuiMcpServerId(id),
        installation_uuid: Uuid::from_u128(id as u128),
        name: format!("server-{id}"),
        transport: TuiMcpTransport::Stdio,
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
            let overlay = build_zero_state_overlay(
                Some(long_cwd),
                ZeroStateSectionVisibility::default(),
                &builder,
                app_ctx,
            );
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

            let overlay = build_zero_state_overlay(
                Some(long_cwd),
                ZeroStateSectionVisibility::default(),
                &builder,
                app_ctx,
            );
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

// ---------------------------------------------------------------------------
// Per-section visibility (APP-5070)
// ---------------------------------------------------------------------------

/// With the default settings every section is rendered, so the overlay keeps
/// showing the account line, the changelog, the project path, and MCP.
#[test]
fn zero_state_shows_every_section_by_default() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);
        add_test_changelog(&mut app);

        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let overlay = build_zero_state_overlay(
                Some("/tmp/project"),
                ZeroStateSectionVisibility::default(),
                &builder,
                ctx,
            );
            let rendered = render_element_lines(overlay, ctx, 60, 24).join("\n");

            assert!(rendered.contains("Warp Agent CLI"), "{rendered}");
            assert!(
                rendered.contains("Signed in as test_user@warp.dev"),
                "{rendered}"
            );
            assert!(rendered.contains("What's new"), "{rendered}");
            assert!(rendered.contains("Configurable zero state"), "{rendered}");
            assert!(rendered.contains("/tmp/project"), "{rendered}");
            assert!(rendered.contains("MCP"), "{rendered}");
        });
    });
}

/// Each toggle hides exactly its own section and leaves the others alone.
#[test]
fn zero_state_hides_only_the_section_whose_toggle_is_off() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);
        add_test_changelog(&mut app);

        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let render_with = |visibility| {
                let overlay =
                    build_zero_state_overlay(Some("/tmp/project"), visibility, &builder, ctx);
                render_element_lines(overlay, ctx, 60, 24).join("\n")
            };

            let hidden_account = render_with(ZeroStateSectionVisibility {
                signed_in_user: false,
                ..ZeroStateSectionVisibility::default()
            });
            assert!(!hidden_account.contains("Signed in"), "{hidden_account}");
            assert!(hidden_account.contains("What's new"), "{hidden_account}");
            assert!(hidden_account.contains("/tmp/project"), "{hidden_account}");
            assert!(hidden_account.contains("MCP"), "{hidden_account}");

            let hidden_changelog = render_with(ZeroStateSectionVisibility {
                changelog: false,
                ..ZeroStateSectionVisibility::default()
            });
            assert!(
                !hidden_changelog.contains("What's new"),
                "{hidden_changelog}"
            );
            assert!(
                !hidden_changelog.contains("Configurable zero state"),
                "{hidden_changelog}"
            );
            assert!(hidden_changelog.contains("Signed in"), "{hidden_changelog}");

            let hidden_project = render_with(ZeroStateSectionVisibility {
                project_info: false,
                ..ZeroStateSectionVisibility::default()
            });
            assert!(!hidden_project.contains("/tmp/project"), "{hidden_project}");
            assert!(hidden_project.contains("MCP"), "{hidden_project}");

            let hidden_mcp = render_with(ZeroStateSectionVisibility {
                mcp: false,
                ..ZeroStateSectionVisibility::default()
            });
            assert!(!hidden_mcp.contains("MCP"), "{hidden_mcp}");
            assert!(hidden_mcp.contains("/tmp/project"), "{hidden_mcp}");
        });
    });
}

/// Turning every section off leaves only the always-on title and version rows —
/// no orphaned headers and no blank spacer rows for the hidden sections.
#[test]
fn zero_state_with_all_sections_hidden_renders_only_title_and_version() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);
        add_test_changelog(&mut app);

        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let overlay = build_zero_state_overlay(
                Some("/tmp/project"),
                all_sections_hidden(),
                &builder,
                ctx,
            );
            let lines = render_element_lines(overlay, ctx, 60, 24);

            assert_eq!(
                lines.len(),
                2,
                "only the title and version rows should remain;\ngot lines:\n{}",
                lines.join("\n")
            );
            assert_eq!(lines[0].trim_end(), "Warp Agent CLI");
            assert!(!lines[1].trim_end().is_empty(), "{:?}", lines[1]);
        });
    });
}

/// Hiding the animation drops the starfield and the reserved animation panel
/// instead of leaving an empty region beside the copy.
#[test]
fn zero_state_copy_only_layout_reserves_no_animation_panel() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let copy_only = render_to_buffer(
                build_zero_state_copy_only_layout(TuiText::new("copy here").finish()),
                ctx,
                120,
                9,
            );
            assert_eq!(
                copy_only.area.width,
                text_width("copy here"),
                "the copy-only layout should occupy just the copy, not the animation panel"
            );
            assert!(
                copy_only
                    .to_lines()
                    .iter()
                    .any(|line| line.trim_end() == "copy here")
            );

            let with_animation = render_to_buffer(
                build_zero_state_layout(
                    TuiText::new("*".repeat(120)).finish(),
                    TuiText::new("").finish(),
                    TuiText::new("copy here").finish(),
                ),
                ctx,
                120,
                9,
            );
            assert!(
                with_animation.area.width > copy_only.area.width,
                "the animated layout still spans the full width"
            );
        });
    });
}

/// The visibility snapshot reads the settings group when it is registered and
/// falls back to "everything visible" when it is not.
#[test]
fn zero_state_visibility_reads_settings_and_defaults_to_visible() {
    App::test((), |mut app| async move {
        app.read(|ctx| {
            assert_eq!(
                ZeroStateSectionVisibility::from_settings(ctx),
                ZeroStateSectionVisibility::default(),
                "an unregistered settings group must keep every section visible"
            );
        });

        app.update(|ctx| {
            ctx.add_singleton_model(|_| TuiZeroStateSettings {
                object: TuiZeroStateObjectSetting::new(None),
                rotation_period_seconds: TuiZeroStateRotationPeriodSecondsSetting::new(None),
                extrusion_depth: TuiZeroStateExtrusionDepthSetting::new(None),
                show_signed_in_user: TuiZeroStateShowSignedInUserSetting::new(Some(false)),
                show_changelog: TuiZeroStateShowChangelogSetting::new(None),
                show_project_info: TuiZeroStateShowProjectInfoSetting::new(Some(false)),
                show_mcp: TuiZeroStateShowMcpSetting::new(None),
                show_animation: TuiZeroStateShowAnimationSetting::new(Some(false)),
            });
        });

        app.read(|ctx| {
            assert_eq!(
                ZeroStateSectionVisibility::from_settings(ctx),
                ZeroStateSectionVisibility {
                    signed_in_user: false,
                    changelog: true,
                    project_info: false,
                    mcp: true,
                    animation: false,
                },
                "unset toggles keep their visible default while set ones are honored"
            );
        });
    });
}
