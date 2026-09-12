use futures::channel::oneshot;
use string_offset::CharOffset;
use warp::tui_export::Appearance;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App};
use warpui_core::elements::tui::{Color, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{TuiView, ViewHandle};

use super::{
    MAX_CODE_LINES, MAX_HIGHLIGHT_BYTES, TRUNCATION_NOTICE, TuiCodeBlockPayload, TuiCodeBlockView,
    TuiCodeBlockViewEvent, bounded_fallback_text,
};
use crate::test_fixtures::TestHostView;

#[test]
fn renders_read_only_code_with_language_and_wrapping() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(
                TuiCodeBlockPayload::new(
                    "fn main() {\n    println!(\"hello world\");\n}",
                    Some("rust".to_owned()),
                ),
                ctx,
            )
        });
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 18, 10),
                ctx,
            );
            let lines = frame
                .buffer
                .to_lines()
                .into_iter()
                .map(|line| line.trim_end().to_owned())
                .take_while(|line| !line.is_empty())
                .collect::<Vec<_>>();
            assert_eq!(
                lines,
                vec![
                    "rust",
                    "fn main() {",
                    "    println!",
                    "(\"hello world\");",
                    "}",
                ]
            );
        });
    });
}
fn add_code_view(
    app: &mut App,
    build: impl FnOnce(&mut warpui_core::ViewContext<TuiCodeBlockView>) -> TuiCodeBlockView + 'static,
) -> ViewHandle<TuiCodeBlockView> {
    app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_tui_view(window_id, build)
    })
}

#[test]
fn oversized_code_stores_a_bounded_fallback() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let code = "x".repeat(MAX_HIGHLIGHT_BYTES + 1);
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(TuiCodeBlockPayload::new(code, None), ctx)
        });
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            let fallback_text = view
                .fallback_text
                .as_deref()
                .expect("oversized code should use the fallback");
            assert_eq!(
                fallback_text.lines().next().map(str::len),
                Some(MAX_HIGHLIGHT_BYTES)
            );
            assert_eq!(fallback_text.lines().last(), Some(TRUNCATION_NOTICE));
            assert!(view.text_overrides.is_empty());
        });
    });
}

#[test]
fn fallback_bounds_the_number_of_lines() {
    let code = std::iter::repeat_n("line", MAX_CODE_LINES + 1)
        .collect::<Vec<_>>()
        .join("\n");
    let fallback_text =
        bounded_fallback_text(&code).expect("excessive lines should use the fallback");

    assert_eq!(fallback_text.lines().count(), MAX_CODE_LINES + 1);
    assert_eq!(fallback_text.lines().last(), Some(TRUNCATION_NOTICE));
}

#[test]
fn fallback_preserves_utf8_boundaries_at_the_byte_limit() {
    let prefix = "é".repeat(MAX_HIGHLIGHT_BYTES / 2);
    let code = format!("{prefix}x");
    let fallback_text =
        bounded_fallback_text(&code).expect("oversized code should use the fallback");

    assert!(fallback_text.starts_with(&prefix));
    assert_eq!(fallback_text.lines().last(), Some(TRUNCATION_NOTICE));
}
/// Creates a oneshot channel and subscribes to the view for the next
/// `SyntaxUpdated` event, resolving the receiver when the event fires.
fn wait_for_syntax_updated(
    view: &ViewHandle<TuiCodeBlockView>,
    app: &mut App,
) -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel();
    app.update(|ctx| {
        let mut tx = Some(tx);
        ctx.subscribe_to_view(view, move |_, event, _| {
            if matches!(event, TuiCodeBlockViewEvent::SyntaxUpdated)
                && let Some(tx) = tx.take()
            {
                let _ = tx.send(());
            }
        });
    });
    rx
}

#[test]
fn syntax_highlights_apply_only_to_the_latest_editor_revision() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(TuiCodeBlockPayload::new("", None), ctx)
        });
        let (tx, rx) = oneshot::channel();
        app.update(|ctx| {
            let mut tx = Some(tx);
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if matches!(event, TuiCodeBlockViewEvent::SyntaxUpdated)
                    && let Some(tx) = tx.take()
                {
                    let _ = tx.send(());
                }
            });
            view.update(ctx, |view, ctx| {
                view.sync(
                    TuiCodeBlockPayload::new("fn stale() {}", Some("rust".to_owned())),
                    ctx,
                );
                view.sync(
                    TuiCodeBlockPayload::new(
                        "def latest():\n    return 1",
                        Some("python".to_owned()),
                    ),
                    ctx,
                );
            });
        });
        rx.await.expect("latest syntax parse should complete");
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert_eq!(view.payload.code, "def latest():\n    return 1");
            assert!(!view.text_overrides.is_empty());
        });
    });
}

