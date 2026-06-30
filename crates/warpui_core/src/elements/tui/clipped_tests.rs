use std::cell::RefCell;
use std::rc::Rc;

use super::TuiClipped;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPoint, TuiRect, TuiSize, TuiText,
};
use crate::event::{KeyEventDetails, ModifiersState};
use crate::keymap::Keystroke;
use crate::{App, AppContext, EntityIdMap};

fn render_to_lines(element: &mut dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    element.render(area, &mut buffer, &mut ctx);
    buffer.to_lines()
}

#[test]
fn renders_from_the_requested_logical_row() {
    let mut clipped = TuiClipped::new(TuiText::new("a\nb\nc").truncate()).with_viewport_origin_y(1);

    assert_eq!(
        render_to_lines(&mut clipped, TuiSize::new(3, 2)),
        vec!["b  ", "c  "],
    );
}

#[test]
fn layout_preserves_child_width_and_reports_visible_height() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut clipped =
                TuiClipped::new(TuiText::new("a\nb\nc").truncate()).with_viewport_origin_y(1);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = clipped.layout(TuiConstraint::loose(TuiSize::new(3, 10)), &mut ctx, app_ctx);

            assert_eq!(size, TuiSize::new(1, 2));
        });
    });
}

struct CursorElement {
    cursor: (u16, u16),
}

impl TuiElement for CursorElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(1, 3))
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {}

    fn cursor_position(&self, _area: TuiRect, _ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        Some(self.cursor)
    }
}

#[test]
fn cursor_position_is_shifted_into_the_visible_window() {
    let clipped = TuiClipped::new(CursorElement { cursor: (0, 2) }).with_viewport_origin_y(1);
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };

    assert_eq!(
        clipped.cursor_position(TuiRect::new(0, 0, 3, 2), &mut ctx),
        Some((0, 1)),
    );
}

#[test]
fn cursor_position_above_the_visible_window_is_hidden() {
    let clipped = TuiClipped::new(CursorElement { cursor: (0, 0) }).with_viewport_origin_y(1);
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };

    assert_eq!(
        clipped.cursor_position(TuiRect::new(0, 0, 3, 2), &mut ctx),
        None,
    );
}

// A child element that records the `area` it received in `dispatch_event`,
// used to verify `TuiClipped` translates coordinates correctly.
struct DispatchRecorder {
    seen_area: Rc<RefCell<Option<TuiRect>>>,
    handle: bool,
}

impl TuiElement for DispatchRecorder {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        // Claim 3 rows so origin y=1 leaves a 2-row visible window.
        constraint.clamp(TuiSize::new(1, 3))
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {}

    fn dispatch_event(
        &mut self,
        _event: &TuiEvent,
        area: TuiRect,
        _event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> bool {
        *self.seen_area.borrow_mut() = Some(area);
        self.handle
    }
}

fn left_mouse_down(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

#[test]
fn dispatch_translates_mouse_event_to_full_logical_area() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let seen_area = Rc::new(RefCell::new(None));
            let mut clipped = TuiClipped::new(DispatchRecorder {
                seen_area: seen_area.clone(),
                handle: true,
            })
            .with_viewport_origin_y(1);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let mut event_ctx = TuiEventContext::default();
            // Visible slot (0, 1, 3, 2); the child's logical row 1 maps to y=1.
            let area = TuiRect::new(0, 1, 3, 2);
            let event = left_mouse_down(0, 1);
            let handled = clipped.dispatch_event(&event, area, &mut event_ctx, &mut ctx, app_ctx);
            assert!(handled);
            // Child sees its full logical area: y = 1 - 1 = 0, height = 2 + 1 = 3.
            assert_eq!(*seen_area.borrow(), Some(TuiRect::new(0, 0, 3, 3)));
        });
    });
}

#[test]
fn dispatch_filters_mouse_events_outside_visible_window() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let seen_area = Rc::new(RefCell::new(None));
            let mut clipped = TuiClipped::new(DispatchRecorder {
                seen_area: seen_area.clone(),
                handle: true,
            })
            .with_viewport_origin_y(1);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let mut event_ctx = TuiEventContext::default();
            let area = TuiRect::new(0, 1, 3, 2);
            // Click at y=0, above the visible window [1, 3).
            let event = left_mouse_down(0, 0);
            let handled = clipped.dispatch_event(&event, area, &mut event_ctx, &mut ctx, app_ctx);
            assert!(!handled);
            assert!(
                seen_area.borrow().is_none(),
                "child should not see an out-of-window event"
            );
        });
    });
}

#[test]
fn dispatch_forwards_non_positional_events_without_filtering() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let seen_area = Rc::new(RefCell::new(None));
            let mut clipped = TuiClipped::new(DispatchRecorder {
                seen_area: seen_area.clone(),
                handle: true,
            })
            .with_viewport_origin_y(1);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let mut event_ctx = TuiEventContext::default();
            let area = TuiRect::new(0, 1, 3, 2);
            // A key event carries no position, so it bypasses the window filter
            // and still reaches the child with its full logical area.
            let event = TuiEvent::KeyDown {
                keystroke: Keystroke {
                    key: "a".to_owned(),
                    ..Default::default()
                },
                chars: "a".to_owned(),
                details: KeyEventDetails::default(),
                is_composing: false,
            };
            let handled = clipped.dispatch_event(&event, area, &mut event_ctx, &mut ctx, app_ctx);
            assert!(handled);
            assert_eq!(*seen_area.borrow(), Some(TuiRect::new(0, 0, 3, 3)));
        });
    });
}
