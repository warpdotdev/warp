use std::cell::{Cell, RefCell};
use std::rc::Rc;

use warp::tui_export::Appearance;
use warpui::event::ModifiersState;
use warpui::{App, EntityId, EntityIdMap};
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiContainer, TuiElement, TuiEvent, TuiEventContext,
    TuiFlex, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPoint, TuiRect,
    TuiScreenPosition, TuiSelectionHandle, TuiSize, TuiViewportPosition, TuiViewportedListState,
};

use super::{
    TuiReadOnlyMenu, TuiReadOnlyMenuRow, TuiReadOnlyMenuSection, TuiReadOnlyMenuText,
    TuiReadOnlyMenuVisualRow,
};
use crate::tui_builder::TuiUiBuilder;

fn render(app: &App, element: &mut dyn TuiElement, size: TuiSize) -> TuiBuffer {
    render_with_constraint(app, element, TuiConstraint::tight(size))
}

fn render_with_constraint(
    app: &App,
    element: &mut dyn TuiElement,
    constraint: TuiConstraint,
) -> TuiBuffer {
    app.read(|ctx| {
        let mut rendered_views = EntityIdMap::default();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let size = element.layout(constraint, &mut layout_ctx, ctx);
        let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        {
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
        }
        buffer
    })
}

fn dispatch_mouse(app: &App, element: &mut dyn TuiElement, size: TuiSize, event: TuiEvent) -> bool {
    app.read(|ctx| {
        let mut rendered_views = EntityIdMap::default();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        element.layout(TuiConstraint::tight(size), &mut layout_ctx, ctx);
        let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        {
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
        }
        let scene = Rc::new(paint_ctx.scene.clone());
        drop(paint_ctx);
        let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
        event_ctx.set_origin_view(Some(EntityId::new()));
        element.dispatch_event(&event, &mut event_ctx, ctx)
    })
}

fn left_down(x: u16, y: u16) -> TuiEvent {
    left_down_with_click_count(x, y, 1)
}

fn left_down_with_click_count(x: u16, y: u16, click_count: u32) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count,
        is_first_mouse: false,
    }
}

fn left_drag(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDragged {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
    }
}

fn left_up(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseUp {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
    }
}
fn scroll_wheel(x: u16, y: u16, delta_y: isize) -> TuiEvent {
    TuiEvent::ScrollWheel {
        position: TuiPoint::new(x, y),
        delta: (0, delta_y),
        precise: false,
        modifiers: ModifiersState::default(),
    }
}

fn numbered_menu(builder: &TuiUiBuilder) -> TuiReadOnlyMenu {
    let rows = (0..6)
        .map(|index| {
            TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([(
                format!("Row {index}"),
                builder.primary_text_style(),
            )])])
        })
        .collect();
    TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new("Rows 6", rows)])
}

#[test]
fn visual_rows_own_the_full_width_background() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (mut element, background) = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let row = TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([(
                "Version".to_owned(),
                builder.primary_text_style(),
            )])]);
            (
                TuiReadOnlyMenuVisualRow::Content(row).render(builder.read_only_menu_background()),
                builder.read_only_menu_background(),
            )
        });

        let buffer = render_with_constraint(
            &app,
            element.as_mut(),
            TuiConstraint::loose(TuiSize::new(40, 1)),
        );

        assert_eq!(buffer.area.width, 40);
        assert_eq!(buffer[(0, 0)].style().bg, Some(background));
        assert_eq!(buffer[(39, 0)].style().bg, Some(background));
    });
}

#[test]
fn background_fills_available_width_under_loose_constraints() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (mut element, background) = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let row = TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([(
                "Version".to_owned(),
                builder.primary_text_style(),
            )])]);
            (
                TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new("Status", vec![row])])
                    .render(
                        TuiSelectionHandle::default(),
                        &builder,
                        |_, _| {},
                        |_, _, _| {},
                    ),
                builder.read_only_menu_background(),
            )
        });

        let buffer = render_with_constraint(
            &app,
            element.as_mut(),
            TuiConstraint::loose(TuiSize::new(40, 2)),
        );

        assert_eq!(buffer.area.width, 40);
        for row in 0..buffer.area.height {
            assert_eq!(buffer[(0, row)].style().bg, Some(background));
            assert_eq!(buffer[(39, row)].style().bg, Some(background));
        }
    });
}

#[test]
fn background_fills_available_width_through_session_style_wrapper() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (mut element, background) = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let row = TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([(
                "Version".to_owned(),
                builder.primary_text_style(),
            )])]);
            let menu = TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new("Status", vec![row])])
                .render(
                    TuiSelectionHandle::default(),
                    &builder,
                    |_, _| {},
                    |_, _, _| {},
                );
            (
                TuiFlex::column()
                    .child(TuiContainer::new(menu).with_padding_top(1).finish())
                    .finish(),
                builder.read_only_menu_background(),
            )
        });

        let buffer = render_with_constraint(
            &app,
            element.as_mut(),
            TuiConstraint::loose(TuiSize::new(40, 3)),
        );

        assert_eq!(buffer.area.width, 40);
        for row in 1..buffer.area.height {
            assert_eq!(buffer[(0, row)].style().bg, Some(background));
            assert_eq!(buffer[(39, row)].style().bg, Some(background));
        }
    });
}

