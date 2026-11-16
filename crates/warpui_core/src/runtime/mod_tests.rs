use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::Duration;

use ratatui::crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode,
    MouseEvent, MouseEventKind,
};

use super::*;
use crate::elements::MouseStateHandle;
use crate::elements::tui::{
    TuiChildView, TuiConstraint, TuiElement, TuiEventHandler, TuiFlex, TuiHoverable,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPoint, TuiScreenPoint,
    TuiScreenPosition, TuiText,
};
use crate::keymap::FixedBinding;
use crate::keymap::macros::*;
use crate::platform::WindowStyle;
use crate::{AddWindowOptions, AppContext, Entity, TypedActionView, ViewContext};

/// A trivial leaf element that paints a single line of text.
struct TextElement {
    text: String,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

#[test]
fn blocking_runtime_continues_repaint_deadlines_while_unfocused_by_default() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| RepaintingView));
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut runtime = TuiRuntime::with_terminal(&app, window_id, root, terminal);

        runtime.draw_if_dirty(&mut app).unwrap();
        assert!(runtime.pending_repaint.is_some());

        runtime
            .screen
            .terminal
            .events
            .push_back(CrosstermEvent::FocusLost);
        runtime.poll_and_dispatch(&mut app, Duration::ZERO).unwrap();
        assert!(!runtime.focused);
        assert!(
            runtime.pending_repaint.is_some(),
            "unfocused repaint suspension should be opt-in"
        );

        runtime.dirty.set(true);
        runtime.draw_if_dirty(&mut app).unwrap();
        assert!(runtime.pending_repaint.is_some());
    });
}

#[test]
fn blocking_runtime_suspends_and_resumes_repaint_deadlines_when_enabled() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| RepaintingView));
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut runtime = TuiRuntime::with_terminal(&app, window_id, root, terminal);
        runtime.freeze_repaints_when_unfocused = true;

        runtime.draw_if_dirty(&mut app).unwrap();
        assert!(runtime.pending_repaint.is_some());

        runtime
            .screen
            .terminal
            .events
            .push_back(CrosstermEvent::FocusLost);
        runtime.poll_and_dispatch(&mut app, Duration::ZERO).unwrap();
        assert!(!runtime.focused);
        assert!(runtime.pending_repaint.is_none());

        runtime.dirty.set(true);
        runtime.draw_if_dirty(&mut app).unwrap();
        assert!(
            runtime.pending_repaint.is_none(),
            "ordinary invalidations may draw while blurred but must not restart animation"
        );

        runtime
            .screen
            .terminal
            .events
            .push_back(CrosstermEvent::FocusGained);
        runtime.poll_and_dispatch(&mut app, Duration::ZERO).unwrap();
        assert!(runtime.focused);
        assert!(runtime.dirty.get());
        runtime.draw_if_dirty(&mut app).unwrap();
        assert!(runtime.pending_repaint.is_some());
    });
}

#[test]
fn invalidation_driver_does_not_schedule_repaints_while_unfocused() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| RepaintingView));
        let screen = Rc::new(RefCell::new(TuiScreen::new(
            window_id,
            root,
            TestTerminal::new(TuiSize::new(20, 3)),
        )));
        let timer = Rc::new(RefCell::new(None));
        let focused = Rc::new(Cell::new(true));
        let freeze_repaints_when_unfocused = Rc::new(Cell::new(true));
        let failed = Rc::new(Cell::new(false));

        app.update(|ctx| {
            draw_and_schedule_repaint(
                &screen,
                &timer,
                &focused,
                &freeze_repaints_when_unfocused,
                &failed,
                ctx,
            )
        })
        .unwrap();
        assert!(timer.borrow().is_some());

        focused.set(false);
        app.update(|ctx| {
            draw_and_schedule_repaint(
                &screen,
                &timer,
                &focused,
                &freeze_repaints_when_unfocused,
                &failed,
                ctx,
            )
        })
        .unwrap();
        assert!(timer.borrow().is_none());

        freeze_repaints_when_unfocused.set(false);
        app.update(|ctx| {
            draw_and_schedule_repaint(
                &screen,
                &timer,
                &focused,
                &freeze_repaints_when_unfocused,
                &failed,
                ctx,
            )
        })
        .unwrap();
        assert!(
            timer.borrow().is_some(),
            "disabling the opt-in should resume repaint scheduling while unfocused"
        );
    });
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
struct PresentedFocusRoot {
    visible: crate::ViewHandle<TextView>,
}

