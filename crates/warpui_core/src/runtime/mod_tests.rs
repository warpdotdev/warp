use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::Duration;

use ratatui::crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::elements::tui::{
    TuiChildView, TuiConstraint, TuiElement, TuiEventHandler, TuiFlex, TuiHoverable,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPoint, TuiScreenPoint,
    TuiScreenPosition, TuiText,
};
use crate::elements::MouseStateHandle;
use crate::keymap::macros::*;
use crate::keymap::FixedBinding;
use crate::platform::WindowStyle;
use crate::{AddWindowOptions, AppContext, Entity, TypedActionView, ViewContext};

/// A trivial leaf element that paints a single line of text.
struct TextElement {
    text: String,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiElement for TextElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let width = u16::try_from(self.text.chars().count()).unwrap_or(u16::MAX);
        let size = constraint.clamp(TuiSize::new(width, 1));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let size = self.size.unwrap();
        for (column, character) in self.text.chars().take(usize::from(size.width)).enumerate() {
            if let Some(cell) =
                surface.cell_mut(origin.offset(i32::try_from(column).unwrap_or(i32::MAX), 0))
            {
                cell.set_char(character);
            }
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

/// A minimal root view that renders the text "hello".
struct TextView;

impl Entity for TextView {
    type Event = ();
}

impl TuiView for TextView {
    fn ui_name() -> &'static str {
        "TextView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TextElement {
            text: "hello".to_owned(),
            size: None,
            origin: None,
        })
    }
}

impl TypedActionView for TextView {
    type Action = ();
}

/// An in-memory [`TuiTerminal`] that captures the renderer's bytes and replays a
/// fixed queue of input events.
struct TestTerminal {
    size: TuiSize,
    output: Vec<u8>,
    events: VecDeque<CrosstermEvent>,
}

impl TestTerminal {
    fn new(size: TuiSize) -> Self {
        Self {
            size,
            output: Vec::new(),
            events: VecDeque::new(),
        }
    }

    fn output_string(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

impl TuiTerminal for TestTerminal {
    fn size(&self) -> io::Result<TuiSize> {
        Ok(self.size)
    }

    fn poll_event(&mut self, _timeout: Duration) -> io::Result<Option<CrosstermEvent>> {
        Ok(self.events.pop_front())
    }

    fn writer(&mut self) -> &mut dyn Write {
        &mut self.output
    }
}

fn window_options() -> AddWindowOptions {
    AddWindowOptions {
        window_style: WindowStyle::NotStealFocus,
        ..Default::default()
    }
}

#[test]
fn run_until_draws_view_text_and_exits_on_quit() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| TextView));
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut runtime = TuiRuntime::with_terminal(&app, window_id, root, terminal);

        // Quit after the first iteration so a single draw pass runs and the loop
        // provably terminates rather than spinning forever.
        let mut iterations = 0;
        runtime
            .run_until(&mut app, |_| {
                iterations += 1;
                iterations > 1
            })
            .unwrap();

        assert!(iterations <= 2, "run_until should exit promptly");
        assert!(
            runtime.terminal().output_string().contains("hello"),
            "the view's text should be drawn to the in-memory terminal"
        );
    });
}

/// The typed action only the parent view handles in the embedded-child test.
#[derive(Debug)]
struct Bump;

/// A leaf TUI view whose subtree raises a typed action on `b`.
struct BumpChildView;

impl Entity for BumpChildView {
    type Event = ();
}

impl TuiView for BumpChildView {
    fn ui_name() -> &'static str {
        "BumpChildView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(
            TuiEventHandler::new(TuiText::new("child").finish())
                .on_key("b", |_, ctx, _| ctx.dispatch_typed_action(Bump)),
        )
    }
}

/// The window root: embeds [`BumpChildView`] and handles [`Bump`].
struct BumpParentView {
    child: crate::ViewHandle<BumpChildView>,
    bumps: usize,
}

impl Entity for BumpParentView {
    type Event = ();
}

impl TuiView for BumpParentView {
    fn ui_name() -> &'static str {
        "BumpParentView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        Box::new(TuiChildView::new(&self.child))
    }
}

impl TypedActionView for BumpParentView {
    type Action = Bump;

