//! Interactive manual harness for the generalized TUI viewport stack.
//!
//! Run it from a real terminal:
//!
//! ```sh
//! cargo run -p warpui_core --example tui_viewport_demo --features tui
//! ```
//!
//! It drives the real [`TuiRuntime`] against your terminal over a trivial
//! in-memory index of text blocks, exercising every piece the viewport ships so
//! all of it can be eyeballed in isolation (no `TerminalModel`/AI dependency):
//! - **`TuiViewportIndex` / `TuiViewportCursor`** — the in-memory index seeks
//!   (`Start`/`End`/`Item`) and walks forward/backward to collect visible items.
//! - **`TuiViewportedList` + caller-owned `TuiViewportHandle`** — starts at the
//!   index end; wheel-scrolling up anchors it; scrolling back to the end
//!   restores the end position. Removing the anchored item snaps back to end.
//! - **`TuiScrollable` + wheel conversion + mouse capture** — the mouse wheel
//!   over the list scrolls it (the only path that exercises wheel→`ScrollWheel`
//!   conversion and the alternate-screen mouse capture end to end).
//! - **`TuiClipped`** — a top-clipped block is rendered by offsetting the
//!   wrapped item element to its first visible logical row.
//! - **height reconciliation** — view-measured blocks report a width-dependent
//!   full height; resizing the terminal re-measures and writes the new height
//!   back into the index.
//!
//! Keys: mouse wheel scroll · `x` remove the front block · `q` / `Esc` quit.

use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;

use warpui_core::elements::tui::{
    Modifier, RenderedViewportItem, TuiClipped, TuiColumn, TuiElement, TuiEventHandler,
    TuiParentElement, TuiScrollable, TuiStyle, TuiText, TuiViewportCursor, TuiViewportHandle,
    TuiViewportIndex, TuiViewportIndexItem, TuiViewportIndexPosition, TuiViewportedList,
    ViewportRenderRequest,
};
use warpui_core::platform::WindowStyle;
use warpui_core::runtime::TuiRuntime;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView, ViewContext,
};

// ─────────────────────────────────────────────────────────────────────────────
// Backing index: a trivial in-memory list of text blocks with interior
// mutability, mirroring the unit-test `FakeIndex` shape so the demo exercises
// the same traits the real terminal-history index implements.
// ─────────────────────────────────────────────────────────────────────────────

/// One block in the demo transcript. Fixed blocks own their rows; view-measured
/// blocks own a wrapping paragraph whose height depends on the render width.
#[derive(Clone)]
struct DemoItem {
    id: usize,
    lines: Vec<String>,
    measured_text: Option<String>,
    height: usize,
    needs_measurement: bool,
}

/// The starting transcript: a mix of fixed-height blocks (varying heights) and
/// periodic view-measured wrapping blocks, sized to overflow a normal terminal.
fn initial_items() -> Vec<DemoItem> {
    (0..14)
        .map(|id| {
            if id % 4 == 3 {
                DemoItem {
                    id,
                    lines: Vec::new(),
                    measured_text: Some(format!(
                        "block {id} (view-measured): a long wrapping paragraph that reflows to \
                         the available width, so resizing the terminal re-measures its height and \
                         writes the new height back into the index."
                    )),
                    height: 1,
                    needs_measurement: true,
                }
            } else {
                let rows = 3 + (id % 3);
                DemoItem {
                    id,
                    lines: (0..rows)
                        .map(|row| format!("block {id} · fixed row {row}"))
                        .collect(),
                    measured_text: None,
                    height: rows,
                    needs_measurement: false,
                }
            }
        })
        .collect()
}

/// An ordered-height index over a shared `Vec<DemoItem>`.
struct DemoIndex {
    items: Rc<RefCell<Vec<DemoItem>>>,
}

/// A scoped cursor borrowing the shared items for one collection pass.
struct DemoCursor<'a> {
    items: Ref<'a, Vec<DemoItem>>,
    position: Option<usize>,
}

impl TuiViewportCursor for DemoCursor<'_> {
    type ItemId = usize;
    type Item = DemoItem;

    fn item(&self) -> Option<TuiViewportIndexItem<Self::ItemId, Self::Item>> {
        let item = self.items.get(self.position?)?.clone();
        Some(TuiViewportIndexItem {
            id: item.id,
            height: item.height,
            needs_measurement: item.needs_measurement,
            item,
        })
    }

    fn next(&mut self) {
        self.position = self
            .position
            .and_then(|position| (position + 1 < self.items.len()).then_some(position + 1));
    }

    fn prev(&mut self) {
        self.position = match self.position {
            Some(0) | None => None,
            Some(position) => Some(position - 1),
        };
    }
}

impl TuiViewportIndex for DemoIndex {
    type ItemId = usize;
    type Item = DemoItem;

