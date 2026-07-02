use std::cell::Cell;
use std::rc::Rc;

use super::tui_collapsible;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPoint, TuiRect, TuiSize, TuiStyle, TuiText,
};
use crate::event::ModifiersState;
use crate::{App, AppContext, EntityIdMap};

/// Lays out `element` at 20x4 and returns its rows, trimmed of trailing blanks.
fn layout_and_render(element: &mut dyn TuiElement, app: &AppContext) -> Vec<String> {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let area = TuiRect::new(0, 0, 20, 4);
    element.layout(TuiConstraint::loose(TuiSize::new(20, 4)), &mut ctx, app);
    let mut buffer = TuiBuffer::empty(area);
    element.render(area, &mut buffer, &mut ctx);
    buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect()
}

#[test]
fn renders_header_chevron_and_body_by_state() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let build = |collapsed| {
                tui_collapsible(
                    collapsed,
                    "Thinking...",
                    TuiStyle::default(),
                    TuiText::new("reasoning").finish(),
                    |_, _| {},
                )
            };

            let expanded = layout_and_render(build(false).as_mut(), app_ctx);
            assert_eq!(expanded[0], "Thinking... ▾");
            assert_eq!(expanded[1], "reasoning");

            let collapsed = layout_and_render(build(true).as_mut(), app_ctx);
            assert_eq!(collapsed[0], "Thinking... ▸");
            assert!(collapsed[1..].iter().all(|line| line.is_empty()));
        });
    });
}

#[test]
fn only_a_header_click_invokes_on_toggle() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut collapsible = tui_collapsible(
                false,
                "Thinking...",
                TuiStyle::default(),
                TuiText::new("reasoning").finish(),
                move |_, _| counter.set(counter.get() + 1),
            );
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let area = TuiRect::new(0, 0, 20, 4);
            collapsible.layout(TuiConstraint::loose(TuiSize::new(20, 4)), &mut ctx, app_ctx);
            let mut click = |y| {
                let event = TuiEvent::LeftMouseDown {
                    position: TuiPoint::new(2, y),
                    modifiers: ModifiersState::default(),
                    click_count: 1,
                    is_first_mouse: false,
                };
                let mut event_ctx = TuiEventContext::default();
                collapsible.dispatch_event(&event, area, &mut event_ctx, &mut ctx, app_ctx)
            };

            // Row 0 is the header: the click toggles. Row 1 is the body: the
            // header's handler covers only its own slot, so it goes unhandled.
            assert!(click(0));
            assert_eq!(hits.get(), 1);
            assert!(!click(1));
            assert_eq!(hits.get(), 1);
        });
    });
}