    fn handle_action(&mut self, _action: &Bump, _ctx: &mut ViewContext<Self>) {
        self.bumps += 1;
    }
}

/// The keymap pass: a keystroke binding whose context predicate matches a TUI
/// view's keymap context dispatches its typed action through the responder
/// chain — no element-level key handler is involved.
#[test]
fn keymap_binding_dispatches_typed_action_to_tui_view() {
    App::test((), |mut app| async move {
        let (window_id, root) = app.update(|ctx| {
            ctx.register_fixed_bindings([FixedBinding::new("ctrl-c", Bump, id!("BumpParentView"))]);
            ctx.add_tui_window(window_options(), |view_ctx| {
                let child = view_ctx.add_tui_view(|_| BumpChildView);
                BumpParentView { child, bumps: 0 }
            })
        });

        let mut terminal = TestTerminal::new(TuiSize::new(20, 3));
        terminal.events.push_back(CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        let root_for_runtime = root.clone();
        let mut runtime = TuiRuntime::with_terminal(&app, window_id, root_for_runtime, terminal);

        let mut iterations = 0;
        runtime
            .run_until(&mut app, |_| {
                iterations += 1;
                iterations > 1
            })
            .unwrap();

        assert_eq!(
            root.read(&app, |view, _| view.bumps),
            1,
            "the keymap pass should dispatch the bound action to the focused TUI view"
        );
    });
}

/// End-to-end regression for the Shift+Enter fix: a Shift+Enter key event —
/// the distinct event a terminal only sends once the Kitty keyboard protocol
/// is enabled (see `terminal_screen_lifecycle_toggles_keyboard_enhancement`) —
/// must flow through crossterm decoding, `crossterm_event_to_tui_event`, and
/// the keymap responder chain to dispatch its bound action. This is the exact
/// path the TUI input's `shift-enter` -> insert-newline binding relies on; the
/// bug was that, without the protocol, Shift+Enter arrived indistinguishable
/// from Enter and this event never occurred.
#[test]
fn shift_enter_key_event_dispatches_bound_action() {
    App::test((), |mut app| async move {
        let (window_id, root) = app.update(|ctx| {
            ctx.register_fixed_bindings([FixedBinding::new(
                "shift-enter",
                Bump,
                id!("BumpParentView"),
            )]);
            ctx.add_tui_window(window_options(), |view_ctx| {
                let child = view_ctx.add_tui_view(|_| BumpChildView);
                BumpParentView { child, bumps: 0 }
            })
        });

        let mut terminal = TestTerminal::new(TuiSize::new(20, 3));
        terminal.events.push_back(CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT,
        )));
        let root_for_runtime = root.clone();
        let mut runtime = TuiRuntime::with_terminal(&app, window_id, root_for_runtime, terminal);

        let mut iterations = 0;
        runtime
            .run_until(&mut app, |_| {
                iterations += 1;
                iterations > 1
            })
            .unwrap();

        assert_eq!(
            root.read(&app, |view, _| view.bumps),
            1,
            "a Shift+Enter key event should dispatch the bound shift-enter action"
        );
    });
}

/// A binding with a permissive (always-true) context predicate whose action
/// type has no handler on any view in the TUI responder chain must not swallow
/// the keystroke: the keymap pass reports it unhandled and the element pass
/// still runs. This is what keeps GUI-registered bindings inert in the TUI
/// even when they are missing a context predicate.
#[test]
fn unhandled_keymap_binding_falls_through_to_element_pass() {
    /// An action type no TUI view registers a handler for.
    #[derive(Debug)]
    struct GuiOnlyAction;

    App::test((), |mut app| async move {
        let (window_id, root) = app.update(|ctx| {
            ctx.register_fixed_bindings([FixedBinding::new("b", GuiOnlyAction, always!())]);
            ctx.add_tui_window(window_options(), |view_ctx| {
                let child = view_ctx.add_tui_view(|_| BumpChildView);
                BumpParentView { child, bumps: 0 }
            })
        });

        let mut terminal = TestTerminal::new(TuiSize::new(20, 3));
        terminal.events.push_back(CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::empty(),
        )));
        let root_for_runtime = root.clone();
        let mut runtime = TuiRuntime::with_terminal(&app, window_id, root_for_runtime, terminal);

        let mut iterations = 0;
        runtime
            .run_until(&mut app, |_| {
                iterations += 1;
                iterations > 1
            })
            .unwrap();

        assert_eq!(
            root.read(&app, |view, _| view.bumps),
            1,
            "a matched-but-unhandled binding must fall through to the element pass"
        );
    });
}

