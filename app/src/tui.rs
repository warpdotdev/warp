//! The headless `warp-tui` front-end: a real (headless) Warp app whose root
//! window is a [`RootTuiView`] rendered through the `tui`-gated WarpUI backend.
//!
//! `RootTuiView` composes two child views — a [`TuiTranscriptView`] filling the
//! space above a bottom-anchored single-row [`TuiInputView`] — and routes
//! submissions into the shared [`TuiTerminalSession`]. A leading `!` runs the
//! rest as a command through the persistent `TerminalModel`; plain text is
//! appended to the local transcript. Keystrokes are forwarded to the PTY when a
//! command is running or the alt-screen is active. [`init`] is called from
//! `run_internal` once the headless app is up (see [`crate::run_tui`]). Ctrl-C
//! quit is handled by the runtime's input loop.

mod grid_render;
mod input_view;
mod session;
mod transcript_view;

use std::sync::Arc;
use std::time::Duration;

use input_view::{InputEvent, TuiInputView};
use parking_lot::FairMutex;
use session::{encode_keydown, TuiTerminalSession};
use transcript_view::TuiTranscriptView;
use warpui_core::elements::tui::{
    TuiBuffer, TuiChildView, TuiColumn, TuiConstrainedBox, TuiConstraint, TuiElement,
    TuiEventContext, TuiPresentationContext, TuiRect, TuiSize,
};
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{
    AddWindowOptions, AppContext, Entity, Event, SingletonEntity, TuiView, TypedActionView,
    ViewContext, ViewHandle,
};

use crate::ai::blocklist::agent_view::AgentViewState;
use crate::terminal::color;
use crate::terminal::model::block::Block;
use crate::terminal::model::terminal_model::{TerminalInputState, TerminalModel};

/// The bottom input frame's height: one text row inside a single-cell rounded
/// border (top + bottom), i.e. three rows total.
const INPUT_ROWS: u16 = 3;

/// The interrupt byte (Ctrl-C) sent to the PTY on Esc/Cancel.
const INTERRUPT_BYTE: u8 = 0x03;

/// How often the background task checks for terminal size changes.
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The root TUI view: a transcript that grows upward above a fixed,
/// bottom-anchored input. It owns both child views and forwards the input's
/// submissions into the shared terminal session.
struct RootTuiView {
    transcript: ViewHandle<TuiTranscriptView>,
    input: ViewHandle<TuiInputView>,
}

impl RootTuiView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let transcript = ctx.add_tui_view(|_| TuiTranscriptView::default());
        let input = ctx.add_typed_action_tui_view(|_| TuiInputView::default());

        ctx.subscribe_to_view(&input, |root, _input, event, ctx| match event {
            InputEvent::Submitted(text) => {
                if let Some(command) = text.strip_prefix('!') {
                    let command = command.to_string();
                    TuiTerminalSession::handle(ctx)
                        .update(ctx, |session, ctx| session.run_command(&command, ctx));
                } else {
                    let text = text.clone();
                    root.transcript
                        .update(ctx, |transcript, ctx| transcript.append(text, ctx));
                }
            }
            InputEvent::Cancel => {
                TuiTerminalSession::handle(ctx).update(ctx, |session, ctx| {
                    session.write_input_bytes(vec![INTERRUPT_BYTE], ctx);
                });
            }
        });

        ctx.focus(&input);

        // Repaint when the model changes (PTY output, block updates, etc.).
        // Gracefully skip if the session singleton isn't registered (tests).
        if let Some(session) = TuiTerminalSession::handle(ctx).downgrade().upgrade(ctx) {
            let model_events = session.read(ctx, |s, _| s.model_events().clone());
            ctx.subscribe_to_model(&model_events, |_, _, _event, ctx| {
                ctx.notify();
            });

            // Drain the PTY wakeup channel to repaint on terminal output. The
            // wakeup channel is the terminal's redraw signal (fired on every
            // PTY read); without draining it the receiver is dropped, the
            // sender logs "Failed to send Wakeup event: Closed", and streamed
            // command output never triggers a redraw.
            if let Some(wakeups_rx) = session.update(ctx, |s, _| s.take_wakeups_rx()) {
                ctx.spawn_stream_local(
                    wakeups_rx,
                    |_view, _wakeup, ctx| ctx.notify(),
                    |_view, _ctx| {},
                );
            }
        }

        // Periodically check the terminal size and resize the model + PTY when
        // it changes. The TUI runtime invalidates on resize but doesn't call
        // back into the session, so we poll from a background timer.
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
            |_view, (rows, cols), ctx| {
                TuiTerminalSession::handle(ctx)
                    .update(ctx, |session, ctx| session.resize(rows, cols, ctx));
            },
            |_, _| {},
        );

        Self { transcript, input }
    }
}

