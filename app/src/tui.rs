//! The headless `warp-tui` front-end: a real (headless) Warp app whose root
//! window is a [`RootTuiView`] rendered through the `tui`-gated WarpUI backend.
//!
//! `RootTuiView` renders the shared [`TerminalModel`]'s block list above a
//! bottom-anchored [`TuiInputView`] and routes submissions into the shared
//! [`TuiTerminalSession`]. A leading `!` runs the rest as a command through the
//! persistent `TerminalModel`; plain text is reserved for the future agent
//! prompt and ignored for now. Keystrokes are forwarded to the PTY when a
//! command is running or the alt-screen is active. [`init`] is called from
//! `run_internal` once the headless app is up (see [`crate::run_tui`]). Ctrl-C
//! quit is handled by the runtime's input loop.

mod grid_render;
mod input_view;
mod session;

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Receiver;
use input_view::{InputEvent, TuiInputView};
use parking_lot::FairMutex;
use pathfinder_geometry::vector::vec2f;
use session::encode_keydown;
use warpui_core::elements::tui::{
    TuiBuffer, TuiChildView, TuiColumn, TuiConstrainedBox, TuiConstraint, TuiElement,
    TuiEventContext, TuiPresentationContext, TuiRect, TuiSize,
};
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{
    AddWindowOptions, AppContext, Entity, Event, ModelHandle, SingletonEntity, TuiView,
    TypedActionView, ViewContext, ViewHandle, WeakViewHandle,
};

use crate::ai::blocklist::agent_view::AgentViewState;
use crate::banner::BannerState;
use crate::terminal::color;
#[cfg(unix)]
use crate::terminal::event::BlockCompletedEvent;
use crate::terminal::input::CommandExecutionSource;
use crate::terminal::local_tty::terminal_manager::{
    build_session_core, SessionCore, TerminalManager,
};
use crate::terminal::model::block::Block;
use crate::terminal::model::blocks::BlockHeightItem;
#[cfg(unix)]
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::model::terminal_model::{TerminalInputState, TerminalModel};
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::view::ExecuteCommandEvent;
use crate::terminal::writeable_pty::terminal_surface::{PtySurfaceIntent, TerminalSurface};
use crate::terminal::{ShellLaunchData, SizeInfo, SizeUpdate};

/// The bottom input frame's height: one text row inside a single-cell rounded
/// border (top + bottom), i.e. three rows total.
const INPUT_ROWS: u16 = 3;

/// The interrupt byte (Ctrl-C) sent to the PTY on Esc/Cancel.
const INTERRUPT_BYTE: u8 = 0x03;

/// How often the background task checks for terminal size changes.
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The root TUI view and the [`TerminalSurface`] the TUI's
/// `TerminalManager<RootTuiView>` drives: it renders the shared model's block
/// list above a bottom-anchored input, and emits [`TuiRootEvent`]s (command
/// execution, raw keystroke passthrough, resize) that the manager's wiring
/// translates into PTY writes.
struct RootTuiView {
    input: ViewHandle<TuiInputView>,
    model: Arc<FairMutex<TerminalModel>>,
    view_handle: WeakViewHandle<Self>,
}

/// Events [`RootTuiView`] emits for the terminal manager's PTY wiring.
enum TuiRootEvent {
    /// Run a `!`-command in the shell.
    ExecuteCommand(ExecuteCommandEvent),
    /// Forward raw bytes to the PTY (keystroke passthrough / interrupt).
    WriteBytes(Vec<u8>),
    /// Resize the PTY to a new cell grid.
    Resize(SizeUpdate),
}