/// The highlight for a keyword starting at the beginning of the code must
/// cover char offset 0 (the display lattice's first character), not offset 1.
/// A range [0, N) in text_overrides colors chars 0..N-1; a range [1, N) skips
/// the first character, producing the reported 'f'-in-'for' miscoloring.
#[test]
fn keyword_highlight_covers_first_character_from_offset_zero() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // "for" is a Go keyword and must be colored from the very first character.
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(
                TuiCodeBlockPayload::new("for _, num := range numbers {", Some("go".to_owned())),
                ctx,
            )
        });
        let rx = wait_for_syntax_updated(&view, &mut app);
        rx.await.expect("syntax parse should complete");
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            // There must be at least one override that starts at display char 0
            // and ends at char 3 — covering 'f', 'o', 'r'.
            let for_range = view
                .text_overrides
                .iter()
                .find(|(r, _)| r.start == CharOffset::zero() && r.end == CharOffset::from(3));
            assert!(
                for_range.is_some(),
                "Expected 'for' keyword override at [0, 3), got: {:?}",
                view.text_overrides
            );
        });
    });
}

/// During a streaming code-only update (same language, more code), the prior
/// text_overrides must NOT be cleared while the new parse is in flight.  This
/// is the APP-5004 regression: the immediate frame between a sync and the
/// subsequent SyntaxUpdated must still show the prefix highlight, not a blank
/// (fully-unstyled) flash that users perceive as partial keyword coloring.
#[test]
fn text_overrides_preserved_during_code_only_streaming_sync() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // Phase 1: short seed — keyword at buffer start, parse completes.
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(TuiCodeBlockPayload::new("for ", Some("go".to_owned())), ctx)
        });
        let rx1 = wait_for_syntax_updated(&view, &mut app);
        rx1.await.expect("phase-1 syntax parse should complete");

        // After phase 1 the 'for' range must be present.
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(
                !view.text_overrides.is_empty(),
                "Expected non-empty text_overrides after phase-1 parse"
            );
        });

        // Phase 2: grow the code WITHOUT waiting for the new parse.
        // The text_overrides must survive the sync call so that a render
        // immediately after the sync still shows the 'for' highlight.
        app.update(|ctx| {
            view.update(ctx, |view, ctx| {
                view.sync(
                    TuiCodeBlockPayload::new(
                        "for _, num := range numbers {",
                        Some("go".to_owned()),
                    ),
                    ctx,
                );
            });
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            // The 'for' override from the prior parse must still be present at
            // the specific range [0, 3), even though the new parse has not yet
            // finished.  Asserting a non-empty vec is too weak: it passes for
            // any surviving ranges, including stale ones at the wrong offsets.
            let for_override = view
                .text_overrides
                .iter()
                .find(|(r, _)| r.start == CharOffset::zero() && r.end == CharOffset::from(3));
            assert!(
                for_override.is_some(),
                "text_overrides must retain the 'for' keyword override [0, 3) during a \
                 code-only streaming sync (the new parse is still in flight); \
                 got: {:?}",
                view.text_overrides
            );
        });
    });
}

