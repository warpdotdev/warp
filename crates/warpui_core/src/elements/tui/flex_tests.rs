use std::cell::Cell;
use std::rc::Rc;

use ratatui::style::{Color, Modifier, Style};

use super::TuiFlex;
use crate::elements::CrossAxisAlignment;
use crate::elements::tui::test_support::{render_to_lines, with_event_context, with_paint_surface};
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiChildView, TuiConstraint, TuiElement, TuiEvent, TuiEventHandler,
    TuiLayoutContext, TuiParentElement, TuiPresentationContext, TuiRect, TuiScreenPosition,
    TuiSize, TuiText,
};
use crate::event::KeyEventDetails;
use crate::keymap::Keystroke;
use crate::{App, EntityId, EntityIdMap};

/// Lays `element` out at a loose `size` constraint, returning the size it
/// claimed. Layout must run before render so `child_sizes` is populated.
fn layout_at(element: &mut dyn TuiElement, size: TuiSize, app_ctx: &crate::AppContext) -> TuiSize {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.layout(TuiConstraint::loose(size), &mut ctx, app_ctx)
}

#[test]
fn row_inserts_configured_spacing_between_children() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut row = TuiFlex::row()
                .with_spacing(2)
                .child(TuiText::new("AA").truncate().finish())
                .child(TuiText::new("BB").truncate().finish());

            let size = layout_at(&mut row, TuiSize::new(10, 1), app_ctx);
            assert_eq!(size, TuiSize::new(6, 1));
            assert_eq!(render_to_lines(row, TuiSize::new(6, 1)), vec!["AA  BB"]);
        });
    });
}

// -- column-axis tests --

#[test]
fn column_stacks_two_children_top_to_bottom() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut column = TuiFlex::column()
                .with_child(Box::new(TuiText::new("AA")))
                .with_child(Box::new(TuiText::new("BB")));

            let size = layout_at(&mut column, TuiSize::new(2, 10), app_ctx);
            assert_eq!(size, TuiSize::new(2, 2));
            assert_eq!(
                render_to_lines(column, TuiSize::new(2, 2)),
                vec!["AA", "BB"]
            );
        });
    });
}

#[test]
fn column_sums_multi_row_children_at_the_correct_offsets() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // The middle child spans two rows, so the trailing child must land on row 3.
            let mut column = TuiFlex::column()
                .with_child(Box::new(TuiText::new("A")))
                .with_child(Box::new(TuiText::new("BB\nCC").truncate()))
                .with_child(Box::new(TuiText::new("D")));

            let size = layout_at(&mut column, TuiSize::new(2, 4), app_ctx);
            assert_eq!(size, TuiSize::new(2, 4));
            assert_eq!(
                render_to_lines(column, TuiSize::new(2, 4)),
                vec!["A ", "BB", "CC", "D "],
            );
        });
    });
}

#[test]
fn column_clamps_total_height_to_the_constraint_and_clips_overflow() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut column = TuiFlex::column()
                .with_child(Box::new(TuiText::new("A")))
                .with_child(Box::new(TuiText::new("BB\nCC").truncate()))
                .with_child(Box::new(TuiText::new("D")));

            let size = layout_at(&mut column, TuiSize::new(2, 3), app_ctx);
            assert_eq!(size, TuiSize::new(2, 3));

            // Only the first three rows fit; the final child is clipped away.
            assert_eq!(
                render_to_lines(column, TuiSize::new(2, 3)),
                vec!["A ", "BB", "CC"],
            );
        });
    });
}

#[test]
fn column_flex_child_fills_leftover_and_docks_fixed_child_at_bottom() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // A flex spacer on top fills the leftover height, pushing the fixed
            // single-row child to the bottom of the 4-row area.
            let mut column = TuiFlex::column()
                .flex_child(TuiFlex::column().finish())
                .child(TuiText::new("IN").finish());

            let size = layout_at(&mut column, TuiSize::new(2, 4), app_ctx);
            // With a flex child present, the column fills the offered height.
            assert_eq!(size, TuiSize::new(2, 4));

            // The flex spacer occupies the top three rows; the fixed input row
            // lands on the last row.
            assert_eq!(
                render_to_lines(column, TuiSize::new(2, 4)),
                vec!["  ", "  ", "  ", "IN"],
            );
        });
    });
}

