//! The TUI runtime, additive behind the `tui` feature: the alternate-screen
//! lifecycle and the draw + event loop that drives a [`TuiView`] through the
//! shared [`App`].
//!
//! Placement: the GUI has no in-core analog of this module — its runtime is
//! the platform event loop in the `warpui` crate — so the TUI runtime stands
//! alone as an additive top-level module rather than a backend submodule of an
//! existing one.
//!
//! [`TuiRuntime`] mirrors the GUI's invalidate→redraw flow. On
//! [`enter`](TuiRuntime::enter) it puts the host terminal into raw mode + the
//! alternate screen (restored on drop) and subscribes to the window's
//! invalidation signal; [`run_until`](TuiRuntime::run_until) then repeatedly
//! redraws when dirty and polls crossterm for input, converting each event with
//! [`crossterm_event_to_tui_event`] and dispatching it — first through the
//! shared keymap (the focused view's responder chain, exactly like the GUI
//! window event path), then through the rendered element tree.
//!
//! The host terminal is abstracted behind [`TuiTerminal`] so the loop and the
//! frame renderer can be exercised headlessly against an in-memory writer
//! without a real tty. The concrete [`CrosstermTerminal`] is the production
//! implementation.

use std::cell::{Cell, RefCell};
use std::io::{self, stdout, Stdout, Write};
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use instant::Instant;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event as CrosstermEvent,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

use crate::elements::tui::{TuiEvent, TuiEventContext, TuiPoint, TuiRect, TuiSize};
use crate::event::ModifiersState;
use crate::presenter::tui::TuiPresenter;
use crate::r#async::executor::ForegroundTask;
use crate::r#async::{block_on, Timer};
use crate::{App, AppContext, TuiView, ViewHandle, WindowId};

mod event_conversion;
mod renderer;
mod terminal_probe;

pub use event_conversion::crossterm_event_to_tui_event;
use event_conversion::ClickTracker;
pub use renderer::TuiFrameRenderer;
pub use terminal_probe::{
    probe_terminal_colors, BackgroundLuminance, ProbedRgb, ProbedTerminalColors,
};
use warp_errors::report_error;

/// The host terminal the runtime draws to and reads input from. Abstracted so
/// the draw + event loop is testable against an in-memory target.
pub trait TuiTerminal {
    /// The current terminal size in cells (each axis at least 1).
    fn size(&self) -> io::Result<TuiSize>;

    /// Blocks up to `timeout` for the next input event, returning `None` on
    /// timeout.
    fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<CrosstermEvent>>;

    /// The writer the renderer flushes frames to.
    fn writer(&mut self) -> &mut dyn Write;
}

/// The rendering half of the TUI: owns the presenter, renderer, and host
/// terminal for one window and paints that window's view tree. Kept separate
/// from input dispatch so the invalidation-driven redraw (which paints inside
/// `flush_effects`) never collides with a borrow the input path holds.
struct TuiScreen<T, R: TuiTerminal> {
    window_id: WindowId,
    root_view: ViewHandle<T>,
    presenter: TuiPresenter,
    renderer: TuiFrameRenderer,
    terminal: R,
    /// Synthesizes multi-click counts for left mouse presses, which crossterm
    /// does not report.
    click_tracker: ClickTracker,
    /// The pointer position from the most recent positional event, replayed as
    /// a synthetic `MouseMoved` after each draw so hover state tracks elements
    /// that move under a stationary pointer.
    last_mouse_position: Option<TuiPoint>,
}

impl<T: TuiView, R: TuiTerminal> TuiScreen<T, R> {
    fn new(window_id: WindowId, root_view: ViewHandle<T>, terminal: R) -> Self {
        Self {
            window_id,
            root_view,
            presenter: TuiPresenter::new(),
            renderer: TuiFrameRenderer::new(),
            terminal,
            click_tracker: ClickTracker::default(),
            last_mouse_position: None,
        }
    }

    fn size(&self) -> io::Result<TuiSize> {
        self.terminal.size()
    }

