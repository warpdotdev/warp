//! Headless tests for [`TuiPresenter`]. They drive layout + paint against local
//! [`TuiElement`] test-doubles and assert the composited [`TuiBuffer`] via
//! `to_lines` plus the surfaced cursor. The child-view tests register real
//! [`TuiView`]s in a test [`App`] and resolve them through the app, exactly as
//! the live elements do.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use instant::Instant;

use super::TuiPresenter;
use crate::elements::tui::{
    TuiAnimated, TuiBuffer, TuiBufferExt, TuiChildView, TuiConstraint, TuiElement,
    TuiLayoutContext, TuiPaintContext, TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
    TuiStyle,
};
use crate::platform::WindowStyle;
use crate::{
    AddWindowOptions, App, AppContext, Entity, FocusContext, TuiView, TypedActionView, ViewContext,
    ViewHandle,
};

// --- Test-double elements -------------------------------------------------

/// A single line of text: as wide as its content, one row tall.
struct TextDouble {
    text: String,
}

impl TextDouble {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
        }
    }

    fn width(&self) -> u16 {
        self.text.chars().count() as u16
    }
}

impl TuiElement for TextDouble {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(self.width(), 1))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {
        buffer.set_stringn(
            area.x,
            area.y,
            &self.text,
            usize::from(area.width),
            TuiStyle::default(),
        );
    }
}

/// A vertical stack: each child is laid out at the column's width and stacked
/// top-to-bottom. Records per-child sizes at layout time so `render`/
/// `cursor_position` (which take `&self`) can place children consistently.
struct ColumnDouble {
    children: Vec<Box<dyn TuiElement>>,
    child_sizes: Vec<TuiSize>,
}

impl ColumnDouble {
    fn new(children: Vec<Box<dyn TuiElement>>) -> Self {
        Self {
            children,
            child_sizes: Vec::new(),
        }
    }
}

impl TuiElement for ColumnDouble {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child_sizes.clear();
        let mut total_height = 0u16;
        let mut max_width = 0u16;
        for child in &mut self.children {
            let available = TuiSize::new(
                constraint.max.width,
                constraint.max.height.saturating_sub(total_height),
            );
            let size = child.layout(TuiConstraint::loose(available), ctx, app);
            total_height = total_height.saturating_add(size.height);
            max_width = max_width.max(size.width);
            self.child_sizes.push(size);
        }
        constraint.clamp(TuiSize::new(max_width, total_height))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        let mut remaining = area;
        for (child, size) in self.children.iter().zip(&self.child_sizes) {
            let (row, rest) = remaining.split_top(size.height);
            let child_area = TuiRect::new(row.x, row.y, size.width.min(row.width), row.height);
            child.render(child_area, buffer, ctx);
            remaining = rest;
        }
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        for child in &mut self.children {
            child.present(ctx);
        }
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        let mut remaining = area;
        for (child, size) in self.children.iter().zip(&self.child_sizes) {
            let (row, rest) = remaining.split_top(size.height);
            let child_area = TuiRect::new(row.x, row.y, size.width.min(row.width), row.height);
            if let Some((cx, cy)) = child.cursor_position(child_area, ctx) {
                return Some((child_area.x - area.x + cx, child_area.y - area.y + cy));
            }
            remaining = rest;
        }
        None
    }
}

/// A styled container that fills its area with `fill` then paints its child
/// inset by `padding` on every side.
struct ContainerDouble {
    child: Box<dyn TuiElement>,
    padding: u16,
    fill: char,
    child_size: TuiSize,
}

impl ContainerDouble {
    fn new(child: Box<dyn TuiElement>, padding: u16, fill: char) -> Self {
        Self {
            child,
            padding,
            fill,
            child_size: TuiSize::ZERO,
        }
    }
}

impl TuiElement for ContainerDouble {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let inset = self.padding.saturating_mul(2);
        let inner_max = TuiSize::new(
            constraint.max.width.saturating_sub(inset),
            constraint.max.height.saturating_sub(inset),
        );
        let size = self.child.layout(TuiConstraint::loose(inner_max), ctx, app);
        self.child_size = size;
        constraint.clamp(TuiSize::new(
            size.width.saturating_add(inset),
            size.height.saturating_add(inset),
        ))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        let fill = self.fill.to_string();
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if let Some(cell) = buffer.cell_mut((x, y)) {
                    cell.set_symbol(&fill);
                }
            }
        }
        let inner = area.inset(self.padding);
        let child_area = TuiRect::new(
            inner.x,
            inner.y,
            self.child_size.width.min(inner.width),
            self.child_size.height.min(inner.height),
        );
        self.child.render(child_area, buffer, ctx);
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }
}

/// A leaf that owns the cursor, reporting it at a fixed offset within its area.
struct CursorDouble {
    offset: (u16, u16),
}