impl Entity for PresentedFocusRoot {
    type Event = ();
}

impl TuiView for PresentedFocusRoot {
    fn ui_name() -> &'static str {
        "PresentedFocusRoot"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<crate::EntityId> {
        vec![self.visible.id()]
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        TuiChildView::new(&self.visible).finish()
    }
}

impl TypedActionView for PresentedFocusRoot {
    type Action = ();
}

#[test]
fn draw_preserves_focus_outside_the_presented_tree_by_default() {
    App::test((), |mut app| async move {
        let (window_id, _) = app.update(|ctx| ctx.add_tui_window(window_options(), |_| TextView));
        let visible = app.update(|ctx| ctx.add_tui_view(window_id, |_| TextView));
        let hidden = app.update(|ctx| ctx.add_tui_view(window_id, |_| TextView));
        let visible_for_root = visible.clone();
        let root = app.update(|ctx| {
            ctx.add_tui_view(window_id, move |_| PresentedFocusRoot {
                visible: visible_for_root,
            })
        });
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut screen = TuiScreen::new(window_id, root, terminal);

        visible.update(&mut app, |_, ctx| ctx.focus_self());
        app.update(|ctx| screen.draw(ctx)).unwrap();
        hidden.update(&mut app, |_, ctx| ctx.focus_self());
        app.update(|ctx| screen.draw(ctx)).unwrap();

        assert!(app.read(|ctx| hidden.is_focused(ctx)));
    });
}

#[test]
fn opt_in_draw_repairs_focus_owned_by_a_view_outside_the_presented_tree() {
    App::test((), |mut app| async move {
        let (window_id, _) = app.update(|ctx| ctx.add_tui_window(window_options(), |_| TextView));
        let visible = app.update(|ctx| ctx.add_tui_view(window_id, |_| TextView));
        let hidden = app.update(|ctx| ctx.add_tui_view(window_id, |_| TextView));
        let visible_for_root = visible.clone();
        let root = app.update(|ctx| {
            ctx.add_tui_view(window_id, move |_| PresentedFocusRoot {
                visible: visible_for_root,
            })
        });
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut screen = TuiScreen::new(window_id, root.clone(), terminal)
            .with_focus_policy(TuiFocusPolicy::PresentedTree);

        visible.update(&mut app, |_, ctx| ctx.focus_self());
        app.update(|ctx| screen.draw(ctx)).unwrap();
        assert!(screen.presenter.presented_views.contains(&root.id()));
        assert!(screen.presenter.presented_views.contains(&visible.id()));
        assert!(!screen.presenter.presented_views.contains(&hidden.id()));

        hidden.update(&mut app, |_, ctx| ctx.focus_self());
        assert!(app.read(|ctx| hidden.is_focused(ctx)));
        app.update(|ctx| screen.draw(ctx)).unwrap();
        assert!(app.read(|ctx| root.is_focused(ctx)));
    });
}

struct RepaintingElement {
    size: Option<TuiSize>,
}

impl TuiElement for RepaintingElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let size = constraint.clamp(TuiSize::new(1, 1));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        if let Some(cell) = surface.cell_mut(origin) {
            cell.set_char('*');
        }
        ctx.repaint_after(Duration::from_secs(1));
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }
}

struct RepaintingView;

impl Entity for RepaintingView {
    type Event = ();
}

impl TuiView for RepaintingView {
    fn ui_name() -> &'static str {
        "RepaintingView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn TuiElement> {
        Box::new(RepaintingElement { size: None })
    }
}

impl TypedActionView for RepaintingView {
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
struct FailingWriter {
    attempts: Rc<Cell<usize>>,
}

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        self.attempts.set(self.attempts.get() + 1);
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "terminal disconnected",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.attempts.set(self.attempts.get() + 1);
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "terminal disconnected",
        ))
    }
}