#[test]
fn wheel_scrolling_persists_across_read_only_menu_rebuilds() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let viewport = TuiViewportedListState::new_at_end();
        viewport.scroll_to_rows_from_top(0);
        let mut element = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            numbered_menu(&builder).render_with_viewport(
                TuiSelectionHandle::default(),
                viewport.clone(),
                &builder,
                |_, _| {},
                |_, _, _| {},
            )
        });
        let size = TuiSize::new(20, 3);
        assert_eq!(
            render(&app, element.as_mut(), size)
                .to_lines()
                .into_iter()
                .map(|line| line.trim().to_owned())
                .collect::<Vec<_>>(),
            vec!["Rows 6", "Row 0", "Row 1"],
        );

        assert!(dispatch_mouse(
            &app,
            element.as_mut(),
            size,
            scroll_wheel(1, 1, -1),
        ));
        assert_eq!(viewport.position(), TuiViewportPosition::RowsFromTop(2));

        let mut rebuilt = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            numbered_menu(&builder).render_with_viewport(
                TuiSelectionHandle::default(),
                viewport.clone(),
                &builder,
                |_, _| {},
                |_, _, _| {},
            )
        });
        assert_eq!(
            render(&app, rebuilt.as_mut(), size)
                .to_lines()
                .into_iter()
                .map(|line| line.trim().to_owned())
                .collect::<Vec<_>>(),
            vec!["Row 1", "Row 2", "Row 3"],
        );
    });
}
#[test]
fn selection_spans_section_titles_and_rows() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let starts = Rc::new(Cell::new(0));
        let copies = Rc::new(RefCell::new(Vec::new()));
        let starts_for_callback = starts.clone();
        let copies_for_callback = copies.clone();
        let mut element = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let row = TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([(
                "Version".to_owned(),
                builder.primary_text_style(),
            )])]);
            TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new("Status", vec![row])]).render(
                TuiSelectionHandle::default(),
                &builder,
                move |_, _| starts_for_callback.set(starts_for_callback.get() + 1),
                move |text, _, _| copies_for_callback.borrow_mut().push(text),
            )
        });
        let size = TuiSize::new(40, 2);

        let _ = render(&app, element.as_mut(), size);
        assert!(dispatch_mouse(
            &app,
            element.as_mut(),
            size,
            left_down(1, 0)
        ));
        assert!(dispatch_mouse(
            &app,
            element.as_mut(),
            size,
            left_drag(7, 1)
        ));
        assert!(dispatch_mouse(&app, element.as_mut(), size, left_up(7, 1)));

        assert_eq!(starts.get(), 1);
        assert_eq!(copies.borrow().as_slice(), ["Status\nVersion"]);
    });
}

#[test]
fn selection_stops_at_trailing_whitespace() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let starts = Rc::new(Cell::new(0));
        let copies = Rc::new(RefCell::new(Vec::new()));
        let starts_for_callback = starts.clone();
        let copies_for_callback = copies.clone();
        let (mut element, selection_style) = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let row = TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([
                (format!("{:<19}", "Email"), builder.dim_text_style()),
                ("moira@example.com".to_owned(), builder.primary_text_style()),
            ])]);
            (
                TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new("Status", vec![row])])
                    .render(
                        TuiSelectionHandle::default(),
                        &builder,
                        move |_, _| starts_for_callback.set(starts_for_callback.get() + 1),
                        move |text, _, _| copies_for_callback.borrow_mut().push(text),
                    ),
                builder.selection_style(),
            )
        });
        let size = TuiSize::new(40, 2);

        let _ = render(&app, element.as_mut(), size);
        assert!(!dispatch_mouse(
            &app,
            element.as_mut(),
            size,
            left_down(38, 1)
        ));
        assert_eq!(starts.get(), 0);

        assert!(dispatch_mouse(
            &app,
            element.as_mut(),
            size,
            left_down(20, 1)
        ));
        assert!(dispatch_mouse(
            &app,
            element.as_mut(),
            size,
            left_drag(38, 1)
        ));
        let buffer = render(&app, element.as_mut(), size);
        assert_eq!(buffer[(36, 1)].style().bg, selection_style.bg);
        assert_ne!(buffer[(37, 1)].style().bg, selection_style.bg);
        assert!(dispatch_mouse(&app, element.as_mut(), size, left_up(38, 1)));

        assert_eq!(copies.borrow().as_slice(), ["moira@example.com"]);
    });
}

#[test]
fn double_click_selects_complete_styled_text() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let copies = Rc::new(RefCell::new(Vec::new()));
        let copies_for_callback = copies.clone();
        let mut element = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let field_row = |label: &str, value: &str| {
                TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([
                    (format!("{label:<19}"), builder.dim_text_style()),
                    (value.to_owned(), builder.primary_text_style()),
                ])])
            };
            let rows = vec![
                field_row("Conversation ID", "018f47ac-7e9c-78f4-b816-44e68487ba15"),
                field_row("Email", "moira@example.com"),
                TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([
                    ("Ctrl-P ".to_owned(), builder.link_text_style()),
                    (
                        "Select previous block".to_owned(),
                        builder.primary_text_style(),
                    ),
                ])]),
            ];
            TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new("Status", rows)]).render(
                TuiSelectionHandle::default(),
                &builder,
                |_, _| {},
                move |text, _, _| copies_for_callback.borrow_mut().push(text),
            )
        });
        let size = TuiSize::new(80, 4);
        let selections = [
            (25, 1, "018f47ac-7e9c-78f4-b816-44e68487ba15"),
            (26, 2, "moira@example.com"),
            (16, 3, "Select previous block"),
        ];

        let _ = render(&app, element.as_mut(), size);
        for (x, y, _) in selections {
            assert!(dispatch_mouse(
                &app,
                element.as_mut(),
                size,
                left_down_with_click_count(x, y, 2)
            ));
            assert!(dispatch_mouse(&app, element.as_mut(), size, left_up(x, y)));
        }

        assert_eq!(
            copies.borrow().as_slice(),
            selections.map(|(_, _, expected)| expected)
        );
    });
}
