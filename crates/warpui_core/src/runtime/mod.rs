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

use std::cell::{Cell, RefCell};
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

use crate::elements::tui::{TuiConstraint, TuiEventContext, TuiLayoutContext, TuiRect, TuiSize};
use crate::platform::TerminationMode;
use crate::presenter::tui::TuiPresenter;
use crate::r#async::block_on;
use crate::r#async::executor::ForegroundTask;
use crate::{App, AppContext, EntityId, Event, TuiView, ViewHandle, WindowId};

use std::collections::HashMap;

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
}

impl<T: TuiView, R: TuiTerminal> TuiScreen<T, R> {
    fn new(window_id: WindowId, root_view: ViewHandle<T>, terminal: R) -> Self {
        Self {
            window_id,
            root_view,
            presenter: TuiPresenter::new(),
            renderer: TuiFrameRenderer::new(),
            terminal,
        }
    }

    fn size(&self) -> io::Result<TuiSize> {
        self.terminal.size()
    }

    fn root_view_id(&self) -> EntityId {
        self.root_view.id()
    }

    /// Lays out and paints the root view through the presenter, then flushes the
    /// frame diff to the terminal. Draining this window's invalidations keeps
    /// the manual + autotracking sets from accumulating (the frame is repainted
    /// in full regardless).
    fn draw(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        let size = self.terminal.size()?;
        let area = TuiRect::new(0, 0, size.width, size.height);
        let invalidation = ctx.take_all_invalidations_for_window(self.window_id);
        self.presenter.invalidate(&invalidation, ctx, self.window_id);
        let frame = self.presenter.present(ctx, &self.root_view, area);
        let mut writer = self.terminal.writer();
        self.renderer.draw(&mut writer, &frame.buffer, frame.cursor)
    }

    /// Dispatches a converted input event into the element tree, returning
    /// whether it was handled. Uses the last rendered element tree cached by
    /// the presenter (same tree that was painted), with a `TuiLayoutContext`
    /// so `TuiChildView` can look up its child from `rendered_views`.
    fn dispatch_event(&mut self, ctx: &mut AppContext, event: &Event) -> bool {
        if let Event::KeyDown {
            keystroke,
            is_composing,
            ..
        } = event
        {
            let responder_chain = ctx.get_responder_chain(self.window_id);
            match ctx.dispatch_keystroke(
                self.window_id,
                &responder_chain,
                keystroke,
                *is_composing,
            ) {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => log::error!("error dispatching keystroke: {error}"),
            }
        }

        let Some(element) = self.presenter.last_element.as_mut() else {
            return false;
        };
        let size = self.terminal.size().unwrap_or_default();
        let area = TuiRect::new(0, 0, size.width, size.height);
        let root_view_id = self.root_view.id();
        let mut event_ctx = TuiEventContext::default();
        event_ctx.set_origin_view(Some(root_view_id));
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut self.presenter.rendered_views,
        };
        let handled =
            element.dispatch_event(event, area, &mut event_ctx, &mut layout_ctx, ctx);

        for update in event_ctx.take_updates() {
            update(ctx);
        }
        for action in event_ctx.take_typed_actions() {
            ctx.dispatch_typed_action_for_view(
                self.window_id,
                action.origin_view_id,
                action.action.as_ref(),
            );
        }
        handled
    }
}

/// Dispatches a converted input [`Event`] for the headless driver path, where
/// the [`TuiScreen`]'s presenter is not directly accessible. Runs the keymap
/// pass first; for the element-tree pass it falls back to a fresh render since
/// the cached `last_element` lives inside `TuiScreen`. The blocking
/// [`TuiRuntime`] path uses [`TuiScreen::dispatch_event`] instead.
fn dispatch_event(
    ctx: &mut AppContext,
    window_id: WindowId,
    root_view_id: EntityId,
    size: TuiSize,
    event: &Event,
) -> bool {
    if let Event::KeyDown {
        keystroke,
        is_composing,
        ..
    } = event
    {
        let responder_chain = ctx.get_responder_chain(window_id);
        match ctx.dispatch_keystroke(window_id, &responder_chain, keystroke, *is_composing) {
            Ok(true) => return true,
            Ok(false) => {}
            Err(error) => log::error!("error dispatching keystroke: {error}"),
        }
    }

    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut element = match ctx.render_tui_view(window_id, root_view_id) {
        Ok(element) => element,
        Err(error) => {
            log::error!("failed to render the TUI root view for event dispatch: {error}");
            return false;
        }
    };
    let mut rendered_views = HashMap::new();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.layout(TuiConstraint::loose(size), &mut layout_ctx);

    let mut event_ctx = TuiEventContext::default();
    event_ctx.set_origin_view(Some(root_view_id));
    let handled = element.dispatch_event(event, area, &mut event_ctx, &mut layout_ctx, ctx);

    for update in event_ctx.take_updates() {
        update(ctx);
    }
    for action in event_ctx.take_typed_actions() {
        ctx.dispatch_typed_action_for_view(
            window_id,
            action.origin_view_id,
            action.action.as_ref(),
        );
    }
    handled
}