    fn with_cursor<R>(
        &self,
        position: TuiViewportIndexPosition<'_, Self::ItemId>,
        f: impl FnOnce(&mut dyn TuiViewportCursor<ItemId = Self::ItemId, Item = Self::Item>) -> R,
    ) -> R {
        let items = self.items.borrow();
        let position = match position {
            TuiViewportIndexPosition::Start => (!items.is_empty()).then_some(0),
            TuiViewportIndexPosition::End => items.len().checked_sub(1),
            TuiViewportIndexPosition::Item(id) => items.iter().position(|item| item.id == *id),
        };
        f(&mut DemoCursor { items, position })
    }

    fn update_heights(&self, updates: &[(Self::ItemId, usize)]) {
        let mut items = self.items.borrow_mut();
        for (id, height) in updates {
            if let Some(item) = items.iter_mut().find(|item| item.id == *id) {
                item.height = *height;
                item.needs_measurement = false;
            }
        }
    }
}

/// Builds the visible element for one block, rendering only its visible logical
/// rows and reporting a width-dependent full height for view-measured blocks.
fn render_item(
    request: ViewportRenderRequest<DemoItem>,
    _app: &AppContext,
) -> RenderedViewportItem {
    match request.item.measured_text {
        Some(text) => {
            let measured_full_height =
                usize::from(TuiText::new(text.clone()).desired_height(request.width));
            RenderedViewportItem {
                element: Box::new(
                    TuiClipped::new(TuiText::new(text))
                        .with_vertical_offset(request.visible_rows.start),
                ),
                measured_full_height: Some(measured_full_height),
            }
        }
        None => RenderedViewportItem {
            element: Box::new(
                TuiClipped::new(TuiText::new(request.item.lines.join("\n")).truncate())
                    .with_vertical_offset(request.visible_rows.start),
            ),
            measured_full_height: None,
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Demo view
// ─────────────────────────────────────────────────────────────────────────────

/// Removes the front-most block, dispatched through the shared core so the
/// view's typed-action + notify path is exercised.
#[derive(Debug, Clone, Copy)]
enum DemoAction {
    RemoveFront,
}

struct ViewportDemoView {
    items: Rc<RefCell<Vec<DemoItem>>>,
    viewport: TuiViewportHandle<usize>,
    quit: Rc<Cell<bool>>,
}

impl ViewportDemoView {
    fn new(quit: Rc<Cell<bool>>) -> Self {
        Self {
            items: Rc::new(RefCell::new(initial_items())),
            viewport: TuiViewportHandle::at_end(),
            quit,
        }
    }
}

impl Entity for ViewportDemoView {
    type Event = ();
}

impl TuiView for ViewportDemoView {
    fn ui_name() -> &'static str {
        "ViewportDemoView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        let bold = TuiStyle::default().add_modifier(Modifier::BOLD);
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);

        let at_end = self.viewport.is_at_end();
        let item_count = self.items.borrow().len();

        let index = DemoIndex {
            items: self.items.clone(),
        };
        let viewport_position = self.viewport.position();
        let viewport_for_scroll = self.viewport.clone();
        let list = TuiScrollable::new(TuiViewportedList::new(
            viewport_position,
            index,
            render_item,
            move |position| viewport_for_scroll.set_position(position),
        ));

        let quit_for_q = self.quit.clone();
        let quit_for_esc = self.quit.clone();
        Box::new(
            TuiEventHandler::new(
                TuiColumn::new()
                    .with_child(Box::new(
                        TuiText::new("WarpUI · TUI viewport harness")
                            .with_style(bold)
                            .truncate(),
                    ))
                    .with_child(Box::new(
                        TuiText::new("wheel: scroll · x: remove front block · q/Esc: quit")
                            .with_style(dim)
                            .truncate(),
                    ))
                    .with_child(Box::new(
                        TuiText::new(format!(
                            "{item_count} blocks · {}",
                            if at_end {
                                "at end"
                            } else {
                                "anchored (scrolled)"
                            }
                        ))
                        .with_style(dim)
                        .truncate(),
                    ))
                    .with_child(Box::new(TuiText::new("──── transcript ────").truncate()))
                    .with_child(Box::new(list)),
            )
            .on_key("x", |_, ctx, _| {
                ctx.dispatch_typed_action(DemoAction::RemoveFront)
            })
            .on_key("q", move |_, _, _| quit_for_q.set(true))
            .on_key("escape", move |_, _, _| quit_for_esc.set(true)),
        )
    }
}

impl TypedActionView for ViewportDemoView {
    type Action = DemoAction;

    fn handle_action(&mut self, action: &DemoAction, ctx: &mut ViewContext<Self>) {
        match action {
            DemoAction::RemoveFront => {
                let mut items = self.items.borrow_mut();
                if !items.is_empty() {
                    items.remove(0);
                }
            }
        }
        ctx.notify();
    }
}

fn main() {
    App::test((), |mut app| async move {
        let quit = Rc::new(Cell::new(false));
        let quit_for_view = quit.clone();
        let (window_id, root) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                move |_| ViewportDemoView::new(quit_for_view),
            )
        });

        let mut runtime =
            TuiRuntime::enter(&app, window_id, root).expect("enter the alternate screen");
        let quit_for_loop = quit.clone();
        runtime
            .run_until(&mut app, move |_| quit_for_loop.get())
            .expect("run the TUI loop");
    });
}