// -- row-axis tests --

#[test]
fn row_packs_two_children_left_to_right() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut row = TuiFlex::row()
                .with_child(Box::new(TuiText::new("AA").truncate()))
                .with_child(Box::new(TuiText::new("BB").truncate()));

            let size = layout_at(&mut row, TuiSize::new(10, 1), app_ctx);
            // Without flex children the row hugs its content horizontally.
            assert_eq!(size, TuiSize::new(4, 1));
            assert_eq!(render_to_lines(row, TuiSize::new(4, 1)), vec!["AABB"]);
        });
    });
}

#[test]
fn row_flex_spacer_pushes_trailing_children_to_the_right_edge() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut row = TuiFlex::row()
                .child(TuiText::new("L").truncate().finish())
                .flex_child(TuiFlex::row().finish())
                .child(TuiText::new("RR").truncate().finish());

            let size = layout_at(&mut row, TuiSize::new(8, 1), app_ctx);
            // With a flex child present, the row fills the offered width.
            assert_eq!(size, TuiSize::new(8, 1));
            assert_eq!(render_to_lines(row, TuiSize::new(8, 1)), vec!["L     RR"]);
        });
    });
}

#[test]
fn row_splits_leftover_evenly_across_flex_children() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // 7 leftover columns across two spacers: 4 to the first (remainder),
            // 3 to the second, centering the middle child off by the remainder.
            let mut row = TuiFlex::row()
                .flex_child(TuiFlex::row().finish())
                .child(TuiText::new("MID").truncate().finish())
                .flex_child(TuiFlex::row().finish());

            let size = layout_at(&mut row, TuiSize::new(10, 1), app_ctx);
            assert_eq!(size, TuiSize::new(10, 1));
            assert_eq!(
                render_to_lines(row, TuiSize::new(10, 1)),
                vec!["    MID   "]
            );
        });
    });
}

#[test]
fn row_clips_children_past_the_available_width() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut row = TuiFlex::row()
                .child(TuiText::new("AAAA").truncate().finish())
                .child(TuiText::new("BBBB").truncate().finish());

            let size = layout_at(&mut row, TuiSize::new(6, 1), app_ctx);
            assert_eq!(size, TuiSize::new(6, 1));
            // The second child only has two columns left and is clipped.
            assert_eq!(render_to_lines(row, TuiSize::new(6, 1)), vec!["AAAABB"]);
        });
    });
}

#[test]
fn row_sizes_cross_axis_to_its_tallest_child() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // The cross axis is content-sized (like the GUI Flex): a row of
            // one-row children is one row tall, not the offered three rows.
            let mut row = TuiFlex::row().child(TuiText::new("A").truncate().finish());
            let size = layout_at(&mut row, TuiSize::new(4, 3), app_ctx);
            assert_eq!(size, TuiSize::new(1, 1));
        });
    });
}

#[test]
fn tight_cross_axis_constraint_forces_fill() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // A tight constraint's min clamps the content-sized cross axis
            // (and main axis) up to the offered extent.
            let mut row = TuiFlex::row().child(TuiText::new("A").truncate().finish());
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = row.layout(TuiConstraint::tight(TuiSize::new(4, 3)), &mut ctx, app_ctx);
            assert_eq!(size, TuiSize::new(4, 3));
        });
    });
}

// -- cross-axis alignment --

#[test]
fn stretch_fills_offered_cross_extent_and_tightens_children() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // A stretched column fills the offered width, and its children get
            // a tight cross constraint: the nested (unstretched) column is
            // forced to width 4 even though its content is one column wide.
            let mut column = TuiFlex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    TuiFlex::column()
                        .child(TuiText::new("A").truncate().finish())
                        .finish(),
                );
            let size = layout_at(&mut column, TuiSize::new(4, 3), app_ctx);
            assert_eq!(size, TuiSize::new(4, 1));
        });
    });
}

