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
//! - **vertical scrolling** — a long body scrolls in place (clipped above and
//!   below) under a fixed header, via a real `TuiScrollable`.
//!
//! Keys: `↑`/`↓` · `PgUp`/`PgDn` · `Home`/`End` · `j`/`k` or the mouse wheel
//! scroll the body · resize `↔` to re-wrap · `q` / `Esc` quit.
//!
//! It uses [`App::test`] only to stand up the shared core without the GUI
//! platform; the TUI backend itself renders to stdout, not a GUI window.

use std::cell::Cell;
use std::rc::Rc;

use warpui_core::elements::tui::{
    Modifier, TuiColumn, TuiElement, TuiEventHandler, TuiScrollHandle, TuiScrollable, TuiStyle,
    TuiText,
};
use warpui_core::platform::WindowStyle;
use warpui_core::runtime::TuiRuntime;
use warpui_core::{AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView};

/// A long line that mixes wide CJK, emoji, a ZWJ family/snowman and a flag, so
/// wrapping + grapheme-cluster width handling can be eyeballed as it reflows.
const WRAPPING_PARAGRAPH: &str = "Resize the terminal horizontally to watch this \
paragraph re-wrap on word boundaries. It deliberately mixes wide CJK 日本語 と 世界, \
emoji 😀 🎉 🚀, a polar-bear ZWJ sequence 🐻\u{200d}❄\u{fe0f}, a family 👨\u{200d}👩\u{200d}👧\u{200d}👦, \
and a flag 🇺🇸 so you can confirm that wide and zero-width grapheme clusters keep \
their columns aligned as the text reflows to the available width.";

struct ShowcaseView {
    body: Vec<String>,
    scroll: TuiScrollHandle,
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
            scroll: TuiScrollHandle::new(),
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

        // Fixed header above the scrollable body.
        let header = TuiColumn::new()
            .child(
                TuiText::new("WarpUI · TUI showcase")
                    .with_style(bold)
                    .truncate(),
            )
            .child(
                TuiText::new("↑/↓ · PgUp/PgDn · Home/End · wheel · j/k scroll · q quit")
                    .with_style(dim)
                    .truncate(),
            )
            .child(TuiText::new(" "))
            // Wrapping paragraph: default (word-wrap) policy, so it reflows to width.
            .child(TuiText::new(WRAPPING_PARAGRAPH))
            .child(TuiText::new(" "))
            .child(
                TuiText::new(format!("body: {} rows (scrolls below)", self.body.len()))
                    .with_style(dim)
                    .truncate(),
            )
            .child(TuiText::new("──────── body ────────").truncate());

        // Scrollable body: a column of every row, clipped to the viewport the
        // flex layout gives it and scrolled through the shared handle.
        let body_rows = self
            .body
            .iter()
            .map(|line| Box::new(TuiText::new(line.clone()).truncate()) as Box<dyn TuiElement>);
        let body = TuiScrollable::new(self.scroll.clone(), TuiColumn::with_children(body_rows));

        let content = TuiColumn::new().child(header).flex_child(body);

        // The element handles the wheel and arrow/page/home/end keys itself;
        // `j`/`k` drive the shared handle directly (clamped on the next layout).
        let scroll_for_j = self.scroll.clone();
        let scroll_for_k = self.scroll.clone();
        let quit_for_q = self.quit.clone();
        let quit_for_esc = self.quit.clone();
        Box::new(
            TuiEventHandler::new(content)
                .on_key("j", move |_, _, _| {
                    scroll_for_j.set_offset(scroll_for_j.offset().saturating_add(1))
                })
                .on_key("k", move |_, _, _| {
                    scroll_for_k.set_offset(scroll_for_k.offset().saturating_sub(1))
                })
                .on_key("q", move |_, _, _| quit_for_q.set(true))
                .on_key("escape", move |_, _, _| quit_for_esc.set(true)),
        )
    }
}

impl TypedActionView for ShowcaseView {
    // No typed actions: scrolling is owned by the `TuiScrollable` element and the
    // `j`/`k` handlers, which drive the shared scroll handle directly.
    type Action = ();
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