#[test]
fn typed_action_from_embedded_child_reaches_parent_through_runtime_dispatch() {
    App::test((), |mut app| async move {
        let (window_id, root) = app.update(|ctx| {
            ctx.add_tui_window(window_options(), |view_ctx| {
                let child = view_ctx.add_tui_view(|_| BumpChildView);
                BumpParentView { child, bumps: 0 }
            })
        });

        let mut terminal = TestTerminal::new(TuiSize::new(20, 3));
        terminal.events.push_back(CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::empty(),
        )));
        let root_for_runtime = root.clone();
        let mut runtime = TuiRuntime::with_terminal(&app, window_id, root_for_runtime, terminal);

        // Two iterations: the first draws (reporting the child embedding into
        // the shared view hierarchy) and dispatches the queued `b` key; the
        // second exits.
        let mut iterations = 0;
        runtime
            .run_until(&mut app, |_| {
                iterations += 1;
                iterations > 1
            })
            .unwrap();

        // The action was raised inside the embedded child view's subtree and
        // dispatched from the child's id; the shared responder chain bubbled it
        // to the parent's handler. (The legacy origin-only dispatch could not
        // do this.)
        assert_eq!(root.read(&app, |view, _| view.bumps), 1);
    });
}

/// The typed action that shifts [`ShiftingHoverView`]'s hover target down a row.
#[derive(Debug)]
struct Shift;

/// A root view whose hover target moves down one row after [`Shift`], used to
/// verify the post-draw synthetic mouse move refreshes hover state.
struct ShiftingHoverView {
    hover: MouseStateHandle,
    shifted: bool,
}

impl Entity for ShiftingHoverView {
    type Event = ();
}

impl TuiView for ShiftingHoverView {
    fn ui_name() -> &'static str {
        "ShiftingHoverView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        let mut column = TuiFlex::column();
        if self.shifted {
            column = column.child(TuiText::new("pad").finish());
        }
        let target = TuiHoverable::new(self.hover.clone(), TuiText::new("target").finish());
        column = column.child(target.finish());
        Box::new(
            TuiEventHandler::new(column.finish())
                .on_key("s", |_, ctx, _| ctx.dispatch_typed_action(Shift)),
        )
    }
}

impl TypedActionView for ShiftingHoverView {
    type Action = Shift;

    fn handle_action(&mut self, _action: &Shift, ctx: &mut ViewContext<Self>) {
        self.shifted = true;
        ctx.notify();
    }
}

/// After a redraw, the runtime replays the last pointer position as a
/// synthetic move, so a hover target that shifts out from under a stationary
/// mouse unhoveres without any real mouse movement.
#[test]
fn synthetic_mouse_move_after_redraw_updates_hover() {
    App::test((), |mut app| async move {
        let hover = MouseStateHandle::default();
        let hover_for_view = hover.clone();
        let (window_id, root) = app.update(move |ctx| {
            ctx.add_tui_window(window_options(), move |_| ShiftingHoverView {
                hover: hover_for_view,
                shifted: false,
            })
        });
        let terminal = TestTerminal::new(TuiSize::new(20, 5));
        let mut screen = TuiScreen::new(window_id, root.clone(), terminal);
        app.update(|ctx| screen.draw(ctx)).unwrap();

        let mouse_moved = TuiEvent::MouseMoved {
            position: TuiPoint::new(2, 0),
            modifiers: ModifiersState::default(),
            is_synthetic: false,
        };
        app.update(|ctx| screen.dispatch_event(ctx, &mouse_moved));
        assert!(hover.lock().unwrap().is_hovered());

        root.update(&mut app, |view, ctx| {
            view.shifted = true;
            ctx.notify();
        });
        screen.terminal.output.clear();

        app.update(|ctx| screen.draw(ctx)).unwrap();

        assert!(
            !hover.lock().unwrap().is_hovered(),
            "the post-draw synthetic move should unhover the shifted target"
        );
        assert_eq!(
            screen
                .terminal
                .output_string()
                .matches("\u{1b}[?2026h")
                .count(),
            1,
            "multi-pass hover reconciliation should flush one terminal frame"
        );
    });
}