impl CursorDouble {
    fn new(offset: (u16, u16)) -> Self {
        Self { offset }
    }
}

impl TuiElement for CursorDouble {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(5, 1))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {
        buffer.set_stringn(
            area.x,
            area.y,
            "INPUT",
            usize::from(area.width),
            TuiStyle::default(),
        );
    }

    fn cursor_position(&self, _area: TuiRect, _ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        Some(self.offset)
    }
}

/// A leaf that requests a repaint `delay` after every paint.
struct RepaintDouble {
    delay: Duration,
}

impl TuiElement for RepaintDouble {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(1, 1))
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        ctx.repaint_after(self.delay);
    }
}

// --- Real views for the child-view recursion tests -------------------------

fn window_options() -> AddWindowOptions {
    AddWindowOptions {
        window_style: WindowStyle::NotStealFocus,
        ..Default::default()
    }
}

/// Minimal window root; never presented directly.
#[derive(Default)]
struct RootStub;

impl Entity for RootStub {
    type Event = ();
}

impl TuiView for RootStub {
    fn ui_name() -> &'static str {
        "RootStub"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(())
    }
}

impl TypedActionView for RootStub {
    type Action = ();
}

/// A registered child view whose output is a single line of text.
struct LeafView;

impl Entity for LeafView {
    type Event = ();
}

impl TuiView for LeafView {
    fn ui_name() -> &'static str {
        "LeafView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TextDouble::new("CHILD"))
    }
}

/// A childless root view that counts its renders, to pin the paint-only
/// repaint contract: repaints must reuse the cached element tree instead of
/// re-rendering the view.
struct CountingLeafView {
    renders: Rc<Cell<usize>>,
}

impl Entity for CountingLeafView {
    type Event = ();
}

impl TuiView for CountingLeafView {
    fn ui_name() -> &'static str {
        "CountingLeafView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        self.renders.set(self.renders.get() + 1);
        Box::new(TextDouble::new("LEAF"))
    }
}

/// A parent view: a header above an embedded child view, which it resolves
/// through the app at render time. Records `DescendentFocused` hook firings.
struct ParentView {
    child: ViewHandle<LeafView>,
    descendent_focus_events: usize,
}

impl Entity for ParentView {
    type Event = ();
}

impl TuiView for ParentView {
    fn ui_name() -> &'static str {
        "ParentView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        let child = TuiChildView::new(&self.child);
        Box::new(ColumnDouble::new(vec![
            Box::new(TextDouble::new("HEADER")),
            Box::new(child),
        ]))
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {
        if matches!(focus_ctx, FocusContext::DescendentFocused(_)) {
            self.descendent_focus_events += 1;
        }
    }
}

// --- Tests ----------------------------------------------------------------

#[test]
fn paints_single_root_element_into_area() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                Box::new(TextDouble::new("HELLO")),
                TuiRect::new(0, 0, 10, 1),
                app_ctx,
            );
            assert_eq!(frame.buffer.to_lines(), vec!["HELLO     "]);
            assert_eq!(frame.cursor, None);
        });
    });
}

#[test]
fn composites_nested_container_column_text_with_offsets() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let column = ColumnDouble::new(vec![
                Box::new(TextDouble::new("AB")),
                Box::new(TextDouble::new("CDE")),
            ]);
            let container = ContainerDouble::new(Box::new(column), 1, '.');

            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(Box::new(container), TuiRect::new(0, 0, 5, 4), app_ctx);

            assert_eq!(
                frame.buffer.to_lines(),
                vec![".....", ".AB..", ".CDE.", "....."],
            );
        });
    });
}

#[test]
fn surfaces_cursor_at_absolute_coordinates() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let column = ColumnDouble::new(vec![
                Box::new(TextDouble::new("HEADER")),
                Box::new(CursorDouble::new((2, 0))),
            ]);

            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(Box::new(column), TuiRect::new(0, 0, 8, 2), app_ctx);

            // The cursor element sits on row 1 (below the header) at column 2.
            assert_eq!(frame.cursor, Some((2, 1)));
        });
    });
}

#[test]
fn frames_without_animated_elements_request_no_repaint() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                Box::new(TextDouble::new("STATIC")),
                TuiRect::new(0, 0, 10, 1),
                app_ctx,
            );
            assert_eq!(frame.repaint_at, None);
        });
    });
}

#[test]
fn frame_surfaces_the_earliest_requested_repaint_deadline() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let before = Instant::now();
            let column = ColumnDouble::new(vec![
                Box::new(RepaintDouble {
                    delay: Duration::from_secs(60),
                }),
                Box::new(RepaintDouble {
                    delay: Duration::from_millis(10),
                }),
            ]);

            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(Box::new(column), TuiRect::new(0, 0, 4, 2), app_ctx);

            let repaint_at = frame.repaint_at.expect("a repaint should be requested");
            // Earliest-deadline-wins: the 10ms request beats the 60s one.
            assert!(repaint_at <= before + Duration::from_secs(1));
        });
    });
}