/// After a phase-1 parse completes, a phase-2 sync (same language, appended
/// code, no parse wait) must produce a rendered mid-stream frame where the
/// 'for' keyword retains the syntax-highlight fg color it had in the phase-1
/// frame.  Without the fix (text_overrides always cleared), the mid-stream
/// frame reverts to the base text color at those cells.
#[test]
fn mid_stream_frame_retains_keyword_coloring() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // Phase 1: short seed — keyword at buffer start, parse completes.
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(TuiCodeBlockPayload::new("for ", Some("go".to_owned())), ctx)
        });
        let rx1 = wait_for_syntax_updated(&view, &mut app);
        rx1.await.expect("phase-1 syntax parse should complete");

        // Capture the 'f' cell's fg color from the settled phase-1 render.
        // The code block renders as:
        //   row 0: ┌──────────────────────────────────────┐  (border)
        //   row 1: │ go                                   │  (language)
        //   row 2: │ for …                                │  (code)
        //   row 3: └──────────────────────────────────────┘  (border)
        // 'f' is at column 2 (│=0, space=1, f=2), row 2.
        let phase1_f_fg = app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 40, 6),
                ctx,
            );
            frame.buffer[(2, 2)].fg
        });
        // Sanity-check: the settled frame must have a non-Reset fg so that the
        // comparison below is meaningful.  If syntax highlighting produced no
        // color, phase1_f_fg would be Reset and the test would be vacuous.
        assert_ne!(
            phase1_f_fg,
            Color::Reset,
            "Phase-1 'f' cell must have a syntax-highlight fg color, got Reset"
        );

        // Phase 2: grow the code WITHOUT waiting for the new parse.
        app.update(|ctx| {
            view.update(ctx, |view, ctx| {
                view.sync(
                    TuiCodeBlockPayload::new(
                        "for _, num := range numbers {",
                        Some("go".to_owned()),
                    ),
                    ctx,
                );
            });
        });

        // Render the mid-stream frame (new parse still in flight).
        let midstream_f_fg = app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 40, 6),
                ctx,
            );
            frame.buffer[(2, 2)].fg
        });

        // The syntax-highlight color must be retained in the mid-stream frame.
        // Without the fix, text_overrides were cleared by sync(), causing the
        // 'f' cell to revert to the base text color while the new parse ran.
        assert_eq!(
            phase1_f_fg, midstream_f_fg,
            "Mid-stream frame must retain the 'for' keyword fg color from the \
             phase-1 parse; phase-1 fg = {phase1_f_fg:?}, mid-stream fg = {midstream_f_fg:?}"
        );
    });
}

/// When code is *rewritten* (not appended) under the same language, the retained
/// text_overrides from the prior parse would map to unrelated new text.  The
/// prefix guard must detect the non-append and clear the overrides so stale
/// highlights do not appear.
#[test]
fn text_overrides_cleared_when_code_is_rewritten() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // Phase 1: parse "range foo {" — highlights land on specific offsets.
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(
                TuiCodeBlockPayload::new("range foo {", Some("go".to_owned())),
                ctx,
            )
        });
        let rx = wait_for_syntax_updated(&view, &mut app);
        rx.await.expect("syntax parse should complete");
        app.read(|ctx| {
            assert!(
                !view.as_ref(ctx).text_overrides.is_empty(),
                "text_overrides must be non-empty after phase-1 parse"
            );
        });

        // Phase 2: replace with code whose first char differs from phase-1.
        // "for _, num..." does NOT start with "range foo {", so the prefix
        // guard must clear the overrides immediately.
        app.update(|ctx| {
            view.update(ctx, |view, ctx| {
                view.sync(
                    TuiCodeBlockPayload::new(
                        "for _, num := range numbers {",
                        Some("go".to_owned()),
                    ),
                    ctx,
                );
            });
        });

        app.read(|ctx| {
            assert!(
                view.as_ref(ctx).text_overrides.is_empty(),
                "text_overrides must be cleared when code is rewritten (not \
                 appended); retained ranges would transiently style unrelated new text"
            );
        });
    });
}