impl Entity for RootTuiView {
    type Event = ();
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let input = TuiChildView::new(&self.input, ctx);

        // If the session singleton isn't registered (tests), render just the input.
        let Some(session) = TuiTerminalSession::handle(ctx).downgrade().upgrade(ctx) else {
            return Box::new(TuiKeyInterceptor::new(Box::new(
                TuiColumn::new().child(TuiConstrainedBox::new(input).with_max_rows(INPUT_ROWS)),
            )));
        };

        let model = session.read(ctx, |s, _| s.model());
        let colors = model.lock().colors();

        // When the alt-screen is active, render it full-pane (no input view).
        if model.lock().is_alt_screen_active() {
            return Box::new(TuiKeyInterceptor::new(Box::new(TuiAltScreenElement::new(
                model, colors,
            ))));
        }

        // Otherwise: block list (transcript area) + input.
        let block_list = TuiBlockListElement::new(model, colors);

        let column = TuiColumn::new()
            .flex_child(block_list)
            .child(TuiConstrainedBox::new(input).with_max_rows(INPUT_ROWS));

        Box::new(TuiKeyInterceptor::new(Box::new(column)))
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
}

impl TuiKeyInterceptor {
    fn new(child: Box<dyn TuiElement>) -> Self {
        Self { child }
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
            let session = TuiTerminalSession::as_ref(app);
            let model = session.model();
            let input_state = model.lock().terminal_input_state();

            if matches!(
                input_state,
                TerminalInputState::LongRunningCommand | TerminalInputState::AltScreen
            ) {
                let key_without_modifiers = details.key_without_modifiers.as_deref();
                let bytes = encode_keydown(keystroke, key_without_modifiers, chars, &model)
                    .or_else(|| {
                        if chars.is_empty() {
                            None
                        } else {
                            Some(chars.as_bytes().to_vec())
                        }
                    });

                if let Some(bytes) = bytes {
                    if !bytes.is_empty() {
                        ctx.dispatch_app_update(move |ctx| {
                            TuiTerminalSession::handle(ctx).update(ctx, |session, ctx| {
                                session.write_input_bytes(bytes, ctx);
                            });
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

    /// Computes the displayed height of each block (prompt+command + output),
    /// skipping blocks the TUI does not render (see `block_display_rows`).
    fn block_heights(&self) -> Vec<u16> {
        let model = self.model.lock();
        let agent_view_state = model.block_list().agent_view_state();
        model
            .block_list()
            .blocks()
            .iter()
            .map(|block| {
                block_display_rows(block, agent_view_state)
                    .map(|(pc, out)| pc.saturating_add(out))
                    .unwrap_or(0)
            })
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
        let blocks = model.block_list().blocks();
        let agent_view_state = model.block_list().agent_view_state();
        let width = area.width;

        // Compute each block's height (skipping blocks the TUI doesn't render).
        let heights: Vec<u16> = blocks
            .iter()
            .map(|block| {
                block_display_rows(block, agent_view_state)
                    .map(|(pc, out)| pc.saturating_add(out))
                    .unwrap_or(0)
            })
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
        for (block, &height) in blocks.iter().zip(&heights) {
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
            let (pc_rows, out_rows) = block_display_rows(block, agent_view_state).unwrap_or((0, 0));

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

/// Holds the live TUI session for the app's lifetime; dropping it on app
/// teardown restores the terminal.
struct TuiSession {
    _handle: TuiDriverHandle,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Creates the TUI root window and starts the headless draw + input driver.
/// The [`TuiTerminalSession`] singleton is registered first so the session core
/// exists before any view renders or key events dispatch.
pub fn init(ctx: &mut AppContext) {
    TuiTerminalSession::register(ctx);

    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        RootTuiView::new,
    );

    match spawn_tui_driver(ctx, window_id, root) {
        Ok(handle) => {
            ctx.add_singleton_model(|_| TuiSession { _handle: handle });
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
