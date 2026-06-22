//! [`TuiTranscriptView`]: the prototype's scrollback. It stores entries —
//! submitted prompts and `!`-prefixed shell commands with their captured
//! output — in insertion order and renders them bottom-anchored above the
//! input: the newest entry sits closest to the input, each new entry pushes
//! older ones up, and content that overflows the top is clipped.
//!
//! These local entries are intentionally disposable — a later milestone can
//! source them from a real conversation model without changing this view's
//! layout.

use std::cell::RefCell;
use std::rc::Rc;

use command::Stdio;
use futures::channel::oneshot;
use futures::future::Either;
use futures_lite::io::{AsyncRead, AsyncReadExt as _};
use warpui_core::elements::tui::{
    Color, Modifier, TuiBuffer, TuiCanvas, TuiCanvasCache, TuiColumn, TuiConstraint, TuiElement,
    TuiRect, TuiSize, TuiStyle, TuiText,
};
use warpui_core::{AppContext, Entity, TuiView, ViewContext};

use super::command_output::render_output_to_buffer;

/// Near-white entry text (`#f1f1f1`), bold like the user prompt in the mock.
const ENTRY_COLOR: Color = Color::Rgb(0xf1, 0xf1, 0xf1);
/// Accent color for a `!`-command's echoed command line (`#7aa2f7`).
const COMMAND_COLOR: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
/// Dim gray for the transient "running…" placeholder (`#8e8e8e`).
const RUNNING_COLOR: Color = Color::Rgb(0x8e, 0x8e, 0x8e);

/// Read size for draining a command's stdout/stderr pipes (matches codex's
/// exec chunk size).
const READ_CHUNK_SIZE: usize = 8192;
/// Cap on retained output bytes per command. Keeps the most recent tail so a
/// runaway command can't grow memory without bound.
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// A single transcript entry: a submitted prompt, or a `!`-prefixed shell
/// command and its captured output.
enum TranscriptEntry {
    Prompt(String),
    Command(CommandEntry),
}

/// A chunk streamed from a running command: raw output bytes, or the terminal
/// marker carrying the process exit status (`None` if signal-terminated or
/// unavailable) and whether it was stopped via Esc.
enum CommandChunk {
    Bytes(Vec<u8>),
    Finished {
        exit_status: Option<i32>,
        cancelled: bool,
    },
}

/// A `!`-shell command and its streaming output. `output` accumulates combined
/// stdout+stderr as it arrives; `generation` is bumped on every appended chunk
/// so the [`TuiCanvas`] cache invalidates while the content grows (and is then
/// constant once `finished`). `cache` persists the rasterized grid across
/// redraws so it is reused until the width or generation changes.
struct CommandEntry {
    command: String,
    output: Rc<RefCell<Vec<u8>>>,
    generation: u64,
    finished: bool,
    cancelled: bool,
    exit_status: Option<i32>,
    /// Esc stop signal: resolving it asks the background task to kill the child.
    /// `None` once the command has finished (or been stopped).
    cancel: Option<oneshot::Sender<()>>,
    cache: TuiCanvasCache,
}

impl TranscriptEntry {
    /// Renders this entry to an element (the trailing spacer row is added by the
    /// caller).
    fn render(&self) -> Box<dyn TuiElement> {
        match self {
            TranscriptEntry::Prompt(text) => {
                let style = TuiStyle::default()
                    .fg(ENTRY_COLOR)
                    .add_modifier(Modifier::BOLD);
                Box::new(TuiText::new(text.clone()).with_style(style))
            }
            TranscriptEntry::Command(command) => command.render(),
        }
    }
}

impl CommandEntry {
    fn render(&self) -> Box<dyn TuiElement> {
        // Echo the command line, then the streamed output grid (or a transient
        // placeholder), and a status line once it has exited non-zero.
        let header_style = TuiStyle::default()
            .fg(COMMAND_COLOR)
            .add_modifier(Modifier::BOLD);
        let header = TuiText::new(format!("$ {}", self.command)).with_style(header_style);

        let dim = TuiStyle::default().fg(RUNNING_COLOR);
        let body: Box<dyn TuiElement> = if self.output.borrow().is_empty() {
            let label = if self.finished {
                "(no output)"
            } else {
                "running…"
            };
            Box::new(TuiText::new(label).with_style(dim))
        } else {
            // Share (not clone) the buffer into the producer; `generation` makes
            // the canvas re-rasterize as the content grows.
            let output = self.output.clone();
            let generation = self.generation;
            Box::new(TuiCanvas::new(
                self.cache.clone(),
                generation,
                move |width| render_output_to_buffer(&output.borrow(), width),
            ))
        };

        // `TuiColumn::child` takes a concrete element, but `body` is already
        // boxed, so assemble the column from boxed children instead.
        let mut children: Vec<Box<dyn TuiElement>> = vec![Box::new(header), body];
        if self.finished {
            let status = if self.cancelled {
                Some("[stopped]".to_string())
            } else {
                self.exit_status
                    .filter(|code| *code != 0)
                    .map(|code| format!("[exited {code}]"))
            };
            if let Some(status) = status {
                children.push(Box::new(TuiText::new(status).with_style(dim)));
            }
        }
        Box::new(TuiColumn::with_children(children))
    }
}

