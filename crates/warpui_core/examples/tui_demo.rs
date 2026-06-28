//! Interactive manual smoke test / showcase for the in-core TUI backend.
//!
//! Run it from a real terminal:
//!
//! ```sh
//! cargo run -p warpui_core --example tui_demo --features tui
//! ```
//!
//! It drives the real [`TuiRuntime`] against your terminal and exercises:
//! - **paragraph word-wrapping** — resize the width to watch the paragraph
//!   re-wrap on word boundaries,
//! - **wide-glyph rendering** — emoji, CJK, ZWJ sequences and a flag, to check
//!   that wide / zero-width grapheme clusters keep their columns aligned,
//! - **the ratatui buffer diff** — only changed cells are re-emitted between
//!   frames, and resizing reconciles instead of clearing (so no flicker),
//! - **vertical scrolling** — a long body scrolls in place under a fixed header.
//!
//! Keys: `j` / `↓` scroll down · `k` / `↑` scroll up · resize `↔` to re-wrap ·
//! `q` / `Esc` quit.
//!
//! It uses [`App::test`] only to stand up the shared core without the GUI
//! platform; the TUI backend itself renders to stdout, not a GUI window.

use std::cell::Cell;
use std::rc::Rc;

use warpui_core::elements::tui::{
    Modifier, TuiColumn, TuiElement, TuiEventHandler, TuiParentElement, TuiStyle, TuiText,
};
use warpui_core::platform::WindowStyle;
use warpui_core::runtime::TuiRuntime;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView, ViewContext,
};

/// A long line that mixes wide CJK, emoji, a ZWJ family/snowman and a flag, so
/// wrapping + grapheme-cluster width handling can be eyeballed as it reflows.
const WRAPPING_PARAGRAPH: &str = "Resize the terminal horizontally to watch this \
paragraph re-wrap on word boundaries. It deliberately mixes wide CJK 日本語 と 世界, \
emoji 😀 🎉 🚀, a polar-bear ZWJ sequence 🐻\u{200d}❄\u{fe0f}, a family 👨\u{200d}👩\u{200d}👧\u{200d}👦, \
and a flag 🇺🇸 so you can confirm that wide and zero-width grapheme clusters keep \
their columns aligned as the text reflows to the available width.";

/// Scroll actions, dispatched as typed actions through the shared core so the
/// runtime's typed-action path is exercised end to end.
#[derive(Debug, Clone, Copy)]
enum Scroll {
    Down,
    Up,
}

struct ShowcaseView {
    body: Vec<String>,
    scroll: usize,
    quit: Rc<Cell<bool>>,
}

impl ShowcaseView {
    fn new(quit: Rc<Cell<bool>>) -> Self {
        let emojis = [
            "🦊",
            "🚀",
            "🎉",
            "🐻\u{200d}❄\u{fe0f}",
            "🇺🇸",
            "✨",
            "🧠",
            "📦",
        ];
        let body = (0..40)
            .map(|i| {
                let emoji = emojis[i % emojis.len()];
                format!("row {i:02}  {emoji}  the quick brown fox jumps over 世界 ──────")
            })
            .collect();
        Self {
            body,
            scroll: 0,
            quit,
        }
    }
}

impl Entity for ShowcaseView {
    type Event = ();
}

impl TuiView for ShowcaseView {
    fn ui_name() -> &'static str {
        "ShowcaseView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        let bold = TuiStyle::default().add_modifier(Modifier::BOLD);
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);

        let mut rows: Vec<Box<dyn TuiElement>> = Vec::new();
        rows.push(Box::new(
            TuiText::new("WarpUI · TUI showcase")
                .with_style(bold)
                .truncate(),
        ));
        rows.push(Box::new(
            TuiText::new("j/↓ scroll · k/↑ up · resize ↔ to re-wrap · q quit")
                .with_style(dim)
                .truncate(),
        ));
        rows.push(Box::new(TuiText::new(" ")));
        // Wrapping paragraph: default (word-wrap) policy, so it reflows to width.
        rows.push(Box::new(TuiText::new(WRAPPING_PARAGRAPH)));
        rows.push(Box::new(TuiText::new(" ")));
        rows.push(Box::new(
            TuiText::new(format!("scroll {}/{}", self.scroll, self.body.len()))
                .with_style(dim)
                .truncate(),
        ));
        rows.push(Box::new(TuiText::new("──────── body ────────").truncate()));
        // Scrollable body: feed rows from the scroll offset; the column clips
        // whatever doesn't fit at the bottom, so the list scrolls in place.
        for line in &self.body[self.scroll.min(self.body.len())..] {
            rows.push(Box::new(TuiText::new(line.clone()).truncate()));
        }

        let quit_for_q = self.quit.clone();
        let quit_for_esc = self.quit.clone();
        Box::new(
            TuiEventHandler::new(TuiColumn::new().with_children(rows))
                .on_key("j", |_, ctx, _| ctx.dispatch_typed_action(Scroll::Down))
                .on_key("down", |_, ctx, _| ctx.dispatch_typed_action(Scroll::Down))
                .on_key("k", |_, ctx, _| ctx.dispatch_typed_action(Scroll::Up))
                .on_key("up", |_, ctx, _| ctx.dispatch_typed_action(Scroll::Up))
                .on_key("q", move |_, _, _| quit_for_q.set(true))
                .on_key("escape", move |_, _, _| quit_for_esc.set(true)),
        )
    }
}

impl TypedActionView for ShowcaseView {
    type Action = Scroll;

    fn handle_action(&mut self, action: &Scroll, ctx: &mut ViewContext<Self>) {
        let max = self.body.len().saturating_sub(1);
        match action {
            Scroll::Down => self.scroll = (self.scroll + 1).min(max),
            Scroll::Up => self.scroll = self.scroll.saturating_sub(1),
        }
        // Mark the view dirty so the runtime repaints with the new offset.
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
                move |_| ShowcaseView::new(quit_for_view),
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
