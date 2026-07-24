use std::cell::Cell;
use std::rc::Rc;

use super::tui_collapsible;
use crate::elements::MouseStateHandle;
use crate::elements::tui::test_support::{dispatch_presented_event, with_paint_surface};
use crate::elements::tui::{
    Modifier, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiLayoutContext,
    TuiPoint, TuiRect, TuiScreenPosition, TuiSize, TuiStyle, TuiText,
};
use crate::event::ModifiersState;
use crate::presenter::tui::TuiPresenter;
use crate::{App, EntityIdMap};

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

#[test]
fn header_styles_apply_per_span_without_bleeding_past_the_text() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // A plain glyph span beside an underlined label span; the
            // chevron carries its own style. Nothing may bleed past the
            // header text into the row's trailing cells.
            let underlined = TuiStyle::default().add_modifier(Modifier::UNDERLINED);
            let mut collapsible = tui_collapsible(
                false,
                [
                    ("☰ ".to_owned(), TuiStyle::default()),
                    ("Tasks 3".to_owned(), underlined),
                ],
                TuiStyle::default(),
                MouseStateHandle::default(),
                || TuiText::new("body").finish(),
                |_, _| {},
            );
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let area = TuiRect::new(0, 0, 20, 2);
            collapsible.layout(TuiConstraint::loose(TuiSize::new(20, 2)), &mut ctx, app_ctx);
            let mut buffer = TuiBuffer::empty(area);
            with_paint_surface(&mut buffer, |surface, paint_ctx| {
                collapsible.render(TuiScreenPosition::new(0, 0), surface, paint_ctx)
            });

            // The underline covers exactly the label's cells — not the
            // glyph, the chevron, or the trailing cells past the text. The
            // label's start column is located from the buffer since the
            // glyph's cell width varies by rendering backend.
            assert_eq!(buffer.to_lines()[0].trim_end(), "☰ Tasks 3 ▾");
            let label_start = (0..20u16)
                .find(|&x| buffer[(x, 0)].symbol() == "T")
                .expect("the header row contains the label");
            let underlined: Vec<u16> = (0..20u16)
                .filter(|&x| buffer[(x, 0)].modifier.contains(Modifier::UNDERLINED))
                .collect();
            // "Tasks 3" spans seven cells.
            let label_cells: Vec<u16> = (label_start..label_start + 7).collect();
            assert_eq!(underlined, label_cells);
        });
    });
}
