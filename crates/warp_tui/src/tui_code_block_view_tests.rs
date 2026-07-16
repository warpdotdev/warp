use futures::channel::oneshot;
use warp::tui_export::Appearance;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{TuiView, ViewHandle};

use super::{
    bounded_fallback_text, TuiCodeBlockPayload, TuiCodeBlockView, TuiCodeBlockViewEvent,
    MAX_CODE_LINES, MAX_HIGHLIGHT_BYTES, TRUNCATION_NOTICE,
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
                .take_while(|line| !line.is_empty() || line.starts_with('│'))
                .collect::<Vec<_>>();
            assert_eq!(
                lines,
                vec![
                    "┌────────────────┐",
                    "│ rust           │",
                    "│ fn main() {    │",
                    "│     println!   │",
                    "│ (\"hello        │",
                    "│ world\");       │",
                    "│ }              │",
                    "└────────────────┘",
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
                if matches!(event, TuiCodeBlockViewEvent::SyntaxUpdated) {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(());
                    }
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