#[test]
fn center_positions_child_along_cross_axis() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // A one-row child centered in a three-row row lands on the middle
            // row.
            let mut row = TuiFlex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .child(TuiText::new("A").truncate().finish());
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = row.layout(TuiConstraint::tight(TuiSize::new(5, 3)), &mut ctx, app_ctx);
            assert_eq!(size, TuiSize::new(5, 3));
            let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 5, 3));
            with_paint_surface(&mut buffer, |surface, ctx| {
                row.render(TuiScreenPosition::new(0, 0), surface, ctx)
            });
            assert_eq!(buffer.to_lines(), vec!["     ", "A    ", "     "],);
        });
    });
}

#[test]
fn end_positions_child_along_cross_axis() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // End-aligned column: the one-column child lands at the right edge
            // of the five-column cross extent.
            let mut column = TuiFlex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::End)
                .child(TuiText::new("A").truncate().finish());
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = column.layout(TuiConstraint::tight(TuiSize::new(5, 1)), &mut ctx, app_ctx);
            assert_eq!(size, TuiSize::new(5, 1));
            let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 5, 1));
            with_paint_surface(&mut buffer, |surface, ctx| {
                column.render(TuiScreenPosition::new(0, 0), surface, ctx)
            });
            assert_eq!(buffer.to_lines(), vec!["    A"]);
        });
    });
}

#[test]
fn row_children_keep_their_own_styles() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let dim = Style::default().add_modifier(Modifier::DIM);
            let cyan = Style::default().fg(Color::Cyan);
            let mut row = TuiFlex::row()
                .child(TuiText::new("a").with_style(dim).truncate().finish())
                .flex_child(TuiFlex::row().finish())
                .child(TuiText::new("b").with_style(cyan).truncate().finish());

            layout_at(&mut row, TuiSize::new(4, 1), app_ctx);

            let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 4, 1));
            with_paint_surface(&mut buffer, |surface, ctx| {
                row.render(TuiScreenPosition::new(0, 0), surface, ctx)
            });

            let left_cell = &buffer[(0, 0)];
            assert_eq!(left_cell.symbol(), "a");
            assert!(left_cell.modifier.contains(Modifier::DIM));

            let right_cell = &buffer[(3, 0)];
            assert_eq!(right_cell.symbol(), "b");
            assert_eq!(right_cell.fg, Color::Cyan);
        });
    });
}

// -- axis-independent behavior --

#[test]
fn present_recurses_into_children() {
    let root = EntityId::from_usize(1);
    let embedded = EntityId::from_usize(2);
    let mut parent_by_child = EntityIdMap::default();

    {
        let mut rendered_views_for_child = EntityIdMap::default();
        let mut ctx =
            TuiPresentationContext::new(root, &mut rendered_views_for_child, &mut parent_by_child);
        let child_node = TuiChildView::from_rendered(
            embedded,
            Box::new(TuiText::new("body")),
            ctx.rendered_views,
        );
        let mut column = TuiFlex::column()
            .with_child(Box::new(TuiText::new("header")))
            .with_child(Box::new(child_node));
        column.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&embedded), Some(&root));
}

fn key_event(key: &str) -> TuiEvent {
    TuiEvent::KeyDown {
        keystroke: Keystroke {
            key: key.to_owned(),
            ..Default::default()
        },
        chars: key.to_owned(),
        details: KeyEventDetails::default(),
        is_composing: false,
    }
}

#[test]
fn dispatch_event_broadcasts_to_all_children() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let first_hits = Rc::new(Cell::new(0u32));
            let second_hits = Rc::new(Cell::new(0u32));
            let first_counter = first_hits.clone();
            let second_counter = second_hits.clone();

            let mut column = TuiFlex::column()
                .with_child(Box::new(TuiText::new("header")))
                .with_child(Box::new(
                    TuiEventHandler::new(TuiText::new("first").finish())
                        .on_key("x", move |_, _, _| {
                            first_counter.set(first_counter.get() + 1)
                        }),
                ))
                .with_child(Box::new(
                    TuiEventHandler::new(TuiText::new("second").finish())
                        .on_key("x", move |_, _, _| {
                            second_counter.set(second_counter.get() + 1)
                        }),
                ));

            let handled = with_event_context(|event_ctx| {
                column.dispatch_event(&key_event("x"), event_ctx, app_ctx)
            });

            assert!(handled);
            assert_eq!(first_hits.get(), 1);
            assert_eq!(second_hits.get(), 1);
        });
    });
}
