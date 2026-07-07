use std::cell::Cell;
use std::rc::Rc;

use super::tui_collapsible;
use crate::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPoint, TuiRect,
    TuiSize, TuiStyle, TuiText,
};
use crate::elements::MouseStateHandle;
use crate::event::ModifiersState;
use crate::{App, EntityIdMap};

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
                TuiStyle::default(),
                MouseStateHandle::default(),
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
