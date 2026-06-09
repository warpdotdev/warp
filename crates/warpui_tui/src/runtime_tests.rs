use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::Duration;

use crossterm::event::Event as CrosstermEvent;
use warpui_core::platform::WindowStyle;
use warpui_core::{AddWindowOptions, App, AppContext, Entity, TuiTypedActionView};

use super::*;
use crate::{
    TuiBuffer, TuiConstraint, TuiElement, TuiRect, TuiRenderOutput, TuiSize, TuiStyle, TuiView,
};

/// A trivial leaf element that paints a single line of text.
struct TextElement {
    text: String,
}

impl TuiElement for TextElement {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        let width = u16::try_from(self.text.chars().count()).unwrap_or(u16::MAX);
        constraint.clamp(TuiSize::new(width, 1))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        buffer.set_str(area.x, area.y, area.width, &self.text, TuiStyle::default());
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}

/// A minimal root view that renders the text "hello".
struct TextView;

impl Entity for TextView {
    type Event = ();
}

impl TuiView for TextView {
    type RenderOutput = TuiRenderOutput;

    fn ui_name() -> &'static str {
        "TextView"
    }

    fn render_tui(&self, _ctx: &AppContext) -> TuiRenderOutput {
        Box::new(TextElement {
            text: "hello".to_owned(),
        })
    }
}

impl TuiTypedActionView for TextView {
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