    /// Lays out and paints the root view through the presenter, then flushes the
    /// final frame to the terminal. Draining this window's invalidations keeps
    /// the manual + autotracking sets from accumulating (the frame is repainted
    /// in full regardless). After each presentation, the last pointer position
    /// is replayed as a synthetic `MouseMoved`; resulting invalidations rebuild
    /// the frame within this call, capped at three iterations like the GUI. Only
    /// the final reconciled frame is flushed, matching the GUI's presentation.
    ///
    /// Returns the final frame's earliest requested repaint deadline so the
    /// caller can schedule a timed redraw.
    fn draw(&mut self, ctx: &mut AppContext) -> io::Result<Option<Instant>> {
        let size = self.terminal.size()?;
        let area = TuiRect::new(0, 0, size.width, size.height);

        // Mirrors the GUI's `build_scene` loop: pointer replay can invalidate
        // hover-dependent layout, requiring another presentation and replay.
        // The first iteration always presents; later ones only run if the
        // replay invalidated something. Cap at three total presentations so a
        // hover/layout feedback loop cannot hang the redraw.
        let mut frame = None;
        for _ in 0..3 {
            let invalidation = ctx.take_all_invalidations_for_window(self.window_id);
            if frame.is_some() && invalidation.updated.is_empty() && !invalidation.redraw_requested
            {
                break;
            }
            self.presenter
                .invalidate(&invalidation, ctx, self.window_id);
            frame = Some(self.presenter.present(ctx, &self.root_view, area));
            self.replay_mouse_position(ctx);
        }
        let frame = frame.expect("loop always presents at least once");

        let mut writer = self.terminal.writer();
        self.renderer
            .draw(&mut writer, &frame.buffer, frame.cursor)?;
        Ok(frame.repaint_at)
    }

    /// Redispatches the last known pointer position as a synthetic
    /// `MouseMoved` through the freshly rendered tree, so hover state tracks
    /// elements that moved under a stationary pointer (e.g. a collapsible
    /// expanding and pushing its header out from under the mouse). A state
    /// change invalidates the notified views, which `draw`'s loop picks up to
    /// rebuild the frame within the same call.
    fn replay_mouse_position(&mut self, ctx: &mut AppContext) {
        let Some(position) = self.last_mouse_position else {
            return;
        };
        let event = TuiEvent::MouseMoved {
            position,
            modifiers: ModifiersState::default(),
            is_synthetic: true,
        };
        self.dispatch_event(ctx, &event);
    }

    /// Converts a raw crossterm event into the TUI vocabulary, annotating left
    /// mouse-down events with a synthesized multi-click count (crossterm only
    /// reports raw presses). Returns `None` for events with no TUI equivalent.
    fn convert_event(&mut self, event: CrosstermEvent) -> Option<TuiEvent> {
        let mut tui_event = crossterm_event_to_tui_event(event)?;
        self.click_tracker.annotate(&mut tui_event, Instant::now());
        Some(tui_event)
    }

    /// Dispatches a converted input event into the cached element tree, returning
    /// whether it was handled. Uses the last rendered element tree cached by the
    /// presenter (the same tree that was painted), with a `TuiLayoutContext` so
    /// `TuiChildView` can resolve its child from `rendered_views`.
    fn dispatch_event(&mut self, ctx: &mut AppContext, event: &TuiEvent) -> bool {
        if let Some(position) = event.position() {
            self.last_mouse_position = Some(position);
        }

        // Keymap pass (GUI parity): offer a keystroke to the focused view's
        // responder chain first, exactly like the GUI window event path.
        if let Some((keystroke, is_composing)) = event.key_down() {
            let responder_chain = ctx.get_responder_chain(self.window_id);
            match ctx.dispatch_keystroke(self.window_id, &responder_chain, keystroke, is_composing)
            {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => report_error!(error.context("error dispatching keystroke")),
            }
        }

        // Element-tree pass: walk the last rendered+laid-out element tree.
        // Access the two presenter fields directly so Rust sees disjoint borrows.
        let (Some(element), Some(scene)) = (
            self.presenter.last_element.as_mut(),
            self.presenter.last_scene.clone(),
        ) else {
            return false; // no draw has happened yet
        };
        let root_view_id = self.root_view.id();
        let mut event_ctx = TuiEventContext::new(scene, &mut self.presenter.rendered_views);
        event_ctx.set_origin_view(Some(root_view_id));
        let handled = element.dispatch_event(event, &mut event_ctx, ctx);

        let notified = event_ctx.take_notified();
        for view_id in notified {
            ctx.notify_view_observers(self.window_id, view_id);
        }

        for action in event_ctx.take_typed_actions() {
            // Dispatch through the shared responder chain (the origin view's
            // ancestors), so an action raised inside an embedded child view
            // bubbles to ancestor handlers.
            ctx.dispatch_typed_action_for_view(
                self.window_id,
                action.origin_view_id,
                action.action.as_ref(),
            );
        }
        handled
    }
}

