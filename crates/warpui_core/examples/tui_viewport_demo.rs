//! Interactive manual harness for the generalized TUI viewport stack.
//!
//! Run it from a real terminal:
//!
//! ```sh
//! cargo run -p warpui_core --example tui_viewport_demo --features tui
//! ```
//!
//! It drives the real [`TuiRuntime`] against your terminal over a trivial
//! in-memory source of text blocks, exercising every piece the viewport ships so
//! all of it can be eyeballed in isolation (no `TerminalModel`/AI dependency):
//! - **`TuiViewportedElement`** — the in-memory source maps an absolute row
//!   window to visible elements and content-space origins.
//! - **`TuiViewportedList` + caller-owned `TuiViewportedListState`** — starts at the
//!   content end; wheel-scrolling up stores an absolute row offset; scrolling
//!   back to the end restores the end position.
//! - **`TuiScrollable` + wheel conversion + mouse capture** — the mouse wheel
//!   over the list scrolls it (the only path that exercises wheel→`ScrollWheel`
//!   conversion and the alternate-screen mouse capture end to end).
//! - **internal clipping** — top-clipped blocks are returned as full elements;
//!   the viewport offsets them to the first visible logical row.
//! - **height reconciliation** — view-measured blocks report a width-dependent
//!   full height; resizing the terminal re-measures and writes the new height
//!   back into the source.
//!
//! Keys: mouse wheel scroll · `x` remove the front block · `q` / `Esc` quit.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use warpui_core::elements::tui::{
    Modifier, TuiElement, TuiEventHandler, TuiFlex, TuiParentElement, TuiScrollable, TuiStyle,
    TuiText, TuiViewportContent, TuiViewportWindow, TuiViewportedElement, TuiViewportedList,
    TuiViewportedListState, TuiVisibleViewportItem,
};
use warpui_core::platform::WindowStyle;
use warpui_core::runtime::TuiRuntime;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView, ViewContext,
};

// ─────────────────────────────────────────────────────────────────────────────
// Backing content: a trivial in-memory list of text blocks with interior
// mutability, mirroring the unit-test `FakeContent` shape so the demo exercises
// the same trait the real terminal-history source will implement.
// ─────────────────────────────────────────────────────────────────────────────

/// One block in the demo transcript. Fixed blocks own their rows; view-measured
/// blocks own a wrapping paragraph whose height depends on the render width.
#[derive(Clone)]
struct DemoItem {
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
                    lines: Vec::new(),
                    measured_text: Some(format!(
                        "block {id} (view-measured): a long wrapping paragraph that reflows to \
                         the available width, so resizing the terminal re-measures its height and \
                         writes the new height back into the content source."
                    )),
                    height: 1,
                    needs_measurement: true,
                }
            } else {
                let rows = 3 + (id % 3);
                DemoItem {
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

/// An absolute-row viewport source over a shared `Vec<DemoItem>`.
struct DemoViewportContent {
    items: Rc<RefCell<Vec<DemoItem>>>,
    last_width: Rc<Cell<Option<u16>>>,
}

impl TuiViewportedElement for DemoViewportContent {
    fn visible_items(
        &self,
        window: TuiViewportWindow,
        available_width: u16,
        _app: &AppContext,
    ) -> TuiViewportContent {
        let width_changed = self.last_width.replace(Some(available_width)) != Some(available_width);

        let mut items = self.items.borrow_mut();
        for item in items.iter_mut() {
            if let Some(text) = &item.measured_text {
                if item.needs_measurement || width_changed {
                    item.height =
                        usize::from(TuiText::new(text.clone()).desired_height(available_width));
                    item.needs_measurement = false;
                }
            }
        }

        let viewport_bottom = window
            .scroll_top
            .saturating_add(usize::from(window.viewport_height));
        let mut origin_y = 0usize;
        let mut visible_items = Vec::new();
        for item in items.iter() {
            let item_top = origin_y;
            let item_bottom = item_top.saturating_add(item.height);
            if item_bottom > window.scroll_top && item_top < viewport_bottom {
                visible_items.push(TuiVisibleViewportItem {
                    origin_y: item_top,
                    element: render_item(item),
                });
            }
            origin_y = item_bottom;
        }

        TuiViewportContent {
            content_height: origin_y,
            items: visible_items,
        }
    }
}

/// Builds the full element for one block; the viewport owns partial clipping.
fn render_item(item: &DemoItem) -> Box<dyn TuiElement> {
    match &item.measured_text {
        Some(text) => Box::new(TuiText::new(text.clone())),
        None => Box::new(TuiText::new(item.lines.join("\n")).truncate()),
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
    last_width: Rc<Cell<Option<u16>>>,
    viewport: TuiViewportedListState,
    quit: Rc<Cell<bool>>,
}

impl ViewportDemoView {
    fn new(quit: Rc<Cell<bool>>) -> Self {
        Self {
            items: Rc::new(RefCell::new(initial_items())),
            last_width: Rc::new(Cell::new(None)),
            viewport: TuiViewportedListState::new_at_end(),
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

        let content = DemoViewportContent {
            items: self.items.clone(),
            last_width: self.last_width.clone(),
        };
        let list = TuiScrollable::new(TuiViewportedList::new(self.viewport.clone(), content));

        let quit_for_q = self.quit.clone();
        let quit_for_esc = self.quit.clone();
        Box::new(
            TuiEventHandler::new(
                TuiFlex::column()
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
                                "absolute row offset"
                            }
                        ))
                        .with_style(dim)
                        .truncate(),
                    ))
                    .with_child(Box::new(TuiText::new("──── transcript ────").truncate()))
                    .with_child(Box::new(list))
                    .finish(),
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
