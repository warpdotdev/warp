//! Syntax-highlighted TUI file viewer demo backed by `CodeEditorModel`.
//!
//! This shows how to drive the shared editor infrastructure in char-cell (TUI)
//! mode as a read-only viewer: it loads a file, sets the language for syntax
//! highlighting, and paints per-character syntax colors by overlaying styles
//! directly into the [`TuiBuffer`] (the same `buffer.set_style` approach
//! `TuiInputElement` uses for selection highlights).
//!
//! Run:
//! ```sh
//! cargo run -p warp_tui --example tui_file_viewer_demo -- app/src/code/editor/model.rs
//! ```
//!
//! With no argument it views its own source file.
//!
//! Keys:
//!   Esc                quit
//!   ↑ / Ctrl+P         scroll up one line
//!   ↓ / Ctrl+N         scroll down one line

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use rangemap::RangeSet;
use string_offset::CharOffset;
use syntax_tree::ColorMap;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warpui_core::color::ColorU;
use warpui_core::elements::tui::{
    Color, Modifier, TuiBuffer, TuiColumn, TuiConstraint, TuiElement, TuiEventHandler,
    TuiLayoutContext, TuiParentElement, TuiRect, TuiSize, TuiStyle, TuiText,
};
use warpui_core::platform::WindowStyle;
use warpui_core::runtime::TuiRuntime;
use warpui_core::{
    AddWindowOptions, App, AppContext, Entity, Event, TuiView, TypedActionView, ViewContext,
};

/// Char-cell width handed to the editor model. Soft-wrap is unused here (each
/// logical line is rendered on its own row and clipped to the viewport), so the
/// exact value only needs to be wide enough to avoid surprising the model.
const TERMINAL_WIDTH: u16 = 200;

/// Upper bound on the number of file rows built per frame. The column clips to
/// the real terminal height; over-building is harmless and keeps scrolling
/// simple without needing the viewport height at view-render time.
const MAX_VISIBLE_ROWS: usize = 200;

/// Number of header rows (status line, help line, separator) above the file.
const HEADER_ROWS: u16 = 3;