/// After syncing "format(x)" from phase-1 "for", the 'for' override must be
/// dropped even though "format(x)" starts with "for", because the 'for' range
/// ends exactly at the previous code boundary (the token was still mid-stream).
/// Without this fix the retained `[0, 3)` override would color only `f/o/r` in
/// keyword magenta while `m/a/t/(x)/()` rendered in base grey — an
/// intra-token split in a single frame.
#[test]
fn mid_stream_intra_token_fix_prevents_keyword_prefix_coloring() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // Phase 1: parse "for" with no trailing space so 'for' ends AT the
        // code boundary (prev_end == 3 == range.end).
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(TuiCodeBlockPayload::new("for", Some("go".to_owned())), ctx)
        });
        let rx1 = wait_for_syntax_updated(&view, &mut app);
        rx1.await.expect("phase-1 syntax parse should complete");

        // Phase 2: sync "format(x)" (starts_with "for"), no parse wait.
        // The 'for' override at [0, 3) must be dropped because range.end (3)
        // equals prev_end (3) — that token was growing.
        app.update(|ctx| {
            view.update(ctx, |view, ctx| {
                view.sync(
                    TuiCodeBlockPayload::new("format(x)", Some("go".to_owned())),
                    ctx,
                );
            });
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            let for_override = view
                .text_overrides
                .iter()
                .find(|(r, _)| r.start == CharOffset::zero() && r.end == CharOffset::from(3));
            assert!(
                for_override.is_none(),
                "'for' override [0,3) must be dropped when the token ended at \
                 prev_code's boundary (it was still growing into 'format'); \
                 got text_overrides = {:?}",
                view.text_overrides
            );
        });

        // Render the mid-stream frame and confirm no intra-token color split:
        // all chars of "format(x)" must have the same fg (uniformly unstyled).
        // Buffer is indexed (x=col, y=row). Layout:
        //   row 0: ┌ border
        //   row 1: │ go   (language)
        //   row 2: │ format(x) ... (code)
        //   row 3: └ border
        // Code chars: col 0=│, col 1=space, col 2..=10 = f,o,r,m,a,t,(,x,)
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 40, 6),
                ctx,
            );
            let chars_fg: Vec<_> = (2..11usize)
                .map(|col| frame.buffer[(col as u16, 2)].fg) // (x=col, y=row)
                .collect();
            // Verify all chars have the same fg — no intra-token split.
            let first_fg = chars_fg[0];
            for (i, &fg) in chars_fg.iter().enumerate() {
                assert_eq!(
                    first_fg,
                    fg,
                    "Char at col {} has fg={fg:?} but first char has fg={first_fg:?}; \
                     mid-stream frame must not split keyword prefix from identifier suffix. \
                     All fgs: {chars_fg:?}",
                    i + 2
                );
            }
        });
    });
}

/// The highlight for a Python `def` keyword at the beginning of a code block
/// must cover char offset 0 from the first character, satisfying acceptance
/// criterion 1 (coverage in Go and at least one other language).
#[test]
fn python_keyword_highlight_covers_first_character() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(
                TuiCodeBlockPayload::new("def foo():", Some("python".to_owned())),
                ctx,
            )
        });
        let rx = wait_for_syntax_updated(&view, &mut app);
        rx.await.expect("syntax parse should complete");
        app.read(|ctx| {
            let view = view.as_ref(ctx);
            let def_range = view
                .text_overrides
                .iter()
                .find(|(r, _)| r.start == CharOffset::zero() && r.end == CharOffset::from(3));
            assert!(
                def_range.is_some(),
                "Expected 'def' keyword override at [0, 3) in Python code block, \
                 got: {:?}",
                view.text_overrides
            );
        });
    });
}

/// After a streaming growth (short → longer code, both syncs waited on), the
/// settled highlights must still cover every character of every keyword,
/// including the very first character.  This catches the streaming/paint race
/// described in APP-5004 where stale or misaligned text_overrides from the
/// first parse cycle could persist into renders of the longer content.
#[test]
fn keyword_highlight_covers_first_character_after_streaming_growth() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // Phase 1: short seed — just the keyword at the buffer start.
        let view = add_code_view(&mut app, |ctx| {
            TuiCodeBlockView::new(TuiCodeBlockPayload::new("for ", Some("go".to_owned())), ctx)
        });
        let rx1 = wait_for_syntax_updated(&view, &mut app);
        rx1.await.expect("phase-1 syntax parse should complete");

        // Phase 2: grow the code, simulating a streaming update.
        let rx2 = wait_for_syntax_updated(&view, &mut app);
        app.update(|ctx| {
            view.update(ctx, |view, ctx| {
                view.sync(
                    TuiCodeBlockPayload::new(
                        "for _, num := range numbers {",
                        Some("go".to_owned()),
                    ),
                    ctx,
                );
            });
        });
        rx2.await.expect("phase-2 syntax parse should complete");

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            // 'for' must start at display char 0 (the 'f').
            let for_range = view
                .text_overrides
                .iter()
                .find(|(r, _)| r.start == CharOffset::zero() && r.end == CharOffset::from(3));
            assert!(
                for_range.is_some(),
                "After streaming growth, 'for' override must start at char 0; got: {:?}",
                view.text_overrides
            );
            // 'range' keyword starts at char 14 (0-indexed) and spans 5 chars.
            let range_range = view
                .text_overrides
                .iter()
                .find(|(r, _)| r.start == CharOffset::from(14) && r.end == CharOffset::from(19));
            assert!(
                range_range.is_some(),
                "After streaming growth, 'range' override must cover [14, 19); got: {:?}",
                view.text_overrides
            );
        });
    });
}