struct FailingTerminal {
    size: TuiSize,
    writer: FailingWriter,
}

impl TuiTerminal for FailingTerminal {
    fn size(&self) -> io::Result<TuiSize> {
        Ok(self.size)
    }

    fn poll_event(&mut self, _timeout: Duration) -> io::Result<Option<CrosstermEvent>> {
        Ok(None)
    }

    fn writer(&mut self) -> &mut dyn Write {
        &mut self.writer
    }
}

fn window_options() -> AddWindowOptions {
    AddWindowOptions {
        window_style: WindowStyle::NotStealFocus,
        ..Default::default()
    }
}

#[test]
fn terminal_disconnects_include_pipe_and_tty_errors() {
    assert!(is_terminal_disconnect(&io::Error::new(
        io::ErrorKind::BrokenPipe,
        "closed pipe"
    )));

    #[cfg(unix)]
    {
        assert!(is_terminal_disconnect(&io::Error::from_raw_os_error(
            libc::EIO
        )));
        assert!(is_terminal_disconnect(&io::Error::from_raw_os_error(
            libc::ENXIO
        )));
    }

    #[cfg(windows)]
    assert!(is_terminal_disconnect(&io::Error::from_raw_os_error(233)));

    assert!(!is_terminal_disconnect(&io::Error::other(
        "unexpected failure"
    )));
}
#[test]
fn startup_errors_classify_terminal_disconnects() {
    let disconnect = TuiDriverStartupError::from(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "terminal disconnected",
    ));
    assert!(matches!(
        disconnect,
        TuiDriverStartupError::TerminalDisconnected(_)
    ));

    let unexpected = TuiDriverStartupError::from(io::Error::other("unexpected failure"));
    assert!(matches!(unexpected, TuiDriverStartupError::Unexpected(_)));
}

#[test]
fn disconnected_driver_cancels_repaints_and_stops_drawing() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| TextView));
        let attempts = Rc::new(Cell::new(0));
        let screen = Rc::new(RefCell::new(TuiScreen::new(
            window_id,
            root,
            FailingTerminal {
                size: TuiSize::new(20, 3),
                writer: FailingWriter {
                    attempts: attempts.clone(),
                },
            },
        )));
        let timer = Rc::new(RefCell::new(Some(app.update(|ctx| {
            ctx.foreground_executor().spawn(async {
                std::future::pending::<()>().await;
            })
        }))));
        let focused = Rc::new(Cell::new(true));
        let freeze_repaints_when_unfocused = Rc::new(Cell::new(false));
        let failed = Rc::new(Cell::new(false));

        let error = app
            .update(|ctx| {
                draw_and_schedule_repaint(
                    &screen,
                    &timer,
                    &focused,
                    &freeze_repaints_when_unfocused,
                    &failed,
                    ctx,
                )
            })
            .unwrap_err();
        app.update(|ctx| {
            fail_tui_driver(error, TuiDriverIoOperation::DrawFrame, &failed, &timer, ctx);
        });

        assert!(failed.get());
        assert!(timer.borrow().is_none());
        assert_eq!(attempts.get(), 1);

        app.update(|ctx| {
            draw_and_schedule_repaint(
                &screen,
                &timer,
                &focused,
                &freeze_repaints_when_unfocused,
                &failed,
                ctx,
            )
        })
        .unwrap();

        assert_eq!(attempts.get(), 1);
        assert!(app.termination_result().is_none());
    });
}