impl RootTuiView {
    fn new(
        model: Arc<FairMutex<TerminalModel>>,
        model_events: ModelHandle<ModelEventDispatcher>,
        wakeups_rx: Receiver<()>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let input = ctx.add_typed_action_tui_view(|_| TuiInputView::default());

        ctx.subscribe_to_view(&input, |root, _input, event, ctx| match event {
            InputEvent::Submitted(text) => {
                // Only `!`-prefixed input runs as a shell command today; plain
                // text is reserved for the future agent prompt and ignored.
                if let Some(command) = text.strip_prefix('!') {
                    let Some(session_id) =
                        root.model.lock().block_list().active_block().session_id()
                    else {
                        log::warn!("[DEBUG] TUI: cannot run command — not bootstrapped yet");
                        return;
                    };
                    ctx.emit(TuiRootEvent::ExecuteCommand(ExecuteCommandEvent {
                        command: command.to_string(),
                        session_id,
                        workflow_id: None,
                        workflow_command: None,
                        should_add_command_to_history: true,
                        source: CommandExecutionSource::User,
                    }));
                }
            }
            InputEvent::Cancel => {
                ctx.emit(TuiRootEvent::WriteBytes(vec![INTERRUPT_BYTE]));
            }
        });

        ctx.focus(&input);

        // Repaint when the model changes (PTY output, block updates, etc.).
        ctx.subscribe_to_model(&model_events, |_, _, _event, ctx| {
            ctx.notify();
        });

        // Drain the PTY wakeup channel (the terminal's redraw signal, fired on
        // every PTY read) to repaint on streamed command output.
        ctx.spawn_stream_local(
            wakeups_rx,
            |_view, _wakeup, ctx| ctx.notify(),
            |_view, _ctx| {},
        );

        // Periodically check the terminal size; on change, resize the model and
        // emit a Resize event so the manager resizes the PTY. The TUI runtime
        // invalidates on resize but doesn't call back into the session.
        let (resize_tx, resize_rx) = async_channel::unbounded::<(usize, usize)>();
        ctx.background_executor()
            .spawn(async move {
                let mut last = current_terminal_cells();
                loop {
                    warpui::r#async::Timer::after(RESIZE_POLL_INTERVAL).await;
                    let now = current_terminal_cells();
                    if now != last {
                        last = now;
                        if let Some((cols, rows)) = now {
                            let _ = resize_tx.send((rows as usize, cols as usize)).await;
                        }
                    }
                }
            })
            .detach();

        ctx.spawn_stream_local(
            resize_rx,
            |view, (rows, cols), ctx| {
                let last_size = *view.model.lock().block_list().size();
                let new_size = SizeInfo::new_without_font_metrics(rows, cols);
                let size_update = SizeUpdate::new_for_headless_resize(last_size, new_size);
                view.model.lock().resize(size_update);
                ctx.emit(TuiRootEvent::Resize(size_update));
            },
            |_, _| {},
        );

        Self {
            input,
            model,
            view_handle: ctx.handle(),
        }
    }
}

impl Entity for RootTuiView {
    type Event = TuiRootEvent;
}

impl TerminalSurface for RootTuiView {
    fn pty_intent(event: &Self::Event) -> Option<PtySurfaceIntent> {
        match event {
            TuiRootEvent::ExecuteCommand(command_event) => {
                Some(PtySurfaceIntent::ExecuteCommand(command_event.clone()))
            }
            TuiRootEvent::WriteBytes(bytes) => {
                Some(PtySurfaceIntent::WriteBytes(Cow::Owned(bytes.clone())))
            }
            TuiRootEvent::Resize(size_update) => Some(PtySurfaceIntent::Resize(*size_update)),
        }
    }

    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn on_active_shell_launch_data_updated(
        &mut self,
        _shell_launch_data: Option<ShellLaunchData>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>) {
        log::error!("[DEBUG] TUI: pty spawn failed: {error:#}");
        ctx.notify();
    }

    #[cfg(unix)]
    fn wants_password_poll(&self, _ctx: &AppContext) -> bool {
        false
    }

    #[cfg(unix)]
    fn on_possible_password_prompt(
        &mut self,
        _block_index: Option<BlockIndex>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(unix)]
    fn on_block_completed(
        &mut self,
        _completed: &BlockCompletedEvent,
        _ctx: &mut ViewContext<Self>,
    ) {
    }
}

/// Implements the object-safe manager trait for the TUI instantiation. The
/// sharing-aware `on_view_detached` lives on the GUI instantiation; the TUI uses
/// the trait's no-op default.
impl crate::terminal::TerminalManager for TerminalManager<RootTuiView> {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.shared_model()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let input = TuiChildView::new(&self.input, ctx);
        let colors = self.model.lock().colors();

        // When the alt-screen is active, render it full-pane (no input view).
        if self.model.lock().is_alt_screen_active() {
            return Box::new(TuiKeyInterceptor::new(
                Box::new(TuiAltScreenElement::new(self.model.clone(), colors)),
                self.model.clone(),
                self.view_handle.clone(),
            ));
        }

        // Otherwise: block list (transcript area) + input.
        let block_list = TuiBlockListElement::new(self.model.clone(), colors);
        let column = TuiColumn::new()
            .flex_child(block_list)
            .child(TuiConstrainedBox::new(input).with_max_rows(INPUT_ROWS));

        Box::new(TuiKeyInterceptor::new(
            Box::new(column),
            self.model.clone(),
            self.view_handle.clone(),
        ))
    }
}

