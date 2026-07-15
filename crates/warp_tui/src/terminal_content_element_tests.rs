use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::TerminalModel;
use warp_terminal::model::escape_sequences::{BRACKETED_PASTE_END, BRACKETED_PASTE_START};
use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiScreenPosition, TuiSize,
};
use warpui_core::event::KeyEventDetails;
use warpui_core::keymap::Keystroke;
use warpui_core::{App, AppContext};

use super::{paste_bytes, pty_bytes_for_event, TuiTerminalContentElement};

/// A leaf that fills its constraint and retains the laid-out size.
struct FillElement {
    size: Option<TuiSize>,
}

impl TuiElement for FillElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        self.size = Some(constraint.max);
        constraint.max
    }

    fn render(
        &mut self,
        _origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }
}

fn key_down(key: &str, chars: &str, ctrl: bool) -> TuiEvent {
    TuiEvent::KeyDown {
        keystroke: Keystroke {
            key: key.to_owned(),
            ctrl,
            ..Default::default()
        },
        chars: chars.to_owned(),
        details: KeyEventDetails::default(),
        is_composing: false,
    }
}

#[test]
fn layout_measures_and_after_layout_publishes_the_size() {
    App::test((), |app| async move {
        app.read(|app| {
            let (resize_tx, resize_rx) = async_channel::unbounded();
            let mut element =
                TuiTerminalContentElement::new(resize_tx, FillElement { size: None }.finish());
            let expected_size = TuiSize::new(42, 8);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = element.layout(TuiConstraint::loose(expected_size), &mut layout_ctx, app);
            assert_eq!(size, expected_size);
            // `layout` only measures — it must not publish a resize.
            assert!(
                resize_rx.try_recv().is_err(),
                "layout should not publish a resize"
            );

            // `after_layout` publishes the settled size exactly once.
            element.after_layout(&mut layout_ctx, app);
            assert_eq!(resize_rx.try_recv().unwrap(), expected_size);
            assert!(
                resize_rx.try_recv().is_err(),
                "after_layout should publish the resize exactly once"
            );
        });
    });
}

#[test]
fn key_events_use_terminal_aware_pty_encoding() {
    let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

    assert_eq!(
        pty_bytes_for_event(&key_down("enter", "", false), &model),
        Some(vec![b'\r'])
    );
    assert_eq!(
        pty_bytes_for_event(&key_down("d", "d", true), &model),
        Some(vec![0x04])
    );
    assert_eq!(
        pty_bytes_for_event(&key_down("é", "é", false), &model),
        Some("é".as_bytes().to_vec())
    );
}

#[test]
fn composing_key_event_is_not_forwarded() {
    let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let mut event = key_down("a", "a", false);
    let TuiEvent::KeyDown { is_composing, .. } = &mut event else {
        unreachable!();
    };
    *is_composing = true;

    assert_eq!(pty_bytes_for_event(&event, &model), None);
}

#[test]
fn paste_normalizes_newlines_and_optionally_adds_bracket_markers() {
    assert_eq!(
        paste_bytes("first\nsecond\r\nthird\r", false),
        b"first\rsecond\rthird\r"
    );

    let bytes = paste_bytes("first\nsecond", true);
    assert!(bytes.starts_with(BRACKETED_PASTE_START));
    assert!(bytes.ends_with(BRACKETED_PASTE_END));
    assert_eq!(
        &bytes[BRACKETED_PASTE_START.len()..bytes.len() - BRACKETED_PASTE_END.len()],
        b"first\rsecond"
    );
}

#[test]
fn dropped_receiver_does_not_panic() {
    App::test((), |app| async move {
        app.read(|app| {
            let (resize_tx, resize_rx) = async_channel::unbounded::<TuiSize>();
            drop(resize_rx);
            let mut element =
                TuiTerminalContentElement::new(resize_tx, FillElement { size: None }.finish());
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            element.layout(
                TuiConstraint::loose(TuiSize::new(10, 4)),
                &mut layout_ctx,
                app,
            );
            // The consumer being gone (e.g. a torn-down session) is not an error.
            element.after_layout(&mut layout_ctx, app);
        });
    });
}
