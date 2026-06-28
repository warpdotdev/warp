//! Regression tests for [`TuiInputView`] cursor/coordinate + kill logic.
//!
//! These drive a real [`CodeEditorModel`] (TUI char-cell mode) behind a real
//! [`TuiInputView`] so they exercise the exact render/layout/cursor path the
//! presenter uses, not a reimplementation of it.

use std::collections::HashMap;

use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{TuiConstraint, TuiLayoutContext, TuiRect, TuiSize};
use warpui_core::platform::WindowStyle;
use warpui_core::{AddWindowOptions, App, AppContext, TuiView, TypedActionView, ViewHandle};

use super::{TuiInputAction, TuiInputView};

const W: u16 = 80;

fn build_view(ctx: &mut AppContext) -> ViewHandle<TuiInputView> {
    // `CodeEditorModel::new_tui` reads syntax colors from the `Appearance`
    // singleton, so register a mock one before constructing the editor.
    ctx.add_singleton_model(|_| Appearance::mock());
    let (_window_id, view) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        |ctx| {
            let model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
            TuiInputView::new(model, ctx)
        },
    );
    view
}

fn dispatch(view: &ViewHandle<TuiInputView>, ctx: &mut AppContext, actions: &[TuiInputAction]) {
    view.update(ctx, |v, vctx| {
        for action in actions {
            v.handle_action(action, vctx);
        }
    });
}

fn type_str(view: &ViewHandle<TuiInputView>, ctx: &mut AppContext, s: &str) {
    let actions: Vec<TuiInputAction> = s.chars().map(TuiInputAction::InsertChar).collect();
    dispatch(view, ctx, &actions);
}

/// Render the view, lay it out at width `W`, and return `(cursor, height)`.
fn cursor_and_height(
    view: &ViewHandle<TuiInputView>,
    ctx: &AppContext,
) -> (Option<(u16, u16)>, u16) {
    let mut element = view.as_ref(ctx).render(ctx);
    let mut rendered_views = HashMap::new();
    let mut lctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(TuiConstraint::loose(TuiSize::new(W, 20)), &mut lctx, ctx);
    let cursor = element.cursor_position(TuiRect::new(0, 0, size.width, size.height), &mut lctx);
    (cursor, size.height)
}

fn text(view: &ViewHandle<TuiInputView>, ctx: &AppContext) -> String {
    let v = view.as_ref(ctx);
    let inner = v.model().as_ref(ctx);
    let buffer = inner.content().as_ref(ctx);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

/// The currently selected substring, or `None` when there is no selection.
fn selected_text(view: &ViewHandle<TuiInputView>, ctx: &AppContext) -> Option<String> {
    let range = view.as_ref(ctx).selection_range(ctx)?;
    // `selection_range` is a 1-based gap range; convert to 0-based plain-text indices.
    let start = range.start.as_usize().saturating_sub(1);
    let end = range.end.as_usize().saturating_sub(1);
    let full = text(view, ctx);
    Some(full.chars().skip(start).take(end - start).collect())
}

#[test]
fn cursor_at_origin_when_empty() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((0, 0)));
            assert_eq!(height, 1);
        });
    });
}

/// Regression: navigating a freshly-built (empty, never-edited) view must not
/// panic. The char-cell `line_starts` is seeded with `[0]` at construction, so
/// the soft-wrap helpers reached via `move_to_line_start` etc. index it safely
/// before the first edit ever runs `CharCellState::update_text`.
#[test]
fn navigation_on_empty_buffer_does_not_panic() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            dispatch(
                &view,
                ctx,
                &[
                    TuiInputAction::MoveToLineStart,
                    TuiInputAction::MoveToLineEnd,
                    TuiInputAction::MoveLeft,
                    TuiInputAction::MoveRight,
                    TuiInputAction::MoveUp,
                    TuiInputAction::MoveDown,
                ],
            );
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((0, 0)));
            assert_eq!(height, 1);
        });
    });
}

#[test]
fn cursor_tracks_end_of_single_line() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "ab");
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((2, 0)));
            assert_eq!(height, 1);
        });
    });
}

/// Bug 1: after a hard newline the cursor must render at the start of the new
/// (empty) row. Previously the empty trailing row laid out as 0 height, so the
/// column was only 1 row tall and the cursor (row 1) was clipped away.
#[test]
fn cursor_renders_at_start_of_new_line() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "ab");
            dispatch(&view, ctx, &[TuiInputAction::InsertNewline]);
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((0, 1)), "cursor should be at row 1, col 0");
            assert!(height >= 2, "two visual rows expected, got height {height}");
        });
    });
}

/// Bug 2: an empty interior line must occupy its own row so following lines —
/// and the cursor — land on the correct visual row.
#[test]
fn interior_empty_line_does_not_collapse() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            // "a\n\nb"
            type_str(&view, ctx, "a");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::InsertNewline, TuiInputAction::InsertNewline],
            );
            type_str(&view, ctx, "b");
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(height, 3, "three visual rows expected");
            assert_eq!(cursor, Some((1, 2)), "cursor should be on the 3rd row");
        });
    });
}