impl TypedActionView for RootTuiView {
    type Action = ();
}

/// A wrapper element that intercepts `KeyDown` events before they reach the
/// child. When the terminal is in `LongRunningCommand` or `AltScreen` state,
/// keystrokes are encoded and forwarded to the PTY (the TUI behaves like a real
/// terminal). In `InputEditor` or `NotBootstrapped` state, events pass through
/// to the child unchanged.
struct TuiKeyInterceptor {
    child: Box<dyn TuiElement>,
    model: Arc<FairMutex<TerminalModel>>,
    root: WeakViewHandle<RootTuiView>,
}

impl TuiKeyInterceptor {
    fn new(
        child: Box<dyn TuiElement>,
        model: Arc<FairMutex<TerminalModel>>,
        root: WeakViewHandle<RootTuiView>,
    ) -> Self {
        Self { child, model, root }
    }
}

impl TuiElement for TuiKeyInterceptor {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        self.child.layout(constraint)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        self.child.render(area, buffer);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.child.desired_height(width)
    }

    fn cursor_position(&self, area: TuiRect) -> Option<(u16, u16)> {
        self.child.cursor_position(area)
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        if let Event::KeyDown {
            keystroke,
            chars,
            details,
            ..
        } = event
        {
            let _ = app;
            let input_state = self.model.lock().terminal_input_state();

            if matches!(
                input_state,
                TerminalInputState::LongRunningCommand | TerminalInputState::AltScreen
            ) {
                let key_without_modifiers = details.key_without_modifiers.as_deref();
                let bytes = encode_keydown(keystroke, key_without_modifiers, chars, &self.model)
                    .or_else(|| {
                        if chars.is_empty() {
                            None
                        } else {
                            Some(chars.as_bytes().to_vec())
                        }
                    });

                if let Some(bytes) = bytes {
                    if !bytes.is_empty() {
                        // Emit on the root view so the manager's surface wiring
                        // forwards the bytes to the PTY.
                        let root = self.root.clone();
                        ctx.dispatch_app_update(move |app| {
                            if let Some(root_view) = root.upgrade(app) {
                                root_view.update(app, move |_root, ctx| {
                                    ctx.emit(TuiRootEvent::WriteBytes(bytes));
                                });
                            }
                        });
                    }
                }
                return true;
            }
        }

        self.child.dispatch_event(event, area, ctx, app)
    }
}

/// Renders the `TerminalModel`'s block list bottom-anchored into the
/// transcript area. Each block's `prompt_and_command_grid` and `output_grid`
/// are painted via `render_grid`.
struct TuiBlockListElement {
    model: Arc<FairMutex<TerminalModel>>,
    colors: color::List,
}

impl TuiBlockListElement {
    fn new(model: Arc<FairMutex<TerminalModel>>, colors: color::List) -> Self {
        Self { model, colors }
    }

    /// Sum of each rendered block's displayed rows, in block-list order
    /// (see `ordered_block_rows`).
    fn block_heights(&self) -> Vec<u16> {
        let model = self.model.lock();
        ordered_block_rows(&model)
            .iter()
            .map(|(_, pc, out)| pc.saturating_add(*out))
            .collect()
    }
}

impl TuiElement for TuiBlockListElement {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        constraint.clamp(constraint.max)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        if area.is_empty() {
            return;
        }
        let model = self.model.lock();
        let items = ordered_block_rows(&model);
        let width = area.width;

        // Per-rendered-block height (prompt+command rows + output rows), in order.
        let heights: Vec<u16> = items
            .iter()
            .map(|(_, pc, out)| pc.saturating_add(*out))
            .collect();
        let total: u16 = heights.iter().copied().fold(0, u16::saturating_add);
        if total == 0 {
            return;
        }

        // Bottom-anchor: the newest (last) block sits at the bottom.
        let visible = total.min(area.height);
        let top_clip = total - visible; // rows clipped from the top
        let dst_top = area.y + (area.height - visible);