/// A **development/test harness** that drives a single [`TuiView`] window with a
/// *blocking* loop ([`run_until`](Self::run_until)): it redraws when dirty and
/// polls the terminal for input. It backs the interactive `tui_*` examples and
/// the runtime unit tests; it is **not** used by the shipping app, which drives
/// the TUI with the non-blocking, invalidation-driven [`spawn_tui_driver`]
/// instead. It is intentionally not `#[cfg(test)]`-gated because the examples
/// (which compile outside `cfg(test)`) depend on it.
pub struct TuiRuntime<T, R = CrosstermTerminal>
where
    R: TuiTerminal,
{
    screen: TuiScreen<T, R>,
    dirty: Rc<Cell<bool>>,
    last_size: Option<TuiSize>,
    /// The earliest element-requested repaint deadline from the last draw; the
    /// loop marks itself dirty once it passes.
    pending_repaint: Option<Instant>,
    /// Restores the terminal when the runtime is dropped (the `enter` path).
    /// Held only for its `Drop`.
    _terminal_guard: Option<TuiTerminalGuard>,
}

impl<T> TuiRuntime<T, CrosstermTerminal>
where
    T: TuiView,
{
    /// Enters the alternate screen + raw mode and prepares to drive `root_view`.
    /// The terminal is restored when the returned runtime is dropped.
    pub fn enter(app: &App, window_id: WindowId, root_view: ViewHandle<T>) -> io::Result<Self> {
        let guard = TuiTerminalGuard::enter()?;
        let mut runtime = Self::with_terminal(app, window_id, root_view, CrosstermTerminal::new());
        runtime._terminal_guard = Some(guard);
        Ok(runtime)
    }
}

impl<T, R> TuiRuntime<T, R>
where
    T: TuiView,
    R: TuiTerminal,
{
    /// Builds a runtime over an arbitrary [`TuiTerminal`]. Subscribes to the
    /// window's invalidation signal so a `notify` schedules a redraw, and marks
    /// the runtime dirty so the first loop iteration paints.
    pub fn with_terminal(
        app: &App,
        window_id: WindowId,
        root_view: ViewHandle<T>,
        terminal: R,
    ) -> Self {
        let dirty = Rc::new(Cell::new(true));
        let dirty_for_callback = dirty.clone();
        app.on_window_invalidated(window_id, move |_, _| dirty_for_callback.set(true));
        Self {
            screen: TuiScreen::new(window_id, root_view, terminal),
            dirty,
            last_size: None,
            pending_repaint: None,
            _terminal_guard: None,
        }
    }

    /// Runs the draw + input loop until `should_quit` returns `true`, redrawing
    /// when invalidated (or resized) and dispatching converted input events.
    pub fn run_until(
        &mut self,
        app: &mut App,
        mut should_quit: impl FnMut(&App) -> bool,
    ) -> io::Result<()> {
        while !should_quit(app) {
            self.draw_if_dirty(app)?;
            // 250 ms is a standard event-poll heartbeat: short enough to feel
            // responsive to resize, long enough to avoid busy-waiting. A timeout
            // is not an error — `poll_event` returns `Ok(None)`, making the loop
            // iteration a no-op before the next draw-if-dirty check. A pending
            // element-requested repaint shortens the wait so the redraw lands
            // on time.
            let heartbeat = Duration::from_millis(250);
            let timeout = match self.pending_repaint {
                Some(deadline) => {
                    let now = Instant::now();
                    if deadline > now {
                        (deadline - now).min(heartbeat)
                    } else {
                        Duration::ZERO
                    }
                }
                None => heartbeat,
            };
            self.poll_and_dispatch(app, timeout)?;
        }
        Ok(())
    }

    /// The terminal this runtime draws to. Primarily useful for inspecting an
    /// in-memory terminal's captured output in tests.
    pub fn terminal(&self) -> &R {
        &self.screen.terminal
    }

    fn draw_if_dirty(&mut self, app: &mut App) -> io::Result<()> {
        let size = self.screen.size()?;
        if self.last_size != Some(size) {
            self.dirty.set(true);
        }
        if self
            .pending_repaint
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            self.pending_repaint = None;
            self.dirty.set(true);
        }
        if !self.dirty.replace(false) {
            return Ok(());
        }
        let screen = &mut self.screen;
        self.pending_repaint = app.update(|ctx| screen.draw(ctx))?;
        self.last_size = Some(size);
        Ok(())
    }

    fn poll_and_dispatch(&mut self, app: &mut App, timeout: Duration) -> io::Result<()> {
        let Some(event) = self.screen.terminal.poll_event(timeout)? else {
            return Ok(());
        };

        match event {
            CrosstermEvent::Resize(_, _) => self.dirty.set(true),
            event => {
                let screen = &mut self.screen;
                if let Some(tui_event) = screen.convert_event(event) {
                    let handled = app.update(|ctx| screen.dispatch_event(ctx, &tui_event));
                    if handled {
                        self.dirty.set(true);
                    }
                }
            }
        }
        Ok(())
    }
}