#[test]
fn animated_element_requests_a_repaint_every_paint() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let animated = TuiAnimated::new(Duration::from_millis(50), || {
                TextDouble::new("LIVE").finish()
            });

            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(Box::new(animated), TuiRect::new(0, 0, 10, 1), app_ctx);

            assert!(frame.repaint_at.is_some());
            assert_eq!(frame.buffer.to_lines(), vec!["LIVE      "]);
        });
    });
}

#[test]
fn paint_only_repaint_reuses_the_cached_leaf_root_without_re_rendering() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| RootStub));
        let renders = Rc::new(Cell::new(0));
        let renders_in_view = renders.clone();
        let view = app.update(|ctx| {
            ctx.add_tui_view(window_id, move |_| CountingLeafView {
                renders: renders_in_view,
            })
        });

        let mut presenter = TuiPresenter::new();
        // The first draw renders the view once (via its initial invalidation).
        let frame = app.update(|ctx| {
            let invalidation = ctx.take_all_invalidations_for_window(window_id);
            presenter.invalidate(&invalidation, ctx, window_id);
            presenter.present(ctx, &view, TuiRect::new(0, 0, 8, 1))
        });
        assert_eq!(frame.buffer.to_lines(), vec!["LEAF    "]);
        assert_eq!(renders.get(), 1);

        // A paint-only repaint (no invalidations — e.g. an animation frame)
        // must reuse the cached element tree without re-rendering the view,
        // even though this root has no child views in `rendered_views`.
        let frame = app.update(|ctx| {
            let invalidation = ctx.take_all_invalidations_for_window(window_id);
            presenter.invalidate(&invalidation, ctx, window_id);
            presenter.present(ctx, &view, TuiRect::new(0, 0, 8, 1))
        });
        assert_eq!(frame.buffer.to_lines(), vec!["LEAF    "]);
        assert_eq!(renders.get(), 1);
    });
}

#[test]
fn recurses_into_registered_child_view_and_reports_embeddings() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| RootStub));

        let child = app.update(|ctx| ctx.add_tui_view(window_id, |_| LeafView));
        let child_view_id = child.id();
        let parent = app.update(|ctx| {
            ctx.add_tui_view(window_id, move |_| ParentView {
                child,
                descendent_focus_events: 0,
            })
        });

        let mut presenter = TuiPresenter::new();
        let frame = app.update(|ctx| {
            let invalidation = ctx.take_all_invalidations_for_window(window_id);
            presenter.invalidate(&invalidation, ctx, window_id);
            presenter.present(ctx, &parent, TuiRect::new(0, 0, 8, 3))
        });

        // The child view's output is painted directly below the header, at the
        // area the layout allocated to the embedded child-view element.
        assert_eq!(
            frame.buffer.to_lines(),
            vec!["HEADER  ", "CHILD   ", "        "],
        );

        // The presentation pass reported the embedded view into the core's
        // neutral view hierarchy: the child's ancestor chain now runs through
        // the parent.
        assert_eq!(
            app.read(|ctx| ctx.view_ancestors(window_id, child_view_id)),
            vec![parent.id(), child_view_id],
        );
    });
}

#[test]
fn focusing_embedded_child_fires_descendent_focus_on_parent() {
    App::test((), |mut app| async move {
        let (window_id, _root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| RootStub));

        let child = app.update(|ctx| ctx.add_tui_view(window_id, |_| LeafView));
        let child_for_view = child.clone();
        let parent = app.update(|ctx| {
            ctx.add_tui_view(window_id, move |_| ParentView {
                child: child_for_view,
                descendent_focus_events: 0,
            })
        });

        // Before any present pass the core knows nothing about the embedding.
        assert_eq!(parent.read(&app, |view, _| view.descendent_focus_events), 0);

        // A present pass reports the embedding into `view_parents`...
        let mut presenter = TuiPresenter::new();
        app.update(|ctx| {
            let invalidation = ctx.take_all_invalidations_for_window(window_id);
            presenter.invalidate(&invalidation, ctx, window_id);
            presenter.present(ctx, &parent, TuiRect::new(0, 0, 8, 3))
        });

        // ...so focusing the embedded child walks the ancestor chain and fires
        // the parent's `on_focus` hook with `DescendentFocused`.
        child.update(&mut app, |_, ctx| ctx.focus_self());
        assert_eq!(app.focused_view_id(window_id), Some(child.id()));
        assert_eq!(parent.read(&app, |view, _| view.descendent_focus_events), 1);
    });
}