/// Bug 2 (navigation): moving up from the last line lands the cursor on the
/// correct (rendered) row, not a collapsed one.
#[test]
fn move_up_through_empty_line_positions_cursor() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "a");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::InsertNewline, TuiInputAction::InsertNewline],
            );
            type_str(&view, ctx, "b");
            // Cursor on row 2 ("b"); move up to the empty row 1.
            dispatch(&view, ctx, &[TuiInputAction::MoveUp]);
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(height, 3);
            assert_eq!(
                cursor,
                Some((0, 1)),
                "cursor should be on the empty 2nd row"
            );
        });
    });
}

/// Kill bug: `Ctrl+K` from mid-line must delete from the cursor to the end of
/// the visual line (and nothing before it).
#[test]
fn kill_to_line_end_from_midline() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abcd");
            // Move cursor to just after 'b'.
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::MoveLeft, TuiInputAction::MoveLeft],
            );
            dispatch(&view, ctx, &[TuiInputAction::KillToLineEnd]);
            assert_eq!(text(&view, ctx), "ab");
        });
    });
}

/// Kill bug: `Ctrl+K` at the end of a line is a no-op (nothing after cursor).
#[test]
fn kill_to_line_end_at_eol_is_noop() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abcd");
            dispatch(&view, ctx, &[TuiInputAction::KillToLineEnd]);
            assert_eq!(text(&view, ctx), "abcd");
        });
    });
}

/// Kill bug: `Ctrl+U` from mid-line must delete from the start of the visual
/// line up to the cursor (and nothing after it).
#[test]
fn kill_to_line_start_from_midline() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abcd");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::MoveLeft, TuiInputAction::MoveLeft],
            );
            dispatch(&view, ctx, &[TuiInputAction::KillToLineStart]);
            assert_eq!(text(&view, ctx), "cd");
        });
    });
}

/// Kill + yank round-trips the killed text at the cursor.
#[test]
fn kill_then_yank_round_trips() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abcd");
            dispatch(
                &view,
                ctx,
                &[TuiInputAction::MoveLeft, TuiInputAction::MoveLeft],
            );
            dispatch(&view, ctx, &[TuiInputAction::KillToLineEnd]); // kills "cd" -> "ab"
            dispatch(&view, ctx, &[TuiInputAction::Yank]); // yanks "cd" -> "abcd"
            assert_eq!(text(&view, ctx), "abcd");
        });
    });
}

/// Bug 3: word-wise selection (Ctrl+Shift+←) extends the selection one word back.
#[test]
fn select_word_left_selects_previous_word() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            dispatch(&view, ctx, &[TuiInputAction::SelectWordLeft]);
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("world"));
        });
    });
}

/// Bug 3: word-wise selection (Ctrl+Shift+→) extends the selection one word forward.
#[test]
fn select_word_right_selects_next_word() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "hello world");
            dispatch(&view, ctx, &[TuiInputAction::MoveToLineStart]);
            dispatch(&view, ctx, &[TuiInputAction::SelectWordRight]);
            assert_eq!(selected_text(&view, ctx).as_deref(), Some("hello"));
        });
    });
}

/// Line-boundary navigation (Home/End) lands on the right column of a multi-line buffer.
#[test]
fn move_to_line_start_and_end_multiline() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "abc");
            dispatch(&view, ctx, &[TuiInputAction::InsertNewline]);
            type_str(&view, ctx, "def");
            // Cursor is at end of "def" (row 1, col 3).
            dispatch(&view, ctx, &[TuiInputAction::MoveToLineStart]);
            assert_eq!(cursor_and_height(&view, ctx).0, Some((0, 1)));
            dispatch(&view, ctx, &[TuiInputAction::MoveToLineEnd]);
            assert_eq!(cursor_and_height(&view, ctx).0, Some((3, 1)));
        });
    });
}

/// Wide (double-width) CJK characters advance the cursor by two display columns
/// each, so the rendered cursor column reflects display width, not char count.
#[test]
fn cursor_accounts_for_wide_chars() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "你好");
            let (cursor, height) = cursor_and_height(&view, ctx);
            assert_eq!(
                cursor,
                Some((4, 0)),
                "two double-width chars → cursor col 4"
            );
            assert_eq!(height, 1);
            assert_eq!(text(&view, ctx), "你好");
        });
    });
}

/// A combining mark is zero-width: it shares its base character's cell, so
/// "a\u{0301}b" occupies two display columns and the cursor ends at column 2.
#[test]
fn cursor_accounts_for_zero_width_chars() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let view = build_view(ctx);
            type_str(&view, ctx, "a\u{0301}b");
            let (cursor, _height) = cursor_and_height(&view, ctx);
            assert_eq!(cursor, Some((2, 0)), "a + combining + b → 2 display cols");
        });
    });
}