/// The production [`TuiTerminal`]: writes to the process stdout and reports the
/// terminal size. Raw mode + the alternate screen are managed separately by a
/// [`TuiTerminalGuard`], so the terminal-mode lifetime can be detached from the
/// writer (the headless driver keeps the guard in its [`TuiDriverHandle`] for a
/// deterministic restore, independent of when the async draw loop is dropped).
pub struct CrosstermTerminal {
    stdout: Stdout,
}

impl CrosstermTerminal {
    /// Builds a terminal over the process stdout. Does not change terminal
    /// modes; pair it with a [`TuiTerminalGuard`] to enter raw mode + the
    /// alternate screen.
    pub fn new() -> Self {
        Self { stdout: stdout() }
    }
}

impl Default for CrosstermTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiTerminal for CrosstermTerminal {
    fn size(&self) -> io::Result<TuiSize> {
        let (width, height) = terminal::size()?;
        Ok(TuiSize::new(width.max(1), height.max(1)))
    }

    fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<CrosstermEvent>> {
        if event::poll(timeout)? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }

    fn writer(&mut self) -> &mut dyn Write {
        &mut self.stdout
    }
}

/// Owns the terminal's raw mode + alternate screen for as long as it is alive,
/// restoring the terminal on drop. Held by [`TuiRuntime::enter`] (so the
/// `run_until` path restores when the runtime drops) or by a [`TuiDriverHandle`]
/// (so a headless app restores deterministically when its session is dropped).
pub struct TuiTerminalGuard(RawModeGuard<CrosstermModeControl>);

impl TuiTerminalGuard {
    /// Enables raw mode and switches to the alternate screen, restoring both
    /// when the guard is dropped.
    pub fn enter() -> io::Result<Self> {
        Ok(Self(RawModeGuard::enter(CrosstermModeControl)?))
    }
}

/// Keeps a headless TUI session alive. Store it for the lifetime of the app
/// (e.g. in a singleton model) so the session lives as long as the app does;
/// dropping it tears the session down. Fields drop in declaration order, which
/// is also the teardown order:
/// - `_task`: the input-dispatch loop. It is an [`async_task::Task`], so
///   dropping it *cancels* the future (we intentionally don't `detach()`),
///   which in turn drops the channel receiver feeding it.
/// - `_reader`: the blocking input-reader thread. Dropping a `JoinHandle`
///   detaches rather than joins, so this doesn't stop the thread directly; the
///   thread exits on its own once the receiver above is gone (its next `send`
///   fails) or when the process exits. The handle is held so the session owns
///   the thread it spawned.
/// - `_guard`: restores raw mode + the alternate screen on drop.
pub struct TuiDriverHandle {
    _task: ForegroundTask,
    /// The pending element-requested repaint timer, if any (see
    /// [`draw_and_schedule_repaint`]). Dropping it cancels the timer.
    _repaint_timer: Rc<RefCell<Option<ForegroundTask>>>,
    _reader: thread::JoinHandle<()>,
    _guard: TuiTerminalGuard,
}

