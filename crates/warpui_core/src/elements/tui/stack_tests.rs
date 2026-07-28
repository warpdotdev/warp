use std::cell::Cell as StdCell;
use std::rc::Rc;
use std::time::Duration;

use instant::Instant;
use ratatui::style::{Color, Modifier, Style};

use super::TuiStack;
use crate::elements::tui::test_support::{
    render_to_frame, render_to_lines, with_event_context, with_paint_surface,
};
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstrainedBox, TuiConstraint, TuiElement, TuiEvent,
    TuiEventHandler, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPoint,
    TuiScreenPosition, TuiSize, TuiText,
};
use crate::event::KeyEventDetails;
use crate::keymap::Keystroke;
use crate::{App, EntityIdMap};

fn layout_at(element: &mut dyn TuiElement, size: TuiSize, app: &crate::AppContext) -> TuiSize {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.layout(TuiConstraint::loose(size), &mut ctx, app)
}

#[test]
fn layout_uses_the_largest_child_on_each_axis() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut stack = TuiStack::new()
                .child(TuiText::new("wide").finish())
                .child(TuiText::new("x\ny").truncate().finish());

            assert_eq!(
                layout_at(&mut stack, TuiSize::new(10, 10), app),
                TuiSize::new(4, 2),
            );
        });
    });
}

#[test]
fn later_glyphs_win_while_blank_padding_is_transparent() {
    let background = TuiText::new("stars")
        .with_style(Style::default().fg(Color::Blue))
        .finish();
    let foreground = TuiConstrainedBox::new(
        TuiText::new("A")
            .with_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            .finish(),
    )
    .with_min_cols(5)
    .with_max_cols(5)
    .finish();
    let stack = TuiStack::new().child(background).child(foreground);

    let frame = render_to_frame(stack, TuiSize::new(5, 1));

    assert_eq!(frame.buffer.to_lines(), vec!["Atars"]);
    assert_eq!(frame.buffer[(0, 0)].fg, Color::Red);
    assert_eq!(frame.buffer[(1, 0)].fg, Color::Blue);
}

#[test]
fn blank_cells_with_a_background_are_opaque() {
    let background = TuiText::new("stars").finish();
    let foreground = TuiConstrainedBox::new(
        TuiText::new("A")
            .with_style(Style::default().bg(Color::Red))
            .finish(),
    )
    .with_min_cols(5)
    .with_max_cols(5)
    .finish();
    let stack = TuiStack::new().child(background).child(foreground);

    let frame = render_to_frame(stack, TuiSize::new(5, 1));

    assert_eq!(frame.buffer.to_lines(), vec!["A    "]);
    assert_eq!(frame.buffer[(4, 0)].bg, Color::Red);
}

#[test]
fn a_narrow_foreground_glyph_clears_a_lower_wide_grapheme() {
    let stack = TuiStack::new()
        .child(TuiText::new("界z").finish())
        .child(TuiText::new(" x").finish());

    assert_eq!(render_to_lines(stack, TuiSize::new(3, 1)), vec![" xz"],);
}

#[test]
fn a_wide_foreground_glyph_clears_every_lower_cell_it_covers() {
    let stack = TuiStack::new()
        .child(TuiText::new("abz").finish())
        .child(TuiText::new("界").finish());

    assert_eq!(render_to_lines(stack, TuiSize::new(3, 1)), vec!["界z"],);
}
#[test]
fn zero_sized_front_child_leaves_the_full_sized_back_child_visible() {
    let stack = TuiStack::new()
        .child(TuiText::new("back").finish())
        .child(().finish());

    assert_eq!(render_to_lines(stack, TuiSize::new(4, 1)), vec!["back"],);
}
struct OverpaintingElement {
    size: Option<TuiSize>,
}

impl TuiElement for OverpaintingElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &crate::AppContext,
    ) -> TuiSize {
        let size = constraint.clamp(TuiSize::new(1, 1));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
        surface
            .cell_mut(origin)
            .expect("the retained child bounds contain its first cell")
            .set_symbol("X");
        if let Some(cell) = surface.cell_mut(origin.offset(1, 0)) {
            cell.set_symbol("Y");
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }
}

