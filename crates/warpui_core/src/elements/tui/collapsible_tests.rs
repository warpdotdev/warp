use std::cell::Cell;
use std::rc::Rc;

use super::tui_collapsible;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPoint, TuiRect, TuiSize, TuiStyle, TuiText,
};
use crate::event::ModifiersState;
use crate::{App, AppContext, EntityIdMap};

/// Lays out then renders `element` to one string per row.
fn layout_and_render(element: &mut dyn TuiElement, size: TuiSize, app: &AppContext) -> Vec<String> {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let area = TuiRect::new(0, 0, size.width, size.height);
    element.layout(TuiConstraint::loose(size), &mut ctx, app);
    let mut buffer = TuiBuffer::empty(area);
    element.render(area, &mut buffer, &mut ctx);
    buffer.to_lines()
}

fn left_mouse_down(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

/// Lays out `element` then dispatches a left click at `(x, y)`, returning
/// whether it was handled.
fn layout_and_click(
    element: &mut dyn TuiElement,
    size: TuiSize,
    x: u16,
    y: u16,
    app: &AppContext,
) -> bool {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let area = TuiRect::new(0, 0, size.width, size.height);
    element.layout(TuiConstraint::loose(size), &mut ctx, app);
    let mut event_ctx = TuiEventContext::default();
    element.dispatch_event(&left_mouse_down(x, y), area, &mut event_ctx, &mut ctx, app)
}

#[test]
fn expanded_renders_header_with_down_chevron_and_body() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut collapsible = tui_collapsible(
                false,
                "Thinking...",
                TuiStyle::default(),
                TuiText::new("reasoning"),
                |_ctx, _app| {},
            );
            let lines = layout_and_render(collapsible.as_mut(), TuiSize::new(20, 4), app_ctx);
            assert_eq!(lines[0].trim_end(), "Thinking... ▾");
            assert_eq!(lines[1].trim_end(), "reasoning");
        });
    });
}

#[test]
fn collapsed_renders_only_header_with_right_chevron() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut collapsible = tui_collapsible(
                true,
                "Thinking...",
                TuiStyle::default(),
                TuiText::new("reasoning"),
                |_ctx, _app| {},
            );
            let lines = layout_and_render(collapsible.as_mut(), TuiSize::new(20, 4), app_ctx);
            assert_eq!(lines[0].trim_end(), "Thinking... ▸");
            // The body is not rendered when collapsed.
            assert!(lines[1..].iter().all(|line| line.trim().is_empty()));
        });
    });
}

#[test]
fn header_click_invokes_on_toggle() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut collapsible = tui_collapsible(
                false,
                "Thinking...",
                TuiStyle::default(),
                TuiText::new("reasoning"),
                move |_ctx, _app| counter.set(counter.get() + 1),
            );

            // Click on the header row (row 0).
            let handled =
                layout_and_click(collapsible.as_mut(), TuiSize::new(20, 4), 2, 0, app_ctx);
            assert!(handled);
            assert_eq!(hits.get(), 1);
        });
    });
}

#[test]
fn body_click_does_not_toggle() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut collapsible = tui_collapsible(
                false,
                "Thinking...",
                TuiStyle::default(),
                TuiText::new("reasoning"),
                move |_ctx, _app| counter.set(counter.get() + 1),
            );

            // Click on the body row (row 1): the header's click handler only
            // covers its own slot, so the click is left unhandled.
            let handled =
                layout_and_click(collapsible.as_mut(), TuiSize::new(20, 4), 2, 1, app_ctx);
            assert!(!handled);
            assert_eq!(hits.get(), 0);
        });
    });
}