/// Starts a headless TUI session that draws `root_view` and feeds terminal input
/// back into the shared core.
///
/// This is the headless counterpart to [`TuiRuntime::run_until`]: instead of
/// owning the main thread with a blocking loop, it cooperates with a real app's
/// event loop. Rendering is **invalidation-driven**: an `on_window_invalidated`
/// callback repaints the window, so any `notify()` (an input handler, a model or
/// async update, or the resize handling below) schedules a redraw via the core's
/// normal `flush_effects` pass. Input is read on a background thread and only
/// *dispatched* on the foreground executor. Every event — including `Ctrl-C`,
/// which raw mode delivers as a key event rather than a `SIGINT` — flows
/// through the keymap + element-tree dispatch, so quitting is owned by the
/// app's views (e.g. a double-`Ctrl-C` exit handler), not the driver.
///
/// The returned [`TuiDriverHandle`] owns the session: keep it alive for as long
/// as the session should run, and drop it (e.g. on app teardown) to restore the
/// terminal.
pub fn spawn_tui_driver<T: TuiView>(
    ctx: &mut AppContext,
    window_id: WindowId,
    root_view: ViewHandle<T>,
) -> io::Result<TuiDriverHandle> {
    let guard = TuiTerminalGuard::enter()?;

    // The presenter + renderer + terminal live behind an `Rc<RefCell<_>>` owned
    // by the invalidation callback. The input path never borrows it, so painting
    // inside `flush_effects` can't collide with dispatch.
    let screen = Rc::new(RefCell::new(TuiScreen::new(
        window_id,
        root_view,
        CrosstermTerminal::new(),
    )));

    // Repaint scheduling: at most one pending timer, held in this slot. Every
    // draw reports the earliest element-requested repaint deadline for the
    // whole frame, so each draw replaces (cancelling) the previous timer with
    // one for its own deadline — or clears it when nothing is animating.
    let repaint_timer: Rc<RefCell<Option<ForegroundTask>>> = Rc::default();

    // Redraw whenever the window is invalidated. `update_windows` invokes this at
    // the end of every `flush_effects`, so any `notify()` repaints. (The callback
    // is removed from the registry while it runs, so a draw that itself
    // invalidates can't re-enter it.)
    {
        let screen = screen.clone();
        let repaint_timer = repaint_timer.clone();
        ctx.on_window_invalidated(window_id, move |_, ctx| {
            if let Err(error) = draw_and_schedule_repaint(&screen, &repaint_timer, ctx) {
                report_error!(anyhow::Error::new(error).context("failed to draw a TUI frame"));
            }
        });
    }

    // Paint the first frame now, which also consumes the window's initial
    // invalidation so the callback doesn't redundantly repaint it on the next
    // flush. This runs during setup (unlike the invalidation callback above,
    // which is in the event loop and can only log), so a failure is propagated:
    // returning `Err` here drops `guard` (restoring the terminal) and lets the
    // caller surface the error, rather than leaving a live raw-mode session with
    // no usable frame.
    draw_and_schedule_repaint(&screen, &repaint_timer, ctx)?;

    let weak_app = ctx.weak_app();
    let (sender, receiver) = async_channel::unbounded::<CrosstermEvent>();

    // Blocking terminal reads run off the main thread and are forwarded to the
    // foreground executor through the channel, so the main thread's event loop is
    // never blocked waiting for input.
    let reader = thread::Builder::new()
        .name("warp-tui-input".to_owned())
        .spawn(move || loop {
            match event::read() {
                Ok(event) => {
                    // The reader runs on a dedicated thread, so blocking on the
                    // send is fine; an error means the receiver was dropped.
                    if block_on(sender.send(event)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    report_error!("failed to read a terminal event", extra: { "error" => %error });
                    break;
                }
            }
        })?;

    let dispatch_screen = screen.clone();
    let task = ctx.foreground_executor().spawn(async move {
        while let Ok(event) = receiver.recv().await {
            let Some(mut app) = weak_app.upgrade() else {
                break;
            };
            let screen = dispatch_screen.clone();
            // Dispatch reuses the shared screen's cached element tree (so embedded
            // child views resolve their elements). Edits queue effects that flush
            // when this `update` returns — firing the invalidation callback to
            // repaint — so the screen is never borrowed re-entrantly.
            app.update(move |ctx| match event {
                CrosstermEvent::Resize(_, _) => ctx.invalidate_all_views(),
                event => {
                    let mut screen = screen.borrow_mut();
                    if let Some(tui_event) = screen.convert_event(event) {
                        screen.dispatch_event(ctx, &tui_event);
                    }
                }
            });
        }
    });

    Ok(TuiDriverHandle {
        _task: task,
        _repaint_timer: repaint_timer,
        _reader: reader,
        _guard: guard,
    })
}

/// Draws a frame and schedules a timer for its element-requested repaint
/// deadline, if any.
///
/// Paint traverses the full tree, so each frame's reported deadline is the
/// authoritative next repaint: the new timer replaces — and thereby cancels —
/// any previously pending one, and a frame with no deadline clears the slot.
/// The timer redraws through this same function, so the cycle is
/// self-sustaining while elements animate and fully idle otherwise.
fn draw_and_schedule_repaint<T: TuiView, R: TuiTerminal + 'static>(
    screen: &Rc<RefCell<TuiScreen<T, R>>>,
    timer_slot: &Rc<RefCell<Option<ForegroundTask>>>,
    ctx: &mut AppContext,
) -> io::Result<()> {
    let deadline = screen.borrow_mut().draw(ctx)?;
    let timer = deadline.map(|deadline| {
        let screen = screen.clone();
        // Weak, or the slot (held by the task) and the task (held by the slot)
        // would keep each other alive.
        let weak_slot = Rc::downgrade(timer_slot);
        let weak_app = ctx.weak_app();
        ctx.foreground_executor().spawn(async move {
            let now = Instant::now();
            if deadline > now {
                Timer::after(deadline - now).await;
            }
            let (Some(mut app), Some(timer_slot)) = (weak_app.upgrade(), weak_slot.upgrade())
            else {
                return;
            };
            app.update(move |ctx| {
                // The draw below replaces the slot, dropping this task's own
                // handle; `async_task` defers destruction, so this in-flight
                // poll completes normally.
                if let Err(error) = draw_and_schedule_repaint(&screen, &timer_slot, ctx) {
                    report_error!("failed to draw a TUI frame", extra: { "error" => %error });
                }
            });
        })
    });
    *timer_slot.borrow_mut() = timer;
    Ok(())
}