#[derive(Default)]
pub struct TuiTranscriptView {
    entries: Vec<TranscriptEntry>,
}

impl TuiTranscriptView {
    /// Appends `text` as the newest transcript entry and schedules a redraw.
    pub fn append(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.entries.push(TranscriptEntry::Prompt(text));
        ctx.notify();
    }

    /// Stops every still-running `!` command (bound to Esc): each background
    /// task kills its child and reports completion, flipping the entry to a
    /// `[stopped]` state.
    pub fn cancel_running(&mut self, ctx: &mut ViewContext<Self>) {
        let mut stopped_any = false;
        for entry in &mut self.entries {
            if let TranscriptEntry::Command(command) = entry {
                if !command.finished {
                    if let Some(cancel) = command.cancel.take() {
                        // The receiver resolving makes the background task kill
                        // the child; an error just means it already exited.
                        let _ = cancel.send(());
                        stopped_any = true;
                    }
                }
            }
        }
        if stopped_any {
            ctx.notify();
        }
    }

    /// Runs `command` in the user's shell, appending a command entry immediately
    /// (in the running state) and streaming its output into that entry as it is
    /// produced. Whitespace-only commands are ignored.
    pub fn run_command(&mut self, command: String, ctx: &mut ViewContext<Self>) {
        let command = command.trim().to_string();
        if command.is_empty() {
            return;
        }

        let (cancel_sender, cancel_receiver) = oneshot::channel::<()>();
        self.entries.push(TranscriptEntry::Command(CommandEntry {
            command: command.clone(),
            output: Rc::new(RefCell::new(Vec::new())),
            generation: 0,
            finished: false,
            cancelled: false,
            exit_status: None,
            cancel: Some(cancel_sender),
            cache: TuiCanvasCache::new(),
        }));
        let entry_index = self.entries.len() - 1;
        ctx.notify();

        // Run through the user's login shell so pipes/&&/globs work. The TUI
        // front-end is unix-only for now; Windows shell selection is a follow-up.
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let (sender, receiver) = async_channel::unbounded::<CommandChunk>();

        // Drain the child's pipes off the main thread, forwarding chunks as they
        // arrive; only applying them (below) touches the view.
        ctx.background_executor()
            .spawn(stream_command(shell, command, sender, cancel_receiver))
            .detach();

        // Apply streamed chunks to the entry on the main thread, redrawing each
        // time so output appears progressively.
        ctx.spawn_stream_local(
            receiver,
            move |view, chunk, ctx| {
                if let Some(TranscriptEntry::Command(entry)) = view.entries.get_mut(entry_index) {
                    match chunk {
                        CommandChunk::Bytes(bytes) => {
                            append_capped(&mut entry.output.borrow_mut(), &bytes);
                            entry.generation += 1;
                        }
                        CommandChunk::Finished {
                            exit_status,
                            cancelled,
                        } => {
                            entry.finished = true;
                            entry.exit_status = exit_status;
                            entry.cancelled = cancelled;
                            entry.cancel = None;
                        }
                    }
                }
                ctx.notify();
            },
            move |view, ctx| {
                // Stream ended (all senders dropped); ensure the entry is marked
                // done even if the terminal chunk never arrived.
                if let Some(TranscriptEntry::Command(entry)) = view.entries.get_mut(entry_index) {
                    entry.finished = true;
                }
                ctx.notify();
            },
        );
    }
}

/// Appends `chunk` to `buffer`, dropping the oldest bytes so it never exceeds
/// [`MAX_OUTPUT_BYTES`].
fn append_capped(buffer: &mut Vec<u8>, chunk: &[u8]) {
    buffer.extend_from_slice(chunk);
    if buffer.len() > MAX_OUTPUT_BYTES {
        let drop = buffer.len() - MAX_OUTPUT_BYTES;
        buffer.drain(0..drop);
    }
}