/// A fixed dark-terminal syntax color map, so highlighting does not depend on
/// `Appearance` (which is not registered in this lightweight TUI context). Uses
/// an OneDark-style palette that reads well on dark backgrounds.
fn tui_color_map() -> ColorMap {
    ColorMap {
        keyword_color: ColorU::new(198, 120, 221, 255), // magenta
        function_color: ColorU::new(97, 175, 239, 255), // blue
        string_color: ColorU::new(152, 195, 121, 255),  // green
        type_color: ColorU::new(224, 108, 117, 255),    // red
        number_color: ColorU::new(209, 154, 102, 255),  // orange
        comment_color: ColorU::new(106, 115, 125, 255), // gray
        property_color: ColorU::new(86, 182, 194, 255), // cyan
        tag_color: ColorU::new(224, 108, 117, 255),     // red
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// View
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum ViewerAction {
    Quit,
    ScrollUp,
    ScrollDown,
}

struct FileViewerView {
    model: warpui_core::ModelHandle<CodeEditorModel>,
    file_name: String,
    /// First visible logical line (0-indexed).
    scroll_offset: usize,
    quit: Rc<Cell<bool>>,
}

impl Entity for FileViewerView {
    type Event = ();
}

impl FileViewerView {
    fn new(
        path: PathBuf,
        content: String,
        quit: Rc<Cell<bool>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let model = ctx.add_model(|ctx| CodeEditorModel::new_tui(TERMINAL_WIDTH, ctx));

        // Re-render whenever the model changes (notably once async syntax
        // parsing completes and highlight colors become available).
        ctx.subscribe_to_model(&model, |_, _, _event: &CodeEditorModelEvent, ctx| {
            ctx.notify();
        });

        model.update(ctx, |m, ctx| {
            // Install the fixed terminal color map before setting the language so
            // the highlight query is built with real colors (the `new_tui`
            // default is an all-black stub).
            m.syntax_tree().update(ctx, |syntax_tree, _| {
                syntax_tree.set_color_map(tui_color_map())
            });
            m.set_language_with_local_path(&path, ctx);
            m.reset(InitialBufferState::plain_text(&content), ctx);
            // Kick off the async tree-sitter parse so highlights_in_ranges
            // returns colors on the next render after the parse completes.
            m.rebuild_layout_with_syntax_highlighting(ctx);
        });

        Self {
            model,
            file_name,
            scroll_offset: 0,
            quit,
        }
    }

    /// Number of logical lines in the buffer (ignoring a single trailing
    /// newline so a file ending in `\n` doesn't show a phantom blank line).
    fn total_lines(&self, ctx: &AppContext) -> usize {
        let buffer = self.model.as_ref(ctx).content().as_ref(ctx);
        if buffer.is_empty() {
            return 1;
        }
        let text = buffer.text().into_string();
        let trimmed = text.strip_suffix('\n').unwrap_or(&text);
        trimmed.split('\n').count().max(1)
    }
}

impl TuiView for FileViewerView {
    fn ui_name() -> &'static str {
        "FileViewerView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let inner = self.model.as_ref(ctx);
        let buffer = inner.content().as_ref(ctx);

        let text = if buffer.is_empty() {
            String::new()
        } else {
            buffer.text().into_string()
        };
        // Drop a single trailing newline so split() doesn't yield a phantom row.
        let text = text.strip_suffix('\n').unwrap_or(&text);
        let lines: Vec<&str> = text.split('\n').collect();
        let total_lines = lines.len();

        // Read syntax highlight colors for the whole buffer. The returned map is
        // keyed by buffer `CharOffset`, where the character at 0-based string
        // index `i` lives at `CharOffset(i + 1)`. Returns `None` until the async
        // parse has produced a tree.
        let mut ranges = RangeSet::new();
        ranges.insert(CharOffset::from(1)..buffer.max_charoffset());
        let highlights = inner
            .syntax_tree()
            .as_ref(ctx)
            .highlights_in_ranges(ranges, None, ctx);

        let bold = TuiStyle::default().add_modifier(Modifier::BOLD);
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);

        let mut column = TuiColumn::new();
        column = column.with_child(Box::new(
            TuiText::new(format!("{}  ·  {total_lines} lines", self.file_name))
                .with_style(bold)
                .truncate(),
        ));
        column = column.with_child(Box::new(
            TuiText::new("Esc quit · ↑/↓ or Ctrl+P/Ctrl+N scroll")
                .with_style(dim)
                .truncate(),
        ));
        column = column.with_child(Box::new(
            TuiText::new("─".repeat(TERMINAL_WIDTH as usize))
                .with_style(dim)
                .truncate(),
        ));

        let scroll = self.scroll_offset.min(total_lines.saturating_sub(1));
        let visible = MAX_VISIBLE_ROWS.min(total_lines.saturating_sub(scroll));

        // 0-based global string index of the first visible line's start.
        let mut global = 0usize;
        for line in lines.iter().take(scroll) {
            global += line.chars().count() + 1; // + newline
        }

        let mut gutter_rows: Vec<(u16, u16)> = Vec::new();
        let mut color_cells: Vec<(u16, u16, Color)> = Vec::new();

        for vi in 0..visible {
            let li = scroll + vi;
            let line = lines[li];
            let prefix = format!("{:>4} ", li + 1);
            let gutter_width = prefix.chars().count() as u16;
            let row_in_area = HEADER_ROWS + vi as u16;
            gutter_rows.push((row_in_area, gutter_width));

            // Expand tabs to spaces for display and track visual column per char.
            // `col` = raw char index (used for CharOffset lookup)
            // `vcol` = visual column after tab expansion (used for color_cells x)
            let mut display_line = String::new();
            let mut vcol: u16 = 0;
            if let Some(highlights) = highlights.as_ref() {
                for (col, ch) in line.chars().enumerate() {
                    let offset = CharOffset::from(global + col + 2);
                    if let Some(color) = highlights.get(&offset) {
                        color_cells.push((
                            row_in_area,
                            gutter_width + vcol,
                            Color::Rgb(color.r, color.g, color.b),
                        ));
                    }
                    if ch == '\t' {
                        let spaces = 4 - (vcol as usize % 4);
                        for _ in 0..spaces {
                            display_line.push(' ');
                        }
                        vcol += spaces as u16;
                    } else {
                        display_line.push(ch);
                        vcol += 1;
                    }
                }
            } else {
                // No highlights yet — just expand tabs for display.
                for ch in line.chars() {
                    if ch == '\t' {
                        let spaces = 4 - (vcol as usize % 4);
                        for _ in 0..spaces {
                            display_line.push(' ');
                        }
                        vcol += spaces as u16;
                    } else {
                        display_line.push(ch);
                        vcol += 1;
                    }
                }
            }

            column = column.with_child(Box::new(
                TuiText::new(format!("{prefix}{display_line}")).truncate(),
            ));
            global += line.chars().count() + 1;
        }

        let element = FileViewerElement {
            column,
            gutter_rows,
            color_cells,
        };

        Box::new(
            TuiEventHandler::new(element)
                .on_key("escape", |_, ctx, _| {
                    ctx.dispatch_typed_action(ViewerAction::Quit)
                })
                .on_key("up", |_, ctx, _| {
                    ctx.dispatch_typed_action(ViewerAction::ScrollUp)
                })
                .on_key("down", |_, ctx, _| {
                    ctx.dispatch_typed_action(ViewerAction::ScrollDown)
                })
                .on_key("p", |event, ctx, _| {
                    if is_ctrl(event) {
                        ctx.dispatch_typed_action(ViewerAction::ScrollUp);
                    }
                })
                .on_key("n", |event, ctx, _| {
                    if is_ctrl(event) {
                        ctx.dispatch_typed_action(ViewerAction::ScrollDown);
                    }
                }),
        )
    }

    fn keymap_context(&self, _ctx: &AppContext) -> warpui_core::keymap::Context {
        let mut ctx = warpui_core::keymap::Context::default();
        ctx.set.insert("FileViewerView");
        ctx
    }
}