#[test]
fn child_paint_is_clipped_to_its_retained_size() {
    let stack = TuiStack::new()
        .child(TuiText::new("abc").finish())
        .child(OverpaintingElement { size: None }.finish());

    assert_eq!(render_to_lines(stack, TuiSize::new(3, 1)), vec!["Xbc"],);
}

#[test]
fn smaller_front_child_clears_a_wide_grapheme_across_its_right_edge() {
    let stack = TuiStack::new()
        .child(TuiText::new("ab界z").finish())
        .child(TuiText::new("  X").finish());

    assert_eq!(render_to_lines(stack, TuiSize::new(5, 1)), vec!["abX z"],);
}

struct RepaintElement {
    delay: Duration,
    size: Option<TuiSize>,
}

impl TuiElement for RepaintElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &crate::AppContext,
    ) -> TuiSize {
        let size = constraint.clamp(TuiSize::new(1, 1));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        _origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        ctx.repaint_after(self.delay);
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }
}

#[test]
fn animated_child_repaint_requests_propagate_and_keep_the_earliest_deadline() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut stack = TuiStack::new()
                .child(
                    RepaintElement {
                        delay: Duration::from_millis(250),
                        size: None,
                    }
                    .finish(),
                )
                .child(
                    RepaintElement {
                        delay: Duration::from_millis(20),
                        size: None,
                    }
                    .finish(),
                );
            layout_at(&mut stack, TuiSize::new(1, 1), app);

            let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 1, 1));
            let started_at = Instant::now();
            with_paint_surface(&mut buffer, |surface, ctx| {
                stack.render(TuiScreenPosition::new(0, 0), surface, ctx);
                let repaint_at = ctx
                    .requested_repaint_at()
                    .expect("animated stack child requested a repaint");
                assert!(
                    repaint_at < started_at + Duration::from_millis(100),
                    "the shorter child repaint delay should win"
                );
            });
        });
    });
}

struct CursorElement {
    column: i32,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiElement for CursorElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &crate::AppContext,
    ) -> TuiSize {
        let size = constraint.clamp(TuiSize::new(2, 1));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        let origin = ctx.scene_point(origin);
        self.origin = Some(origin);
        ctx.set_terminal_cursor(TuiScreenPoint::new(
            origin.x.saturating_add(self.column),
            origin.y,
            origin.z_index,
        ));
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

#[test]
fn later_children_have_cursor_priority() {
    let stack = TuiStack::new()
        .child(
            CursorElement {
                column: 0,
                size: None,
                origin: None,
            }
            .finish(),
        )
        .child(
            CursorElement {
                column: 1,
                size: None,
                origin: None,
            }
            .finish(),
        );

    assert_eq!(
        render_to_frame(stack, TuiSize::new(2, 1)).cursor,
        Some((1, 0)),
    );
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
fn dispatch_stops_at_the_first_frontmost_handler() {
    App::test((), |app| async move {
        app.read(|app| {
            let back_hits = Rc::new(StdCell::new(0u32));
            let front_hits = Rc::new(StdCell::new(0u32));
            let back_counter = Rc::clone(&back_hits);
            let front_counter = Rc::clone(&front_hits);
            let mut stack = TuiStack::new()
                .child(
                    TuiEventHandler::new(TuiText::new("back").finish())
                        .on_key("x", move |_, _, _| back_counter.set(back_counter.get() + 1))
                        .finish(),
                )
                .child(
                    TuiEventHandler::new(TuiText::new("front").finish())
                        .on_key("x", move |_, _, _| {
                            front_counter.set(front_counter.get() + 1)
                        })
                        .finish(),
                );

            let handled = with_event_context(|ctx| stack.dispatch_event(&key_event("x"), ctx, app));

            assert!(handled);
            assert_eq!(back_hits.get(), 0);
            assert_eq!(front_hits.get(), 1);
        });
    });
}

#[test]
fn composition_respects_a_mapped_negative_origin() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut stack = TuiStack::new()
                .child(TuiText::new("abc").finish())
                .child(TuiText::new("X").finish());
            layout_at(&mut stack, TuiSize::new(3, 1), app);

            let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 3, 1));
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface =
                    TuiPaintSurface::mapped(&mut buffer, TuiScreenPosition::new(-3, -2));
                stack.render(TuiScreenPosition::new(-3, -2), &mut surface, &mut ctx);
            }

            assert_eq!(buffer.to_lines(), vec!["Xbc"]);
        });
    });
}