#[test]
fn unexpected_driver_failure_terminates_with_one_error() {
    App::test((), |mut app| async move {
        let timer: Rc<RefCell<Option<ForegroundTask>>> = Rc::default();
        let failed = Rc::new(Cell::new(false));

        app.update(|ctx| {
            fail_tui_driver(
                io::Error::other("first failure"),
                TuiDriverIoOperation::ReadEvent,
                &failed,
                &timer,
                ctx,
            );
            fail_tui_driver(
                io::Error::other("second failure"),
                TuiDriverIoOperation::DrawFrame,
                &failed,
                &timer,
                ctx,
            );
        });

        let error = app
            .termination_result()
            .expect("unexpected I/O should set a termination result")
            .expect_err("unexpected I/O should terminate with an error");
        assert_eq!(
            error.to_string(),
            "failed to read a terminal event",
            "the first failure should win"
        );
    });
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

#[test]
fn repeats_dispatch_keymaps_while_modifier_events_bypass_them() {
    App::test((), |mut app| async move {
        let (window_id, root) = app.update(|ctx| {
            ctx.register_fixed_bindings([FixedBinding::new("ctrl-c", Bump, id!("BumpParentView"))]);
            ctx.add_tui_window(window_options(), |view_ctx| {
                let child = view_ctx.add_tui_view(|_| BumpChildView);
                BumpParentView { child, bumps: 0 }
            })
        });
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut screen = TuiScreen::new(window_id, root.clone(), terminal);
        app.update(|ctx| screen.draw(ctx)).unwrap();

        let modifier = screen
            .convert_event(CrosstermEvent::Key(KeyEvent::new(
                KeyCode::Modifier(ModifierKeyCode::LeftControl),
                KeyModifiers::CONTROL,
            )))
            .expect("modifier event");
        assert!(!app.update(|ctx| screen.dispatch_event(ctx, &modifier)));
        assert_eq!(root.read(&app, |view, _| view.bumps), 0);

        let repeat = screen
            .convert_event(CrosstermEvent::Key(KeyEvent::new_with_kind(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
                KeyEventKind::Repeat,
            )))
            .expect("repeat event");
        assert!(app.update(|ctx| screen.dispatch_event(ctx, &repeat)));
        assert_eq!(root.read(&app, |view, _| view.bumps), 1);
    });
}

#[test]
fn shift_lifecycle_restores_shift_and_normalizes_symbol_keystrokes() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| TextView));
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut screen = TuiScreen::new(window_id, root, terminal);

        screen
            .convert_event(CrosstermEvent::Key(KeyEvent::new_with_kind(
                KeyCode::Modifier(ModifierKeyCode::LeftShift),
                KeyModifiers::SHIFT,
                KeyEventKind::Press,
            )))
            .expect("shift press");

        // A letter keeps Shift because lowercasing recovers its base key; a
        // symbol is spelled as the produced character with no Shift, since its
        // base key cannot be derived from it.
        for (char, expected_shift, expected_base) in [('A', true, Some("a")), ('!', false, None)] {
            let Some(TuiEvent::KeyDown {
                keystroke,
                chars,
                details,
                ..
            }) = screen.convert_event(CrosstermEvent::Key(KeyEvent::new(
                KeyCode::Char(char),
                KeyModifiers::CONTROL,
            )))
            else {
                panic!("expected KeyDown");
            };
            assert!(keystroke.ctrl);
            assert_eq!(keystroke.shift, expected_shift);
            assert_eq!(keystroke.key, char.to_string());
            assert_eq!(chars, char.to_string());
            assert_eq!(details.key_without_modifiers.as_deref(), expected_base);
        }

        screen
            .convert_event(CrosstermEvent::Key(KeyEvent::new_with_kind(
                KeyCode::Modifier(ModifierKeyCode::LeftShift),
                KeyModifiers::SHIFT,
                KeyEventKind::Release,
            )))
            .expect("shift release");

        let Some(TuiEvent::KeyDown { keystroke, .. }) = screen.convert_event(CrosstermEvent::Key(
            KeyEvent::new(KeyCode::Char('!'), KeyModifiers::CONTROL),
        )) else {
            panic!("expected KeyDown");
        };
        assert!(keystroke.ctrl);
        assert!(!keystroke.shift);
        assert_eq!(keystroke.key, "!");
    });
}

