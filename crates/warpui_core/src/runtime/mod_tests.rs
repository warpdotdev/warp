use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::Duration;

use ratatui::crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::elements::tui::{
    TuiBuffer, TuiChildView, TuiConstraint, TuiElement, TuiEventHandler, TuiLayoutContext,
    TuiStyle, TuiText,
};
use crate::platform::WindowStyle;
use crate::{AddWindowOptions, AppContext, Entity, TypedActionView, ViewContext};

/// A trivial leaf element that paints a single line of text.
struct TextElement {
    text: String,
}

impl TuiElement for TextElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let width = u16::try_from(self.text.chars().count()).unwrap_or(u16::MAX);
        constraint.clamp(TuiSize::new(width, 1))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {
        buffer.set_stringn(
            area.x,
            area.y,
            &self.text,
            usize::from(area.width),
            TuiStyle::default(),
        );
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
            TuiEventHandler::new(TuiText::new("child"))
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