/// The alternate-screen + raw-mode operations a [`RawModeGuard`] toggles.
/// Behind a trait so the guard's enter/leave lifecycle can be exercised without
/// a real terminal.
trait TerminalModeControl {
    fn enter(&mut self) -> io::Result<()>;
    fn leave(&mut self);
}

struct CrosstermModeControl;
fn enter_terminal_screen(out: &mut impl Write) -> io::Result<()> {
    execute!(
        out,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        Hide
    )
}

fn leave_terminal_screen(out: &mut impl Write) -> io::Result<()> {
    execute!(
        out,
        Show,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )
}

impl TerminalModeControl for CrosstermModeControl {
    fn enter(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let mut out = stdout();
        if let Err(error) = enter_terminal_screen(&mut out) {
            let _ = leave_terminal_screen(&mut out);
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(())
    }

    fn leave(&mut self) {
        let mut out = stdout();
        let _ = leave_terminal_screen(&mut out);
        let _ = terminal::disable_raw_mode();
    }
}

/// Restores the host terminal on drop, so a panic or early return never strands
/// it in the alternate screen or raw mode.
struct RawModeGuard<C: TerminalModeControl> {
    control: C,
}

impl<C: TerminalModeControl> RawModeGuard<C> {
    fn enter(mut control: C) -> io::Result<Self> {
        control.enter()?;
        Ok(Self { control })
    }
}

impl<C: TerminalModeControl> Drop for RawModeGuard<C> {
    fn drop(&mut self) {
        self.control.leave();
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
