use std::collections::HashMap;

use super::{TuiScrollHandle, TuiScrollable};
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext, TuiRect,
    TuiSize, TuiText,
};
use crate::event::{KeyEventDetails, ModifiersState};
use crate::geometry::vector::{vec2f, Vector2F};
use crate::keymap::Keystroke;
use crate::{App, Event};

/// Content with ten one-character rows ("0".."9"), so a rendered window names
/// exactly the rows it shows.
fn digits() -> TuiText {
    TuiText::new("0\n1\n2\n3\n4\n5\n6\n7\n8\n9").truncate()
}

/// Lays the element out at `size` (as the runtime does before paint) and returns
/// the rendered rows.
fn lines(element: &mut dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    element.layout(TuiConstraint::tight(size), &mut ctx);
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
    element.render(TuiRect::new(0, 0, size.width, size.height), &mut buffer, &mut ctx);
    buffer.to_lines()
}

fn scroll_wheel(position: Vector2F, delta_y: f32) -> Event {
    Event::ScrollWheel {
        position,
        delta: vec2f(0.0, delta_y),
        precise: false,
        modifiers: ModifiersState::default(),
    }
}

fn key(name: &str) -> Event {
    Event::KeyDown {
        keystroke: Keystroke {
            key: name.to_owned(),
            ..Default::default()
        },
        chars: String::new(),
        details: KeyEventDetails::default(),
        is_composing: false,
    }
}

#[test]
fn shows_the_top_of_the_content_at_offset_zero() {
    let mut scrollable = TuiScrollable::new(TuiScrollHandle::new(), digits());
    assert_eq!(
        lines(&mut scrollable, TuiSize::new(1, 3)),
        vec!["0", "1", "2"]
    );
}

#[test]
fn shows_a_scrolled_window_clipping_above_and_below() {
    let handle = TuiScrollHandle::new();
    handle.set_offset(4);
    let mut scrollable = TuiScrollable::new(handle, digits());
    assert_eq!(
        lines(&mut scrollable, TuiSize::new(1, 3)),
        vec!["4", "5", "6"]
    );
}

#[test]
fn clamps_offset_to_the_bottom() {
    let handle = TuiScrollHandle::new();
    handle.set_offset(100);
    let mut scrollable = TuiScrollable::new(handle.clone(), digits());
    // 10 rows in a 3-row viewport => max offset 7 => the last three rows show.
    assert_eq!(
        lines(&mut scrollable, TuiSize::new(1, 3)),
        vec!["7", "8", "9"]
    );
    assert_eq!(
        handle.offset(),
        7,
        "layout clamps the stored offset to the bottom"
    );
}

#[test]
fn does_not_scroll_when_the_content_fits() {
    let handle = TuiScrollHandle::new();
    handle.set_offset(5);
    let mut scrollable = TuiScrollable::new(handle.clone(), TuiText::new("a\nb").truncate());
    // Two content rows in a four-row viewport: clamped to the top, rest blank.
    assert_eq!(
        lines(&mut scrollable, TuiSize::new(1, 4)),
        vec!["a", "b", " ", " "],
    );
    assert_eq!(handle.offset(), 0);
}

#[test]
fn mouse_wheel_inside_the_viewport_scrolls() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let handle = TuiScrollHandle::new();
            let mut scrollable = TuiScrollable::new(handle.clone(), digits());
            let area = TuiRect::new(0, 0, 1, 3);
    let mut rendered_views_for_setup = HashMap::new();
    let mut setup_ctx = TuiLayoutContext { rendered_views: &mut rendered_views_for_setup };
    scrollable.layout(TuiConstraint::tight(area.as_size()), &mut setup_ctx);

            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = HashMap::new();
            let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
            // Wheel down (delta.y = -1) scrolls toward the bottom by WHEEL_STEP.
            let handled = scrollable.dispatch_event(
                &scroll_wheel(vec2f(0.0, 0.0), -1.0),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );

            assert!(handled);
            assert_eq!(handle.offset(), 3);
        });
    });
}

#[test]
fn mouse_wheel_outside_the_viewport_is_ignored() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let handle = TuiScrollHandle::new();
            let mut scrollable = TuiScrollable::new(handle.clone(), digits());
            let area = TuiRect::new(0, 0, 1, 3);
            let mut rendered_views_setup = HashMap::new();
            let mut ctx_setup = TuiLayoutContext { rendered_views: &mut rendered_views_setup };
            scrollable.layout(TuiConstraint::tight(area.as_size()), &mut ctx_setup);

            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = HashMap::new();
            let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
            // Row 5 lies below the three-row viewport.
            let handled = scrollable.dispatch_event(
                &scroll_wheel(vec2f(0.0, 5.0), -1.0),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );

            assert!(!handled);
            assert_eq!(handle.offset(), 0);
        });
    });
}

#[test]
fn keyboard_scrolling_moves_by_line_page_and_to_the_ends() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let handle = TuiScrollHandle::new();
            let mut scrollable = TuiScrollable::new(handle.clone(), digits());
            // Content 10, viewport 3 => max offset 7, page = viewport - 1 = 2.
            let area = TuiRect::new(0, 0, 1, 3);
            let mut dispatch = |name: &str| {
                let mut rendered_views = HashMap::new();
                let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
                scrollable.layout(TuiConstraint::tight(area.as_size()), &mut ctx);
                let mut event_ctx = TuiEventContext::default();
                scrollable.dispatch_event(&key(name), area, &mut event_ctx, &mut ctx, app_ctx)
            };

            assert!(dispatch("down"));
            assert_eq!(handle.offset(), 1);
            assert!(dispatch("pagedown"));
            assert_eq!(handle.offset(), 3);
            assert!(dispatch("up"));
            assert_eq!(handle.offset(), 2);
            assert!(dispatch("pageup"));
            assert_eq!(handle.offset(), 0);
            assert!(dispatch("end"));
            assert_eq!(handle.offset(), 7);
            assert!(dispatch("home"));
            assert_eq!(handle.offset(), 0);
            // A no-op scroll (already at the top) is left unhandled so other
            // handlers still see the key.
            assert!(!dispatch("up"));
        });
    });
}

#[test]
fn offset_persists_in_the_handle_across_rebuilt_elements() {
    let handle = TuiScrollHandle::new();
    {
        let mut first = TuiScrollable::new(handle.clone(), digits());
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    first.layout(TuiConstraint::tight(TuiSize::new(1, 3)), &mut ctx);
        handle.set_offset(5);
    }
    // A freshly built element (as a re-render produces) sees the same offset.
    let mut second = TuiScrollable::new(handle.clone(), digits());
    assert_eq!(lines(&mut second, TuiSize::new(1, 3)), vec!["5", "6", "7"]);
    assert_eq!(handle.offset(), 5);
}

#[test]
fn preserves_wide_glyph_columns_through_the_blit() {
    let handle = TuiScrollHandle::new();
    handle.set_offset(1);
    // After scrolling, the visible window starts on the wide CJK row, which must
    // still render across both of its columns once copied out of the off-screen
    // buffer.
    let mut scrollable = TuiScrollable::new(handle, TuiText::new("a\n世\nb").truncate());
    assert_eq!(lines(&mut scrollable, TuiSize::new(2, 2)), vec!["世", "b "]);
}