        // Paint blocks top-to-bottom into the buffer, skipping clipped rows.
        let mut src_y: u16 = 0;
        let mut dst_y = dst_top;
        for (&(block, pc_rows, out_rows), &height) in items.iter().zip(&heights) {
            if height == 0 {
                continue;
            }
            // Skip blocks entirely above the clip.
            if src_y + height <= top_clip {
                src_y += height;
                continue;
            }
            // Partially clipped block: skip the clipped rows.
            let skip = top_clip.saturating_sub(src_y);

            // Render prompt+command grid.
            let pc_skip = skip.min(pc_rows);
            if pc_skip < pc_rows {
                let pc_area = TuiRect::new(area.x, dst_y, width, pc_rows - pc_skip);
                render_block_grid(
                    block.prompt_and_command_grid(),
                    pc_skip as usize,
                    pc_area,
                    buffer,
                    &self.colors,
                );
                dst_y = dst_y.saturating_add(pc_rows - pc_skip);
            }

            // Render output grid.
            let out_skip = skip.saturating_sub(pc_rows);
            if out_skip < out_rows {
                let out_area = TuiRect::new(area.x, dst_y, width, out_rows - out_skip);
                render_block_grid(
                    block.output_grid(),
                    out_skip as usize,
                    out_area,
                    buffer,
                    &self.colors,
                );
                dst_y = dst_y.saturating_add(out_rows - out_skip);
            }

            src_y += height;
            if dst_y >= area.y + area.height {
                break;
            }
        }
    }

    fn desired_height(&self, _width: u16) -> u16 {
        self.block_heights()
            .iter()
            .copied()
            .fold(0, u16::saturating_add)
    }
}

/// Renders the alt-screen grid full-pane.
struct TuiAltScreenElement {
    model: Arc<FairMutex<TerminalModel>>,
    colors: color::List,
}

impl TuiAltScreenElement {
    fn new(model: Arc<FairMutex<TerminalModel>>, colors: color::List) -> Self {
        Self { model, colors }
    }
}

impl TuiElement for TuiAltScreenElement {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        constraint.clamp(constraint.max)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        let model = self.model.lock();
        let grid = model.alt_screen().grid_handler();
        grid_render::render_grid(grid, area, buffer, &self.colors);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        use crate::terminal::model::grid::Dimensions as _;
        let model = self.model.lock();
        let grid = model.alt_screen().grid_handler();
        grid.len_displayed().unwrap_or_else(|| grid.visible_rows()) as u16
    }
}

/// Walks the block list's ordered items and returns the terminal blocks the TUI
/// should paint, each with its (prompt+command rows, output rows), in order.
///
/// This iterates the same `block_heights` sum tree the GUI's block list renders
/// from (the `match` in `app/src/terminal/block_list_element.rs:3307`). Today
/// only the `Block` arm renders; `RichContent` (agent/AI blocks) is the seam
/// where interleaved agent output will be painted once the agent cluster lands.
/// `Block` items map 1:1, in order, to `blocks()` by their running count.
fn ordered_block_rows(model: &TerminalModel) -> Vec<(&Block, u16, u16)> {
    let agent_view_state = model.block_list().agent_view_state();
    let blocks = model.block_list().blocks();
    let mut items = Vec::new();
    let mut block_count = 0usize;
    for height_item in model.block_list().block_heights().cursor::<(), ()>() {
        match height_item {
            BlockHeightItem::Block(_) => {
                let index = block_count;
                block_count += 1;
                if let Some(block) = blocks.get(index) {
                    if let Some((pc, out)) = block_display_rows(block, agent_view_state) {
                        items.push((block, pc, out));
                    }
                }
            }
            // Agent/AI output interleaves here once the agent cluster lands: this
            // is where a TUI-native AI block would be produced (GUI counterpart:
            // block_list_element.rs:3484).
            BlockHeightItem::RichContent(_) => {}
            // Visual-only items the TUI prototype does not render.
            BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. } => {}
        }
    }
    items
}

/// Returns the (prompt+command rows, output rows) the TUI should paint for
/// `block`, or `None` to skip it: blocks the GUI hides (bootstrap, empty,
/// agent-only) via `is_visible`, and the idle current-prompt block (no command
/// started or finished), which the TUI surfaces through its own input view.
fn block_display_rows(block: &Block, agent_view_state: &AgentViewState) -> Option<(u16, u16)> {
    if !block.is_visible(agent_view_state) || !(block.started() || block.finished()) {
        return None;
    }
    let pc = if block.should_hide_command_grid() {
        0
    } else {
        block.prompt_and_command_grid().len_displayed() as u16
    };
    let out = if block.should_hide_output_grid() {
        0
    } else {
        block.output_grid().len_displayed() as u16
    };
    Some((pc, out))
}

