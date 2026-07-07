use std::cell::Cell;
use std::rc::Rc;

use super::tui_collapsible;
use crate::elements::tui::test_support::dispatch_presented_event;
use crate::elements::tui::{TuiElement, TuiEvent, TuiPoint, TuiRect, TuiStyle, TuiText};
use crate::elements::MouseStateHandle;
use crate::event::ModifiersState;
use crate::presenter::tui::TuiPresenter;
use crate::App;

#[test]
fn only_a_header_click_invokes_on_toggle() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let collapsible = tui_collapsible(
                false,
                [("Thinking...".to_owned(), TuiStyle::default())],
                TuiStyle::default(),
                MouseStateHandle::default(),
                || TuiText::new("reasoning").finish(),
                move |_, _| counter.set(counter.get() + 1),
            );
            let area = TuiRect::new(0, 0, 20, 4);
            let mut presenter = TuiPresenter::new();
            presenter.present_element(collapsible, area, app_ctx);
            // A click is a press-then-release pair; the hoverable's arming
            // notify needs an origin view to attribute the redraw to.
            let mut click = |x, y| {
                let down = TuiEvent::LeftMouseDown {
                    position: TuiPoint::new(x, y),
                    modifiers: ModifiersState::default(),
                    click_count: 1,
                    is_first_mouse: false,
                };
                let pressed = dispatch_presented_event(&mut presenter, &down, app_ctx).0;
                let up = TuiEvent::LeftMouseUp {
                    position: TuiPoint::new(x, y),
                    modifiers: ModifiersState::default(),
                };
                let released = dispatch_presented_event(&mut presenter, &up, app_ctx).0;
                pressed && released
            };

            // Row 0 is the header: the click toggles. Row 1 is the body: the
            // header's handler covers only its own slot, so it goes unhandled.
            assert!(click(2, 0));
            assert_eq!(hits.get(), 1);
            assert!(!click(2, 1));
            assert_eq!(hits.get(), 1);

            // The blank space right of the label + chevron ("Thinking... ▾"
            // spans 13 columns) is not part of the click target.
            assert!(!click(15, 0));
            assert_eq!(hits.get(), 1);
        });
    });
}