/// Spawns `command` under `shell` with piped stdout/stderr, streaming output
/// chunks (then a terminal [`CommandChunk::Finished`]) into `sender`. Runs on a
/// background thread.
async fn stream_command(
    shell: String,
    command: String,
    sender: async_channel::Sender<CommandChunk>,
    cancel: oneshot::Receiver<()>,
) {
    let mut child = match ::command::r#async::Command::new(shell)
        .arg("-c")
        .arg(&command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = sender
                .send(CommandChunk::Bytes(
                    format!("failed to run command: {error}").into_bytes(),
                ))
                .await;
            let _ = sender
                .send(CommandChunk::Finished {
                    exit_status: None,
                    cancelled: false,
                })
                .await;
            return;
        }
    };

    // Read both pipes concurrently so a chatty stream can't deadlock the other.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let reads = futures::future::join(
        read_pipe(stdout, sender.clone()),
        read_pipe(stderr, sender.clone()),
    );
    futures::pin_mut!(reads);

    // Race reading against an Esc stop signal. `cancel` resolving wakes the task
    // even when the command is producing no output (e.g. `sleep`), so the kill
    // is prompt; killing the child then closes the pipes.
    let cancelled = match futures::future::select(reads, cancel).await {
        Either::Left(_) => false,
        Either::Right(_) => {
            let _ = child.kill();
            true
        }
    };

    let exit_status = child.status().await.ok().and_then(|status| status.code());
    let _ = sender
        .send(CommandChunk::Finished {
            exit_status,
            cancelled,
        })
        .await;
}

/// Reads `reader` to EOF in [`READ_CHUNK_SIZE`] chunks, sending each as a
/// [`CommandChunk::Bytes`]. Stops early if the receiver was dropped.
async fn read_pipe<R: AsyncRead + Unpin>(
    reader: Option<R>,
    sender: async_channel::Sender<CommandChunk>,
) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut buffer = [0u8; READ_CHUNK_SIZE];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if sender
                    .send(CommandChunk::Bytes(buffer[..n].to_vec()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

impl Entity for TuiTranscriptView {
    type Event = ();
}

impl TuiView for TuiTranscriptView {
    fn ui_name() -> &'static str {
        "TuiTranscriptView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        if self.entries.is_empty() {
            // Empty state: render nothing until the first submission.
            return Box::new(TuiColumn::new());
        }

        // Each entry is followed by a blank spacer row so successive entries
        // (and the input below) stay visually separated.
        let children: Vec<Box<dyn TuiElement>> = self
            .entries
            .iter()
            .flat_map(|entry| {
                [
                    entry.render(),
                    Box::new(TuiText::new(" ")) as Box<dyn TuiElement>,
                ]
            })
            .collect();

        Box::new(BottomAnchoredColumn::new(children))
    }
}

/// A vertical stack that anchors its children to the bottom of the area it is
/// given: when the content is shorter than the area it sits flush against the
/// bottom edge, and when it is taller the top rows are clipped (so the newest,
/// bottom-most content stays visible).
struct BottomAnchoredColumn {
    children: Vec<Box<dyn TuiElement>>,
}

impl BottomAnchoredColumn {
    fn new(children: Vec<Box<dyn TuiElement>>) -> Self {
        Self { children }
    }

    /// The height each child wants at `width`, in order.
    fn child_heights(&self, width: u16) -> Vec<u16> {
        self.children
            .iter()
            .map(|child| child.desired_height(width))
            .collect()
    }
}

impl TuiElement for BottomAnchoredColumn {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        // Fill the height offered so the content can anchor to the bottom of the
        // whole slot.
        constraint.clamp(constraint.max)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        if area.is_empty() {
            return;
        }
        let width = area.width;
        let heights = self.child_heights(width);
        let total = heights.iter().fold(0u16, |acc, &h| acc.saturating_add(h));
        if total == 0 {
            return;
        }

        // Paint the full content into a scratch buffer, then copy the bottom-most
        // rows that fit into the real area. This makes top-clipping (when the
        // content overflows) and bottom-alignment (when it underflows) fall out
        // of a single offset calculation.
        let mut scratch = TuiBuffer::empty(TuiRect::new(0, 0, width, total));
        let mut y = 0u16;
        for (child, &height) in self.children.iter().zip(&heights) {
            if height == 0 {
                continue;
            }
            child.render(TuiRect::new(0, y, width, height), &mut scratch);
            y = y.saturating_add(height);
        }

        let visible = total.min(area.height);
        let src_top = total - visible; // clipped top rows when overflowing
        let dst_top = area.y + (area.height - visible); // bottom padding when underflowing
        for row in 0..visible {
            for col in 0..width {
                let cell = scratch[(col, src_top + row)].clone();
                if let Some(dst) = buffer.cell_mut((area.x + col, dst_top + row)) {
                    *dst = cell;
                }
            }
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.child_heights(width)
            .iter()
            .fold(0u16, |acc, &h| acc.saturating_add(h))
    }
}

#[cfg(test)]
#[path = "transcript_view_tests.rs"]
mod tests;