impl TypedActionView for FileViewerView {
    type Action = ViewerAction;

    fn handle_action(&mut self, action: &ViewerAction, ctx: &mut ViewContext<Self>) {
        match action {
            ViewerAction::Quit => self.quit.set(true),
            ViewerAction::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                ctx.notify();
            }
            ViewerAction::ScrollDown => {
                let max_scroll = self.total_lines(ctx).saturating_sub(1);
                self.scroll_offset = (self.scroll_offset + 1).min(max_scroll);
                ctx.notify();
            }
        }
    }
}

/// Returns whether `event` is a `KeyDown` with the Ctrl modifier held.
fn is_ctrl(event: &Event) -> bool {
    matches!(event, Event::KeyDown { keystroke, .. } if keystroke.ctrl)
}

// ─────────────────────────────────────────────────────────────────────────────
// Element — overlays syntax colors and dims the gutter on top of plain rows
// ─────────────────────────────────────────────────────────────────────────────

struct FileViewerElement {
    column: TuiColumn,
    /// `(row_in_area, gutter_width)` for each file row, used to dim the gutter.
    gutter_rows: Vec<(u16, u16)>,
    /// `(row_in_area, col_in_area, color)` per syntax-colored character cell.
    color_cells: Vec<(u16, u16, Color)>,
}

impl TuiElement for FileViewerElement {
    fn layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext) -> TuiSize {
        self.column.layout(constraint, ctx)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        self.column.render(area, buffer, ctx);

        let bottom = area.y.saturating_add(area.height);
        let right = area.x.saturating_add(area.width);

        // Dim the line-number gutter.
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);
        for &(row, gutter_width) in &self.gutter_rows {
            let y = area.y.saturating_add(row);
            if y < bottom && gutter_width > 0 {
                let width = gutter_width.min(area.width);
                buffer.set_style(TuiRect::new(area.x, y, width, 1), dim);
            }
        }

        // Paint per-character syntax colors.
        for &(row, col, color) in &self.color_cells {
            let y = area.y.saturating_add(row);
            let x = area.x.saturating_add(col);
            if y < bottom && x < right {
                buffer.set_style(TuiRect::new(x, y, 1, 1), TuiStyle::default().fg(color));
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    // Default to viewing this example's own source file.
    let path: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(file!()));

    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            eprintln!("Failed to read {}: {err}", path.display());
            std::process::exit(1);
        }
    };

    let path_for_view = path.clone();
    App::test((), move |mut app| async move {
        let quit = Rc::new(Cell::new(false));
        let quit_for_view = quit.clone();

        let (window_id, root) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                move |ctx| FileViewerView::new(path_for_view, content, quit_for_view, ctx),
            )
        });

        let mut runtime = TuiRuntime::enter(&app, window_id, root).expect("enter alternate screen");

        // The background tree-sitter parse runs on a Tokio thread and posts its
        // result via the foreground executor. Give Tokio time to finish (the
        // parse of a typical source file takes <10 ms), then yield to the
        // LocalExecutor so the result callback fires before the first frame.
        // We use run_until_async with a 0-tick quit predicate to pump the
        // executor without entering the interactive loop.
        std::thread::sleep(std::time::Duration::from_millis(150));
        let mut ticks = 0u32;
        runtime
            .run_until_async(&mut app, |_| {
                ticks += 1;
                ticks > 20
            })
            .await
            .expect("flush background tasks");

        let quit_for_loop = quit.clone();
        runtime
            .run_until_async(&mut app, move |_| quit_for_loop.get())
            .await
            .expect("run TUI event loop");
    });
}