/// Renders a `BlockGrid` starting from `skip_rows` into `area`.
fn render_block_grid(
    block_grid: &crate::terminal::model::blockgrid::BlockGrid,
    skip_rows: usize,
    area: TuiRect,
    buffer: &mut TuiBuffer,
    colors: &color::List,
) {
    use crate::terminal::model::grid::Dimensions as _;

    let grid = block_grid.grid_handler();
    // Use `BlockGrid::len_displayed()` (falls back to the grid's full length
    // when there is no displayed-output filter) so the painted row count matches
    // the height reserved in `TuiBlockListElement`. The raw `GridHandler`
    // `len_displayed()` returns `None` for an ordinary block, which would paint
    // zero rows even though the block reserved space.
    let num_rows = block_grid.len_displayed();
    let num_cols = grid.columns().min(area.width as usize);

    for (i, row_idx) in (skip_rows..num_rows).enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let Some(row) = grid.row(row_idx) else {
            continue;
        };
        for col_idx in 0..num_cols {
            let x = area.x + col_idx as u16;
            let cell = &row[col_idx];
            let style = grid_render::cell_to_style(cell, colors);
            let symbol = grid_render::sanitized_symbol(cell);
            if let Some(buffer_cell) = buffer.cell_mut((x, y)) {
                buffer_cell.set_symbol(&symbol);
                buffer_cell.set_style(style);
            }
        }
    }
}

/// Holds the live TUI session for the app's lifetime: the draw/input driver and
/// the shared terminal manager. Dropping it on app teardown restores the
/// terminal and shuts down the PTY event loop.
struct TuiSession {
    _handle: TuiDriverHandle,
    _manager: ModelHandle<Box<dyn crate::terminal::TerminalManager>>,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Builds the TUI's shared terminal session and root window, then starts the
/// headless draw + input driver.
///
/// The session core is built first; the [`RootTuiView`] window is created
/// against the core's model + redraw channel; then a `TerminalManager<RootTuiView>`
/// is wired to that view as its [`TerminalSurface`], so the TUI drives the same
/// shared manager the GUI uses.
pub fn init(ctx: &mut AppContext) {
    let startup_directory = std::env::current_dir().ok();
    let (cols, rows) = current_terminal_cells().unwrap_or((80, 24));
    let initial_size = vec2f(cols as f32, rows as f32);
    // The TUI app is headless, so `build_session_core` would otherwise size the
    // model with a hardcoded default. Pass the real terminal size so the shell's
    // PTY winsize matches the actual terminal (e.g. `ls` columns).
    let headless_size = SizeInfo::new_without_font_metrics(rows as usize, cols as usize);
    let SessionCore {
        model,
        sessions,
        model_events,
        pty_controller,
        event_loop_tx,
        event_loop_rx,
        wakeups_rx,
        inactive_pty_reads_rx,
        channel_event_proxy,
        wsl_name_or_shell_starter,
    } = build_session_core(
        startup_directory.clone(),
        None,
        initial_size,
        Some(headless_size),
        None,
        ctx,
    );

    // The root view owns the model + redraw channel and renders/drives input.
    let model_for_view = model.clone();
    let model_events_for_view = model_events.clone();
    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        move |ctx| RootTuiView::new(model_for_view, model_events_for_view, wakeups_rx, ctx),
    );

    // Wire the shared terminal manager to the root view as its surface, then
    // asynchronously determine the shell and spawn the PTY.
    let banner = ctx.add_model(|_| BannerState::default());
    let env_vars = std::env::vars_os().collect();
    let manager = TerminalManager::<RootTuiView>::from_session_core(
        root.clone(),
        model,
        sessions,
        model_events,
        pty_controller,
        None,
        None,
        event_loop_tx,
        event_loop_rx,
        inactive_pty_reads_rx,
        channel_event_proxy,
        wsl_name_or_shell_starter,
        startup_directory,
        env_vars,
        banner,
        None,
        ctx,
    );

    match spawn_tui_driver(ctx, window_id, root) {
        Ok(handle) => {
            ctx.add_singleton_model(|_| TuiSession {
                _handle: handle,
                _manager: manager,
            });
        }
        Err(error) => {
            log::error!("failed to start the TUI driver: {error}");
            // Not in the alternate screen yet (entering it is what failed), so
            // print to stderr too — otherwise the process just exits instantly
            // with the reason buried in the log file.
            eprintln!(
                "warp-tui: could not start the terminal UI: {error}\n\
                 Run it directly in an interactive terminal (a real TTY), not piped or backgrounded."
            );
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        }
    }
}

/// Reads the current terminal size in cells from crossterm.
fn current_terminal_cells() -> Option<(u16, u16)> {
    crossterm::terminal::size().ok()
}
