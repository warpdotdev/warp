//! Playground: render a file from disk inside the TUI runtime.
//!
//! This validates the TUI rendering pipeline end-to-end:
//!   - `TuiRuntime` hosting a scrollable view
//!   - `TuiText` word-wrapping for markdown/long-line content
//!   - Resize reflows cleanly (no flicker)
//!   - Full event loop (keypress → action → `ctx.notify()` → repaint)
//!
//! **Note**: The full editor-backed `TuiInputView` lives in `crates/warp_tui/src/input/`
//! and is exercised by the real `warp-tui` binary (`./script/run-tui`). This
//! example proves out the TUI runtime layer independently of the editor.
//!
//! Run from a real terminal:
//!
//! ```sh
//! cargo run -p warpui_core --example tui_file_viewer --features tui -- /path/to/file.md
//! ```
//!
//! Keys:
//!   `j` / `↓`    scroll down one line
//!   `k` / `↑`    scroll up one line
//!   `d` / `PgDn` scroll down half a page
//!   `u` / `PgUp` scroll up half a page
//!   `g`          jump to top
//!   `G`          jump to bottom
//!   `q` / `Esc`  quit

use std::cell::Cell;
use std::rc::Rc;

use warpui_core::elements::tui::{
    Modifier, TuiElement, TuiEventHandler, TuiFlex, TuiParentElement, TuiStyle, TuiText,
};
use warpui_core::platform::WindowStyle;
use warpui_core::runtime::TuiRuntime;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView, ViewContext,
};

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Nav {
    LineDown,
    LineUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    Quit,
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

struct FileView {
    path: String,
    /// Raw lines of the file (no further processing — markdown renders as-is).
    lines: Vec<String>,
    /// Index of the first visible line.
    scroll: usize,
    quit: Rc<Cell<bool>>,
}

impl FileView {
    fn new(path: String, content: String, quit: Rc<Cell<bool>>) -> Self {
        // Preserve empty lines so paragraph spacing in markdown is visible.
        let lines = content.lines().map(|l| l.to_string()).collect();
        Self {
            path,
            lines,
            scroll: 0,
            quit,
        }
    }

    fn max_scroll(&self, visible_rows: usize) -> usize {
        self.lines.len().saturating_sub(visible_rows)
    }
}

impl Entity for FileView {
    type Event = ();
}

impl TuiView for FileView {
    fn ui_name() -> &'static str {
        "FileView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        let bold = TuiStyle::default().add_modifier(Modifier::BOLD);
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);

        // Build the full column of rows.
        let mut rows: Vec<Box<dyn TuiElement>> = Vec::new();

        // ── Header ──────────────────────────────────────────────────────────
        rows.push(Box::new(
            TuiText::new(format!("  {}", self.path))
                .with_style(bold)
                .truncate(),
        ));
        rows.push(Box::new(
            TuiText::new(format!(
                "  line {}/{} · j/k scroll · d/u page · g/G top/bottom · q quit",
                self.scroll + 1,
                self.lines.len().max(1),
            ))
            .with_style(dim)
            .truncate(),
        ));
        rows.push(Box::new(TuiText::new("─".repeat(80)).truncate()));

        // ── Body: lines from scroll offset ─────────────────────────────────
        // The TuiFlex clips children that don't fit vertically, so we can
        // push all remaining lines and the presenter will stop rendering once
        // the allocated area is exhausted.
        for line in &self.lines[self.scroll..] {
            if line.is_empty() {
                // Preserve blank lines for paragraph spacing.
                rows.push(Box::new(TuiText::new(" ")));
            } else {
                // Word-wrap at the terminal width — the default TuiText policy.
                rows.push(Box::new(TuiText::new(line.clone())));
            }
        }

        // ── Wire up key handlers ────────────────────────────────────────────
        Box::new(
            TuiEventHandler::new(TuiFlex::column().with_children(rows).finish())
                .on_key("j", |_, ctx, _| ctx.dispatch_typed_action(Nav::LineDown))
                .on_key("down", |_, ctx, _| ctx.dispatch_typed_action(Nav::LineDown))
                .on_key("k", |_, ctx, _| ctx.dispatch_typed_action(Nav::LineUp))
                .on_key("up", |_, ctx, _| ctx.dispatch_typed_action(Nav::LineUp))
                .on_key("d", |_, ctx, _| ctx.dispatch_typed_action(Nav::PageDown))
                .on_key("pagedown", |_, ctx, _| {
                    ctx.dispatch_typed_action(Nav::PageDown)
                })
                .on_key("u", |_, ctx, _| ctx.dispatch_typed_action(Nav::PageUp))
                .on_key("pageup", |_, ctx, _| ctx.dispatch_typed_action(Nav::PageUp))
                .on_key("g", |_, ctx, _| ctx.dispatch_typed_action(Nav::Top))
                .on_key("G", |_, ctx, _| ctx.dispatch_typed_action(Nav::Bottom))
                .on_key("q", |_, ctx, _| ctx.dispatch_typed_action(Nav::Quit))
                .on_key("escape", |_, ctx, _| ctx.dispatch_typed_action(Nav::Quit)),
        )
    }
}

impl TypedActionView for FileView {
    type Action = Nav;

    fn handle_action(&mut self, action: &Nav, ctx: &mut ViewContext<Self>) {
        // Approximate half-page as 10 lines; the real page size would require
        // knowing the terminal height at action time, which is available via
        // the presenter's resize event. Good enough for a playground.
        const HALF_PAGE: usize = 10;

        let total = self.lines.len();
        match action {
            Nav::LineDown => {
                // Use a generous viewport estimate; actual clipping is in render.
                self.scroll = (self.scroll + 1).min(self.max_scroll(3));
            }
            Nav::LineUp => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            Nav::PageDown => {
                self.scroll = (self.scroll + HALF_PAGE).min(self.max_scroll(3));
            }
            Nav::PageUp => {
                self.scroll = self.scroll.saturating_sub(HALF_PAGE);
            }
            Nav::Top => {
                self.scroll = 0;
            }
            Nav::Bottom => {
                self.scroll = total.saturating_sub(3);
            }
            Nav::Quit => {
                self.quit.set(true);
            }
        }
        ctx.notify();
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: tui_file_viewer <path>");
        std::process::exit(1);
    });

    let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("error reading {path}: {e}");
        std::process::exit(1);
    });

    App::test((), |mut app| async move {
        let quit = Rc::new(Cell::new(false));
        let quit_for_view = quit.clone();
        let path_clone = path.clone();

        let (window_id, root) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                move |_| FileView::new(path_clone, content, quit_for_view),
            )
        });

        let mut runtime = TuiRuntime::enter(&app, window_id, root).expect("enter alternate screen");
        let quit_for_loop = quit.clone();
        runtime
            .run_until(&mut app, move |_| quit_for_loop.get())
            .expect("run TUI event loop");
    });
}