#[test]
fn shift_remains_active_until_both_shift_keys_are_released() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| TextView));
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut screen = TuiScreen::new(window_id, root, terminal);

        for modifier in [ModifierKeyCode::LeftShift, ModifierKeyCode::RightShift] {
            screen.convert_event(CrosstermEvent::Key(KeyEvent::new_with_kind(
                KeyCode::Modifier(modifier),
                KeyModifiers::SHIFT,
                KeyEventKind::Press,
            )));
        }
        screen.convert_event(CrosstermEvent::Key(KeyEvent::new_with_kind(
            KeyCode::Modifier(ModifierKeyCode::LeftShift),
            KeyModifiers::SHIFT,
            KeyEventKind::Release,
        )));

        // A letter probes the tracked state directly, since a restored Shift
        // survives on letters but is normalized away on symbols.
        let Some(TuiEvent::KeyDown { keystroke, .. }) = screen.convert_event(CrosstermEvent::Key(
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::empty()),
        )) else {
            panic!("expected KeyDown");
        };
        assert!(keystroke.shift);

        screen.convert_event(CrosstermEvent::Key(KeyEvent::new_with_kind(
            KeyCode::Modifier(ModifierKeyCode::RightShift),
            KeyModifiers::SHIFT,
            KeyEventKind::Release,
        )));
        let Some(TuiEvent::KeyDown { keystroke, .. }) = screen.convert_event(CrosstermEvent::Key(
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::empty()),
        )) else {
            panic!("expected KeyDown");
        };
        assert!(!keystroke.shift);
    });
}

/// A dropped Shift release would otherwise latch Shift on forever, so every
/// event that reports Shift accurately re-syncs the tracked state.
#[test]
fn stale_shift_state_is_cleared_by_events_that_report_shift_accurately() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), |_| TextView));
        let terminal = TestTerminal::new(TuiSize::new(20, 3));
        let mut screen = TuiScreen::new(window_id, root, terminal);

        let shift_press = || {
            CrosstermEvent::Key(KeyEvent::new_with_kind(
                KeyCode::Modifier(ModifierKeyCode::LeftShift),
                KeyModifiers::SHIFT,
                KeyEventKind::Press,
            ))
        };
        let letter =
            || CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::empty()));
        let mouse_move = |modifiers| {
            CrosstermEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 1,
                row: 1,
                modifiers,
            })
        };

        for clearing_event in [
            CrosstermEvent::FocusLost,
            CrosstermEvent::FocusGained,
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::empty())),
            mouse_move(KeyModifiers::empty()),
        ] {
            screen.convert_event(shift_press());
            screen.convert_event(clearing_event);
            let Some(TuiEvent::KeyDown { keystroke, .. }) = screen.convert_event(letter()) else {
                panic!("expected KeyDown");
            };
            assert!(!keystroke.shift);
        }

        // Shift reported on such an event instead confirms it is still held.
        screen.convert_event(shift_press());
        for confirming_event in [
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT)),
            mouse_move(KeyModifiers::SHIFT),
        ] {
            screen.convert_event(confirming_event);
            let Some(TuiEvent::KeyDown { keystroke, .. }) = screen.convert_event(letter()) else {
                panic!("expected KeyDown");
            };
            assert!(keystroke.shift);
        }
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
    enter_terminal_screen(&mut enter_output, true, true).unwrap();
    assert!(
        enter_output
            .windows(b"\x1b[?2004h".len())
            .any(|window| window == b"\x1b[?2004h"),
        "entering the TUI should enable bracketed paste"
    );

    let mut leave_output = Vec::new();
    leave_terminal_screen(&mut leave_output).unwrap();
    assert!(
        leave_output
            .windows(b"\x1b[?2004l".len())
            .any(|window| window == b"\x1b[?2004l"),
        "leaving the TUI should disable bracketed paste"
    );
}

#[test]
fn terminal_screen_lifecycle_toggles_focus_reporting() {
    let mut enter_output = Vec::new();
    enter_terminal_screen(&mut enter_output, true, true).unwrap();
    assert!(
        enter_output
            .windows(b"\x1b[?1004h".len())
            .any(|window| window == b"\x1b[?1004h"),
        "entering the TUI should enable focus reporting"
    );

    let mut leave_output = Vec::new();
    leave_terminal_screen(&mut leave_output).unwrap();
    assert!(
        leave_output
            .windows(b"\x1b[?1004l".len())
            .any(|window| window == b"\x1b[?1004l"),
        "leaving the TUI should disable focus reporting"
    );
}