/// Records the mode-control enter/leave calls so the guard's lifecycle can be
/// asserted without touching a real terminal.
struct RecordingControl {
    log: Rc<RefCell<Vec<&'static str>>>,
    fail_enter: bool,
}

impl TerminalModeControl for RecordingControl {
    fn enter(&mut self) -> io::Result<()> {
        if self.fail_enter {
            return Err(io::Error::other("enter failed"));
        }
        self.log.borrow_mut().push("enter");
        Ok(())
    }

    fn leave(&mut self) {
        self.log.borrow_mut().push("leave");
    }
}

#[test]
fn terminal_screen_lifecycle_toggles_bracketed_paste() {
    let mut enter_output = Vec::new();
    enter_terminal_screen(&mut enter_output, true).unwrap();
    assert!(
        enter_output
            .windows(b"\x1b[?2004h".len())
            .any(|window| window == b"\x1b[?2004h"),
        "entering the TUI should enable bracketed paste"
    );

    let mut leave_output = Vec::new();
    leave_terminal_screen(&mut leave_output, true).unwrap();
    assert!(
        leave_output
            .windows(b"\x1b[?2004l".len())
            .any(|window| window == b"\x1b[?2004l"),
        "leaving the TUI should disable bracketed paste"
    );
}

/// Regression test for Shift+Enter not inserting a newline in terminals that
/// require the Kitty keyboard protocol (e.g. Ghostty): entering the TUI must
/// push the `DISAMBIGUATE_ESCAPE_CODES` enhancement flag (CSI `>1u`) so modified
/// keys are reported distinctly, and leaving must pop it (CSI `<1u`).
///
/// Crossterm hard-routes these commands to the unsupported legacy Windows
/// console API, so the ANSI sequences are only emitted off Windows. The
/// enter/leave calls must still succeed on every platform (the `.unwrap()`s
/// below), and the byte assertions are gated to non-Windows where the sequences
/// are actually written.
#[test]
fn terminal_screen_lifecycle_toggles_keyboard_enhancement() {
    let mut enter_output = Vec::new();
    enter_terminal_screen(&mut enter_output, true).unwrap();

    let mut leave_output = Vec::new();
    leave_terminal_screen(&mut leave_output, true).unwrap();

    #[cfg(not(windows))]
    {
        assert!(
            enter_output
                .windows(b"\x1b[>1u".len())
                .any(|window| window == b"\x1b[>1u"),
            "entering the TUI should push the DISAMBIGUATE_ESCAPE_CODES keyboard enhancement flag"
        );
        assert!(
            leave_output
                .windows(b"\x1b[<1u".len())
                .any(|window| window == b"\x1b[<1u"),
            "leaving the TUI should pop the keyboard enhancement flags"
        );
    }
}

#[test]
fn terminal_screen_lifecycle_skips_unsupported_keyboard_enhancement() {
    let mut enter_output = Vec::new();
    enter_terminal_screen(&mut enter_output, false).unwrap();
    assert!(!enter_output
        .windows(b"\x1b[>1u".len())
        .any(|window| window == b"\x1b[>1u"));

    let mut leave_output = Vec::new();
    leave_terminal_screen(&mut leave_output, false).unwrap();
    assert!(!leave_output
        .windows(b"\x1b[<1u".len())
        .any(|window| window == b"\x1b[<1u"));
}
#[test]
fn raw_mode_guard_restores_on_drop() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let control = RecordingControl {
        log: log.clone(),
        fail_enter: false,
    };
    {
        let _guard = RawModeGuard::enter(control).unwrap();
        assert_eq!(*log.borrow(), vec!["enter"]);
    }
    assert_eq!(
        *log.borrow(),
        vec!["enter", "leave"],
        "dropping the guard should restore the terminal"
    );
}

#[test]
fn raw_mode_guard_does_not_leave_when_enter_fails() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let control = RecordingControl {
        log: log.clone(),
        fail_enter: true,
    };
    assert!(RawModeGuard::enter(control).is_err());
    assert!(
        log.borrow().is_empty(),
        "a failed enter must not run the leave/restore path"
    );
}
