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
//! [`crossterm_event_to_warp_event`] and dispatching it — first through the
//! shared keymap (the focused view's responder chain, exactly like the GUI
//! window event path), then through the rendered element tree.
//!
//! The host terminal is abstracted behind [`TuiTerminal`] so the loop and the
//! frame renderer can be exercised headlessly against an in-memory writer
//! without a real tty. The concrete [`CrosstermTerminal`] is the production
//! implementation.

use std::cell::Cell;
use std::io::{self, stdout, Stdout, Write};
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode, KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

use crate::elements::tui::{TuiConstraint, TuiEventContext, TuiRect, TuiSize};
use crate::platform::TerminationMode;
use crate::presenter::tui::TuiPresenter;
use crate::r#async::block_on;
use crate::r#async::executor::ForegroundTask;
use crate::{App, AppContext, Event, TuiView, ViewHandle, WindowId};

mod event_conversion;
mod renderer;

pub use event_conversion::crossterm_event_to_warp_event;
pub use renderer::TuiFrameRenderer;

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

/// Drives a single [`TuiView`] window: redraws it when invalidated and routes
/// input events back through the shared core.
pub struct TuiRuntime<T, R = CrosstermTerminal>
where
    R: TuiTerminal,
{
    window_id: WindowId,
    root_view: ViewHandle<T>,
    presenter: TuiPresenter,
    renderer: TuiFrameRenderer,
    terminal: R,
    dirty: Rc<Cell<bool>>,
    last_size: Option<TuiSize>,
    /// Restores the terminal when the runtime is dropped, for the
    /// [`enter`](TuiRuntime::enter)/[`run_until`](TuiRuntime::run_until) path.
    /// `None` when the terminal mode is owned elsewhere (the headless driver
    /// keeps it in its [`TuiDriverHandle`]). Held only for its `Drop`.
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
            window_id,
            root_view,
            presenter: TuiPresenter::new(),
            renderer: TuiFrameRenderer::new(),
            terminal,
            dirty,
            last_size: None,
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
            self.poll_and_dispatch(app, Duration::from_millis(250))?;
        }
        Ok(())
    }

    /// The terminal this runtime draws to. Primarily useful for inspecting an
    /// in-memory terminal's captured output in tests.
    pub fn terminal(&self) -> &R {
        &self.terminal
    }

    /// Lays out and paints the root view through the presenter and flushes the
    /// frame diff to the terminal.
    ///
    /// This is the stepwise draw entry: it takes a `&mut AppContext` so it can
    /// be driven directly from a real (headless) app — e.g. from a foreground
    /// task that redraws after handling input — without an [`App`] wrapper.
    /// [`run_until`](Self::run_until) calls it via [`App::update`].
    pub fn draw(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        let size = self.terminal.size()?;
        let area = TuiRect::new(0, 0, size.width, size.height);

        // Drain this window's invalidations each draw. The runtime repaints the
        // full frame, but the manual + autotracking invalidation sets must still
        // be consumed so they don't accumulate forever (and so per-view caching
        // can use them later).
        let _invalidation = ctx.take_all_invalidations_for_window(self.window_id);
        let frame = self.presenter.present(ctx, &self.root_view, area);

        let mut writer = self.terminal.writer();
        self.renderer
            .draw(&mut writer, &frame.buffer, frame.cursor)?;
        self.last_size = Some(size);
        Ok(())
    }

    /// Routes a raw crossterm event into the shared core, returning whether the
    /// caller should redraw afterwards (a resize, or an event a view handled).
    ///
    /// Like [`draw`](Self::draw), this takes a `&mut AppContext` so a headless
    /// app can feed input events (read on a background thread) straight into the
    /// shared dispatch path.
    pub fn dispatch_crossterm_event(
        &mut self,
        ctx: &mut AppContext,
        event: CrosstermEvent,
    ) -> bool {
        match event {
            CrosstermEvent::Resize(_, _) => true,
            event => match crossterm_event_to_warp_event(event) {
                Some(warp_event) => self.dispatch_event(ctx, &warp_event),
                None => false,
            },
        }
    }

    fn draw_if_dirty(&mut self, app: &mut App) -> io::Result<()> {
        let size = self.terminal.size()?;
        if self.last_size != Some(size) {
            self.dirty.set(true);
        }
        if !self.dirty.replace(false) {
            return Ok(());
        }
        let this = &mut *self;
        app.update(|ctx| this.draw(ctx))
    }

    fn poll_and_dispatch(&mut self, app: &mut App, timeout: Duration) -> io::Result<()> {
        let Some(event) = self.terminal.poll_event(timeout)? else {
            return Ok(());
        };
        let this = &mut *self;
        let redraw = app.update(|ctx| this.dispatch_crossterm_event(ctx, event));
        if redraw {
            self.dirty.set(true);
        }
        Ok(())
    }

    fn dispatch_event(&mut self, ctx: &mut AppContext, event: &Event) -> bool {
        // Keymap pass (GUI parity): offer a keystroke to the focused view's
        // responder chain first, exactly like the GUI window event path.
        if let Event::KeyDown {
            keystroke,
            is_composing,
            ..
        } = event
        {
            let responder_chain = ctx.get_responder_chain(self.window_id);
            match ctx.dispatch_keystroke(self.window_id, &responder_chain, keystroke, *is_composing)
            {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => log::error!("error dispatching keystroke: {error}"),
            }
        }

        // Element-tree pass: walk the rendered tree, offering the event to
        // each element (child-view elements re-scope the action origin while
        // descending into their subtree).
        let size = self
            .last_size
            .or_else(|| self.terminal.size().ok())
            .unwrap_or_default();
        let area = TuiRect::new(0, 0, size.width, size.height);

        let root_view_id = self.root_view.id();
        let mut element = match ctx.render_tui_view(self.window_id, root_view_id) {
            Ok(element) => element,
            Err(error) => {
                log::error!("failed to render the TUI root view for event dispatch: {error}");
                return false;
            }
        };
        element.layout(TuiConstraint::tight(size));

        let mut event_ctx = TuiEventContext::default();
        event_ctx.set_origin_view(Some(root_view_id));
        let handled = element.dispatch_event(event, area, &mut event_ctx, ctx);

        for update in event_ctx.take_updates() {
            update(ctx);
        }
        for action in event_ctx.take_typed_actions() {
            // Dispatch through the shared responder chain (the origin view's
            // ancestors in the neutral view hierarchy), so an action raised
            // inside an embedded child view bubbles to ancestor handlers.
            ctx.dispatch_typed_action_for_view(
                self.window_id,
                action.origin_view_id,
                action.action.as_ref(),
            );
        }
        handled
    }
}