/// Enhancement-capable terminals report standalone modifier event types while
/// preserving shifted text through Crossterm's alternate-key decoding (CSI
/// `>15u`), then restore the previous protocol on exit.
///
/// Crossterm hard-routes these commands to the unsupported legacy Windows
/// console API, so the ANSI sequences are only emitted off Windows. The
/// enter/leave calls must still succeed on every platform (the `.unwrap()`s
/// below), and the byte assertions are gated to non-Windows where the sequences
/// are actually written.
#[test]
fn terminal_screen_lifecycle_toggles_keyboard_enhancement() {
    let mut enter_output = Vec::new();
    enter_terminal_screen(&mut enter_output, true, true).unwrap();

    let mut leave_output = Vec::new();
    leave_terminal_screen(&mut leave_output).unwrap();

    #[cfg(not(windows))]
    {
        assert!(
            enter_output
                .windows(b"\x1b[>15u".len())
                .any(|window| window == b"\x1b[>15u"),
            "entering the TUI should request modifier lifecycle support"
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
fn terminal_screen_lifecycle_can_skip_all_key_reporting() {
    let mut enter_output = Vec::new();
    enter_terminal_screen(&mut enter_output, true, false).unwrap();

    #[cfg(not(windows))]
    {
        assert!(
            enter_output
                .windows(b"\x1b[>3u".len())
                .any(|window| window == b"\x1b[>3u"),
            "compatibility mode should retain safe keyboard enhancements"
        );
        assert!(
            !enter_output
                .windows(b"\x1b[>15u".len())
                .any(|window| window == b"\x1b[>15u"),
            "compatibility mode should not request all-key reporting"
        );
    }
}

#[test]
fn terminal_screen_lifecycle_reconfigures_modifier_reporting() {
    let mut output = Vec::new();
    set_terminal_keyboard_enhancement_flags(&mut output, false).unwrap();
    assert_eq!(output, b"\x1b[=3;1u");

    output.clear();
    set_terminal_keyboard_enhancement_flags(&mut output, true).unwrap();
    assert_eq!(output, b"\x1b[=15;1u");
}

#[test]
fn terminal_screen_lifecycle_uses_baseline_keyboard_enhancement_when_unconfirmed() {
    let mut enter_output = Vec::new();
    enter_terminal_screen(&mut enter_output, false, true).unwrap();

    #[cfg(not(windows))]
    {
        assert!(
            enter_output
                .windows(b"\x1b[>3u".len())
                .any(|window| window == b"\x1b[>3u"),
            "unconfirmed terminals should still receive safe baseline keyboard enhancements"
        );
        assert!(
            !enter_output
                .windows(b"\x1b[>15u".len())
                .any(|window| window == b"\x1b[>15u"),
            "unconfirmed terminals should not receive all-key reporting"
        );
    }

    let mut leave_output = Vec::new();
    leave_terminal_screen(&mut leave_output).unwrap();
    #[cfg(not(windows))]
    assert!(
        leave_output
            .windows(b"\x1b[<1u".len())
            .any(|window| window == b"\x1b[<1u"),
        "leaving should pop the baseline keyboard enhancement request"
    );
}

#[test]
fn keyboard_enhancement_probe_retries_a_negative_result_once() {
    let mut results = VecDeque::from([Ok(false), Ok(true)]);
    assert!(probe_keyboard_enhancement_support(|| {
        results.pop_front().expect("probe should run at most twice")
    }));
    assert!(results.is_empty());
}

#[test]
fn keyboard_enhancement_probe_does_not_retry_success_or_error() {
    let mut successful_results = VecDeque::from([Ok(true), Ok(false)]);
    assert!(probe_keyboard_enhancement_support(|| {
        successful_results
            .pop_front()
            .expect("successful probe should run once")
    }));
    assert_eq!(successful_results.len(), 1);

    let mut failed_results = VecDeque::from([Err(io::Error::other("probe failed")), Ok(true)]);
    assert!(!probe_keyboard_enhancement_support(|| {
        failed_results
            .pop_front()
            .expect("failed probe should run once")
    }));
    assert_eq!(failed_results.len(), 1);
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
