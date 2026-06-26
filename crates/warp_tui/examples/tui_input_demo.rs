//! Interactive validation demo for `TuiInputView` + `TuiInputModel`.
//!
//! This is the Step 4 validation from `specs/tui-input-view/TECH.md`:
//! a real terminal session that proves the full editor-backed input stack works.
//!
//! Run:
//! ```sh
//! cargo run -p warp_tui --example tui_input_demo
//! ```
//!
//! Keys (full Emacs/readline keybinding table):
//!   Printable chars   insert text
//!   Shift+Enter       insert newline (multi-line)
//!   Ctrl+J            insert newline
//!   ←→↑↓             cursor movement
//!   Ctrl+A/E          line start/end
//!   Ctrl+B/F          char left/right
//!   Ctrl+P/N          line up/down
//!   Alt+B/F           word left/right
//!   Backspace         delete back
//!   Delete/Ctrl+D     delete forward
//!   Ctrl+W            delete word back
//!   Alt+D             delete word forward
//!   Ctrl+K            kill to line end
//!   Ctrl+U            kill to line start
//!   Ctrl+Y            yank
//!   Ctrl+Z            undo
//!   Enter             submit (prints text and quits)
//!   Esc               quit without submitting

use std::cell::Cell;
use std::rc::Rc;

use warp::editor::CodeEditorModel;
use warp_tui::input::{TuiInputView, TuiInputViewEvent};
use warpui_core::elements::tui::{
    Modifier, TuiColumn, TuiElement, TuiEventHandler, TuiParentElement, TuiStyle, TuiText,
};
use warpui_core::platform::WindowStyle;
use warpui_core::runtime::TuiRuntime;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView, ViewContext,
};

// ─────────────────────────────────────────────────────────────────────────────
// Shell view: wraps TuiInputView with a status bar and handles Submit/quit
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum ShellAction {
    Quit,
    Submitted(String),
}

struct ShellView {
    input_view: warpui_core::ViewHandle<TuiInputView>,
    quit: Rc<Cell<bool>>,
    last_submitted: Option<String>,
}

impl Entity for ShellView {
    type Event = ();
}

impl ShellView {
    fn new(quit: Rc<Cell<bool>>, ctx: &mut ViewContext<Self>) -> Self {
        let terminal_width = 80_u16;
        let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(terminal_width, ctx));

        // Create TuiInputView — subscribe to its Submitted event for submit handling.
        let input_view = ctx.add_typed_action_tui_view(move |ctx| {
            TuiInputView::new(input_model, terminal_width, ctx)
        });
        ctx.subscribe_to_view(&input_view, Self::handle_input_view_event);

        Self {
            input_view,
            quit,
            last_submitted: None,
        }
    }

    fn handle_input_view_event(
        &mut self,
        _view: warpui_core::ViewHandle<TuiInputView>,
        event: &TuiInputViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiInputViewEvent::Submitted(text) => {
                // Defer the dispatch: this callback runs from inside
                // `emit_event`, which has temporarily removed `ShellView` from
                // the window's view map. A synchronous `dispatch_typed_action`
                // here would fail to find `ShellView` in the responder chain, so
                // `ShellAction::Submitted` would never be handled. Deferring
                // queues it as an effect that runs after the view is reinserted.
                ctx.dispatch_typed_action_deferred(ShellAction::Submitted(text.clone()));
            }
        }
    }
}

impl TuiView for ShellView {
    fn ui_name() -> &'static str {
        "ShellView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let input_view = self.input_view.as_ref(ctx);
        let lines = input_view.visual_line_count(ctx);
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);
        let bold = TuiStyle::default().add_modifier(Modifier::BOLD);

        let mut column = TuiColumn::new();

        // ── Header ──────────────────────────────────────────────────────────
        column = column.with_child(Box::new(
            TuiText::new("TuiInputView validation demo")
                .with_style(bold)
                .truncate(),
        ));
        column = column.with_child(Box::new(
            TuiText::new(
                "Enter=submit · Esc=quit · Shift+Enter=newline · Ctrl+K/U/Y=kill/yank · Ctrl+Z=undo",
            )
            .with_style(dim)
            .truncate(),
        ));
        if let Some(ref submitted) = self.last_submitted {
            column = column.with_child(Box::new(
                TuiText::new(format!("Last submitted: {submitted:?}"))
                    .with_style(dim)
                    .truncate(),
            ));
        }
        column = column.with_child(Box::new(TuiText::new("─".repeat(80)).truncate()));

        // ── Prompt line ──────────────────────────────────────────────────────
        column = column.with_child(Box::new(
            TuiText::new(format!("≫  ({lines} visual rows)"))
                .with_style(dim)
                .truncate(),
        ));

        // ── TuiInputView (editor-backed) ─────────────────────────────────────
        column = column.with_child(Box::new(warpui_core::elements::tui::TuiChildView::new(
            &self.input_view,
        )));

        // ── Escape handler (quit) ─────────────────────────────────────────────
        Box::new(TuiEventHandler::new(column).on_key("escape", |_, ctx, _| {
            ctx.dispatch_typed_action(ShellAction::Quit)
        }))
    }

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<warpui_core::EntityId> {
        vec![self.input_view.id()]
    }

    fn keymap_context(&self, _ctx: &AppContext) -> warpui_core::keymap::Context {
        // Focus propagates into TuiInputView so keystrokes reach its dispatch_event.
        let mut ctx = warpui_core::keymap::Context::default();
        ctx.set.insert("ShellView");
        ctx
    }
}

impl TypedActionView for ShellView {
    type Action = ShellAction;

    fn handle_action(&mut self, action: &ShellAction, ctx: &mut ViewContext<Self>) {
        match action {
            ShellAction::Quit => self.quit.set(true),
            ShellAction::Submitted(text) => {
                self.last_submitted = Some(text.clone());
                ctx.notify();
                // Print submitted text after quitting so it's visible in the
                // regular terminal (not the alternate screen).
                eprintln!("\n[submitted] {text:?}");
                self.quit.set(true);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

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
                move |ctx| ShellView::new(quit_for_view, ctx),
            )
        });

        let mut runtime = TuiRuntime::enter(&app, window_id, root).expect("enter alternate screen");
        let quit_for_loop = quit.clone();
        runtime
            .run_until(&mut app, move |_| quit_for_loop.get())
            .expect("run TUI event loop");
    });
}