/// Drives a single [`TuiView`] window with a blocking loop: it redraws when
/// dirty and polls the terminal for input. Used by the example and tests via
/// [`run_until`](Self::run_until). A real app uses [`spawn_tui_driver`] instead,
/// which is invalidation-driven and cooperates with the app's own event loop.
pub struct TuiRuntime<T, R = CrosstermTerminal>
where
    R: TuiTerminal,
{
    screen: TuiScreen<T, R>,
    dirty: Rc<Cell<bool>>,
    last_size: Option<TuiSize>,
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
            // iteration a no-op before the next draw-if-dirty check.
            self.poll_and_dispatch(app, Duration::from_millis(250))?;
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
        if !self.dirty.replace(false) {
            return Ok(());
        }
        let screen = &mut self.screen;
        app.update(|ctx| screen.draw(ctx))?;
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
                if let Some(warp_event) = crossterm_event_to_warp_event(event) {
                    let screen = &mut self.screen;
                    let handled = app.update(|ctx| screen.dispatch_event(ctx, &warp_event));
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
/// This is the headless counterpart to [`TuiRuntime::run_until`]: instead of
/// owning the main thread with a blocking loop, it cooperates with a real app's
/// event loop. Rendering is **invalidation-driven**: a `on_window_invalidated`
/// callback repaints the window, so any `notify()` (an input handler, a model or
/// async update, or the resize handling below) schedules a redraw via the core's
/// normal `flush_effects` pass — the frame reacts to state changes rather than
/// being sequenced after input. Input is read on a background thread and only
/// *dispatched* on the foreground executor; `Ctrl-C` terminates the app.
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
    let root_view_id = root_view.id();

    // The renderer + terminal live behind an `Rc<RefCell<_>>` owned by the
    // invalidation callback. The input path never borrows it, so painting inside
    // `flush_effects` can't collide with dispatch.
    let screen = Rc::new(RefCell::new(TuiScreen::new(
        window_id,
        root_view,
        CrosstermTerminal::new(),
    )));

    // Redraw whenever the window is invalidated. `update_windows` invokes this at
    // the end of every `flush_effects`, so any `notify()` repaints. (The
    // callback is removed from the registry while it runs, so a draw that itself
    // invalidates can't re-enter it.)
    {
        let screen = screen.clone();
        ctx.on_window_invalidated(window_id, move |_, ctx| {
            if let Err(error) = screen.borrow_mut().draw(ctx) {
                log::error!("failed to draw a TUI frame: {error}");
            }
        });
    }

    // Paint the first frame now, which also consumes the window's initial
    // invalidation so the callback doesn't redundantly repaint it on the next
    // flush.
    if let Err(error) = screen.borrow_mut().draw(ctx) {
        log::error!("failed to draw the initial TUI frame: {error}");
    }

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
        while let Ok(event) = receiver.recv().await {
            let Some(mut app) = weak_app.upgrade() else {
                break;
            };
            let quit = is_ctrl_c(&event);
            // Only dispatch here; the redraw happens reactively in the
            // invalidation callback when dispatch (or a resize) invalidates the
            // window.
            app.update(move |ctx| {
                if quit {
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                    return;
                }
                handle_input_event(ctx, window_id, root_view_id, event);
            });
        }
    });

    Ok(TuiDriverHandle {
        _task: task,
        _reader: reader,
        _guard: guard,
    })
}

/// Routes one raw terminal event into the shared core. A resize invalidates the
/// window so the next flush repaints at the new size; other events dispatch and,
/// if handled, invalidate the window so a state change made during dispatch
/// (e.g. a scroll offset) repaints — matching `run_until`'s "handled => redraw".
/// A handler that calls `notify` itself also repaints through that path.
fn handle_input_event(
    ctx: &mut AppContext,
    window_id: WindowId,
    root_view_id: EntityId,
    event: CrosstermEvent,
) {
    match event {
        CrosstermEvent::Resize(_, _) => ctx.invalidate_all_views(),
        event => {
            if let Some(warp_event) = crossterm_event_to_warp_event(event) {
                let handled = dispatch_event(
                    ctx,
                    window_id,
                    root_view_id,
                    current_terminal_size(),
                    &warp_event,
                );
                if handled {
                    ctx.invalidate_all_views();
                }
            }
        }
    }
}

/// The current terminal size in cells (each axis at least 1), falling back to a
/// sane default if the size can't be queried.
fn current_terminal_size() -> TuiSize {
    match terminal::size() {
        Ok((width, height)) => TuiSize::new(width.max(1), height.max(1)),
        Err(_) => TuiSize::new(80, 24),
    }
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