/// The production [`TuiTerminal`]: writes to the process stdout and reports the
/// terminal size. Raw mode + the alternate screen are managed separately by a
/// [`TuiTerminalGuard`], so the terminal-mode lifetime can be detached from the
/// writer (the headless driver keeps the guard in its [`TuiDriverHandle`] for
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
/// (so a headless app restores deterministically when its session is dropped,
/// regardless of when the async draw loop is torn down).
pub struct TuiTerminalGuard(RawModeGuard<CrosstermModeControl>);

impl TuiTerminalGuard {
    /// Enables raw mode and switches to the alternate screen, restoring both
    /// when the guard is dropped.
    pub fn enter() -> io::Result<Self> {
        Ok(Self(RawModeGuard::enter(CrosstermModeControl)?))
    }
}

/// Keeps a headless TUI session alive. Dropping it tears the session down:
/// it restores the terminal (via the guard) and ends the draw loop + input
/// reader. Store it for the lifetime of the app (e.g. in a singleton model) so
/// the session lives as long as the app does.
pub struct TuiDriverHandle {
    _task: ForegroundTask,
    _reader: thread::JoinHandle<()>,
    _guard: TuiTerminalGuard,
}

/// Starts a headless TUI session that draws `root_view` and feeds terminal
/// input back into the shared core.
///
/// This is the headless counterpart to [`TuiRuntime::run_until`]: rather than
/// owning the main thread with a blocking loop, it cooperates with a real app's
/// event loop. It enters the alternate screen, reads terminal input on a
/// background thread, and runs the draw + dispatch steps on the foreground
/// executor (redrawing after each handled event). `Ctrl-C` terminates the app.
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

    // Built without an invalidation subscription: the headless driver redraws
    // explicitly after each input event rather than via the dirty flag that
    // `run_until` uses.
    let mut runtime: TuiRuntime<T, CrosstermTerminal> = TuiRuntime {
        window_id,
        root_view,
        presenter: TuiPresenter::new(),
        renderer: TuiFrameRenderer::new(),
        terminal: CrosstermTerminal::new(),
        dirty: Rc::new(Cell::new(true)),
        last_size: None,
        _terminal_guard: None,
    };

    let weak_app = ctx.weak_app();
    let (sender, receiver) = async_channel::unbounded::<CrosstermEvent>();

    // Blocking terminal reads run off the main thread and are forwarded to the
    // foreground executor through the channel, so the main thread's event loop
    // is never blocked waiting for input.
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
                    log::error!("failed to read a terminal event: {error}");
                    break;
                }
            }
        })?;

    let task = ctx.foreground_executor().spawn(async move {
        // Paint the first frame.
        if let Some(mut app) = weak_app.upgrade() {
            app.update(|ctx| {
                if let Err(error) = runtime.draw(ctx) {
                    log::error!("failed to draw the initial TUI frame: {error}");
                }
            });
        }

        while let Ok(event) = receiver.recv().await {
            let Some(mut app) = weak_app.upgrade() else {
                break;
            };
            let quit = is_ctrl_c(&event);
            let runtime = &mut runtime;
            app.update(move |ctx| {
                if quit {
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                    return;
                }
                if runtime.dispatch_crossterm_event(ctx, event) {
                    if let Err(error) = runtime.draw(ctx) {
                        log::error!("failed to draw a TUI frame: {error}");
                    }
                }
            });
        }
    });

    Ok(TuiDriverHandle {
        _task: task,
        _reader: reader,
        _guard: guard,
    })
}

/// Whether a crossterm event is `Ctrl-C`, the headless session's quit chord
/// (raw mode delivers it as a key event rather than a `SIGINT`).
fn is_ctrl_c(event: &CrosstermEvent) -> bool {
    matches!(
        event,
        CrosstermEvent::Key(key)
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
    )
}

/// The alternate-screen + raw-mode operations a [`RawModeGuard`] toggles.
/// Behind a trait so the guard's enter/leave lifecycle can be exercised without
/// a real terminal.
trait TerminalModeControl {
    fn enter(&mut self) -> io::Result<()>;
    fn leave(&mut self);
}

struct CrosstermModeControl;

impl TerminalModeControl for CrosstermModeControl {
    fn enter(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let mut out = stdout();
        if let Err(error) = execute!(out, EnterAlternateScreen, EnableMouseCapture, Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(())
    }

    fn leave(&mut self) {
        let mut out = stdout();
        let _ = execute!(out, Show, DisableMouseCapture, LeaveAlternateScreen);
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
#[path = "mod_test.rs"]
mod tests;
