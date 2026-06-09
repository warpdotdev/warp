//! Rasterizes a WarpUI [`Scene`] into a grid of terminal cells and writes it to
//! the terminal using crossterm.
//!
//! One terminal cell == one WarpUI "pixel": the scene is built with a backing
//! scale factor of 1.0 and a window size of `(cols, rows)`, so mapping a scene
//! coordinate to a cell is a simple `floor()`. Each frame is rasterized into a
//! [`CellGrid`] (styled `char`s with truecolor fg/bg), then diffed against the
//! previously-drawn grid so only changed cells are rewritten.
//!
//! The surface used by the event loop is fixed:
//!
//! * [`TerminalRenderer::new`] — enter raw mode + alternate screen, hide cursor.
//! * [`TerminalRenderer::render`] — rasterize a scene and flush the diff.
//!
//! [`Scene`]: crate::Scene

use std::io::{self, BufWriter, Stdout, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::style::{
    Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use unicode_width::UnicodeWidthChar;

use crate::color::ColorU;
use crate::elements::Fill;
use crate::geometry::rect::RectF;
use crate::scene::{Border, Glyph, Layer, Rect};
use crate::Scene;

/// Background used for cells the scene never paints. The app's root view
/// normally fills the window, so this is only visible at the very edges.
const DEFAULT_BG: ColorU = ColorU {
    r: 0x0d,
    g: 0x11,
    b: 0x17,
    a: 0xff,
};

/// Foreground used for otherwise-unstyled cells (cleared/space cells).
const DEFAULT_FG: ColorU = ColorU {
    r: 0xc9,
    g: 0xd1,
    b: 0xd9,
    a: 0xff,
};

// Rounded box-drawing characters used to paint bordered rects.
const BOX_TOP_LEFT: char = '╭';
const BOX_TOP_RIGHT: char = '╮';
const BOX_BOTTOM_LEFT: char = '╰';
const BOX_BOTTOM_RIGHT: char = '╯';
const BOX_HORIZONTAL: char = '─';
const BOX_VERTICAL: char = '│';

/// A single styled terminal cell.
#[derive(Clone, Copy, PartialEq)]
struct Cell {
    ch: char,
    fg: ColorU,
    bg: ColorU,
    /// Rendered with reduced intensity (used for faded glyphs).
    dim: bool,
    /// The right half of a double-width glyph drawn in the previous column; the
    /// writer skips it because the wide glyph already covers this cell.
    wide_tail: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            dim: false,
            wide_tail: false,
        }
    }
}

/// A `cols` x `rows` grid of styled cells, addressed in row-major order.
struct CellGrid {
    cols: u16,
    rows: u16,
    cells: Vec<Cell>,
}

impl CellGrid {
    fn new(cols: u16, rows: u16) -> Self {
        let len = cols as usize * rows as usize;
        Self {
            cols,
            rows,
            cells: vec![Cell::default(); len],
        }
    }

    /// Row-major index of `(col, row)`, or `None` if outside the grid.
    fn index(&self, col: i32, row: i32) -> Option<usize> {
        if col < 0 || row < 0 {
            return None;
        }
        let (col, row) = (col as u16, row as u16);
        if col >= self.cols || row >= self.rows {
            return None;
        }
        Some(row as usize * self.cols as usize + col as usize)
    }

    /// Fills the cells covered by `rect`'s background, alpha-compositing over
    /// whatever is already there. A fully-opaque fill also clears the glyph in
    /// each covered cell so it occludes lower layers.
    fn fill_rect(&mut self, rect: &Rect, clip: &CellRect) {
        let Some(color) = fill_color(&rect.background) else {
            return;
        };
        if color.a == 0 {
            return;
        }
        let area = rect_to_cells(rect.bounds).intersect(clip);
        for row in area.y0..=area.y1 {
            for col in area.x0..=area.x1 {
                let Some(idx) = self.index(col, row) else {
                    continue;
                };
                let cell = &mut self.cells[idx];
                cell.bg = over(color, cell.bg);
                if color.a == u8::MAX {
                    cell.ch = ' ';
                    cell.fg = DEFAULT_FG;
                    cell.dim = false;
                    cell.wide_tail = false;
                }
            }
        }
    }

    /// Draws rounded box-drawing characters along the enabled edges of `rect`'s
    /// border, recoloring (but not erasing the background of) each edge cell.
    fn draw_border(&mut self, rect: &Rect, clip: &CellRect) {
        let border = rect.border;
        if border.width <= 0.0 || !(border.top || border.bottom || border.left || border.right) {
            return;
        }
        let Some(color) = fill_color(&border.color) else {
            return;
        };
        if color.a == 0 {
            return;
        }
        let area = rect_to_cells(rect.bounds);
        if area.is_empty() {
            return;
        }
        for row in area.y0..=area.y1 {
            for col in area.x0..=area.x1 {
                let edges = EdgePosition {
                    top: row == area.y0,
                    bottom: row == area.y1,
                    left: col == area.x0,
                    right: col == area.x1,
                };
                let Some(ch) = border_char(edges, &border) else {
                    continue;
                };
                if !clip.contains(col, row) {
                    continue;
                }
                let Some(idx) = self.index(col, row) else {
                    continue;
                };
                let cell = &mut self.cells[idx];
                cell.fg = over(color, cell.bg);
                cell.ch = ch;
                cell.dim = false;
                cell.wide_tail = false;
            }
        }
    }

    /// Draws a single glyph at `floor(position)`. Space, control and
    /// zero-width glyphs are skipped so they cannot erase backgrounds or
    /// borders; double-width glyphs reserve the following cell.
    fn draw_glyph(&mut self, glyph: &Glyph, clip: &CellRect) {
        let Some(ch) = char::from_u32(glyph.glyph_key.glyph_id) else {
            return;
        };
        let width = ch.width().unwrap_or(0);
        if width == 0 || ch == ' ' {
            return;
        }
        let col = glyph.position.x().floor() as i32;
        let row = glyph.position.y().floor() as i32;
        if !clip.contains(col, row) {
            return;
        }
        let Some(idx) = self.index(col, row) else {
            return;
        };
        let fg = over(glyph.color, self.cells[idx].bg);
        let cell = &mut self.cells[idx];
        cell.ch = ch;
        cell.fg = fg;
        cell.dim = glyph.fade.is_some();
        cell.wide_tail = false;

        if width >= 2 && clip.contains(col + 1, row) {
            if let Some(tail_idx) = self.index(col + 1, row) {
                self.cells[tail_idx].ch = ' ';
                self.cells[tail_idx].wide_tail = true;
            }
        }
    }
}

/// Which edges of a rectangle a perimeter cell lies on.
#[derive(Clone, Copy)]
struct EdgePosition {
    top: bool,
    bottom: bool,
    left: bool,
    right: bool,
}

/// An inclusive rectangle in integer cell coordinates.
#[derive(Clone, Copy)]
struct CellRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl CellRect {
    fn contains(&self, col: i32, row: i32) -> bool {
        col >= self.x0 && col <= self.x1 && row >= self.y0 && row <= self.y1
    }

    fn is_empty(&self) -> bool {
        self.x1 < self.x0 || self.y1 < self.y0
    }

    fn intersect(&self, other: &CellRect) -> CellRect {
        CellRect {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }
}

/// Owns stdout and the previously-drawn grid (for frame diffing), and restores
/// the terminal on drop.
pub(super) struct TerminalRenderer {
    out: BufWriter<Stdout>,
    prev: CellGrid,
}

impl TerminalRenderer {
    pub(super) fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = BufWriter::new(io::stdout());
        execute!(out, EnterAlternateScreen, Hide)?;
        Ok(Self {
            out,
            // A zero-sized previous grid forces a full redraw on the first frame.
            prev: CellGrid::new(0, 0),
        })
    }

    pub(super) fn render(&mut self, scene: &Scene, cols: u16, rows: u16) -> io::Result<()> {
        if cols == 0 || rows == 0 {
            return Ok(());
        }

        let mut next = CellGrid::new(cols, rows);
        let bounds = CellRect {
            x0: 0,
            y0: 0,
            x1: cols as i32 - 1,
            y1: rows as i32 - 1,
        };

        // Layers are painted bottom-to-top; within a layer, backgrounds and
        // borders paint before glyphs so text sits on top.
        for layer in scene.layers() {
            let clip = layer_clip(layer, &bounds);
            if clip.is_empty() {
                continue;
            }
            for rect in &layer.rects {
                next.fill_rect(rect, &clip);
                next.draw_border(rect, &clip);
            }
            for glyph in &layer.glyphs {
                next.draw_glyph(glyph, &clip);
            }
        }

        let full_redraw = self.prev.cols != cols || self.prev.rows != rows;
        if full_redraw {
            queue!(self.out, Clear(ClearType::All))?;
        }
        self.write_diff(next, full_redraw)
    }

    /// Emits only the cells that differ from the previous frame, coalescing
    /// redundant color/attribute escapes, then flushes once.
    fn write_diff(&mut self, next: CellGrid, full_redraw: bool) -> io::Result<()> {
        // The terminal's pen state is unknown at the start of each frame, so the
        // first emitted cell always writes its fg, bg and intensity explicitly.
        let mut pen_fg: Option<ColorU> = None;
        let mut pen_bg: Option<ColorU> = None;
        let mut pen_dim: Option<bool> = None;

        for row in 0..next.rows {
            for col in 0..next.cols {
                let idx = row as usize * next.cols as usize + col as usize;
                let cell = next.cells[idx];
                if cell.wide_tail {
                    continue;
                }
                let unchanged =
                    !full_redraw && self.prev.cells.get(idx).is_some_and(|prev| *prev == cell);
                if unchanged {
                    continue;
                }

                queue!(self.out, MoveTo(col, row))?;

                if pen_dim != Some(cell.dim) {
                    let attr = if cell.dim {
                        Attribute::Dim
                    } else {
                        Attribute::NormalIntensity
                    };
                    queue!(self.out, SetAttribute(attr))?;
                    pen_dim = Some(cell.dim);
                }
                if pen_fg != Some(cell.fg) {
                    queue!(self.out, SetForegroundColor(to_color(cell.fg)))?;
                    pen_fg = Some(cell.fg);
                }
                if pen_bg != Some(cell.bg) {
                    queue!(self.out, SetBackgroundColor(to_color(cell.bg)))?;
                    pen_bg = Some(cell.bg);
                }
                queue!(self.out, Print(cell.ch))?;
            }
        }

        self.out.flush()?;
        self.prev = next;
        Ok(())
    }
}

impl Drop for TerminalRenderer {
    fn drop(&mut self) {
        // Best-effort restoration of the host terminal.
        let _ = execute!(self.out, Show, LeaveAlternateScreen);
        let _ = self.out.flush();
        let _ = disable_raw_mode();
    }
}

fn to_color(color: ColorU) -> Color {
    Color::Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

/// Resolves a [`Fill`] to a single color, approximating a gradient by its
/// midpoint. `Fill::None` yields `None`.
fn fill_color(fill: &Fill) -> Option<ColorU> {
    match fill {
        Fill::None => None,
        Fill::Solid(color) => Some(*color),
        Fill::Gradient {
            start_color,
            end_color,
            ..
        } => Some(blend_half(*start_color, *end_color)),
    }
}

/// Alpha-composites `src` over an assumed-opaque `dst`, returning an opaque
/// color.
fn over(src: ColorU, dst: ColorU) -> ColorU {
    if src.a == u8::MAX {
        return ColorU { a: u8::MAX, ..src };
    }
    if src.a == 0 {
        return dst;
    }
    let sa = src.a as f32 / 255.0;
    let blend = |s: u8, d: u8| (s as f32 * sa + d as f32 * (1.0 - sa)).round() as u8;
    ColorU {
        r: blend(src.r, dst.r),
        g: blend(src.g, dst.g),
        b: blend(src.b, dst.b),
        a: u8::MAX,
    }
}

/// Channel-wise average of two colors.
fn blend_half(a: ColorU, b: ColorU) -> ColorU {
    let mid = |x: u8, y: u8| ((x as u16 + y as u16) / 2) as u8;
    ColorU {
        r: mid(a.r, b.r),
        g: mid(a.g, b.g),
        b: mid(a.b, b.b),
        a: mid(a.a, b.a),
    }
}

/// Converts a pixel-space rect to the inclusive range of cells it covers.
fn rect_to_cells(rect: RectF) -> CellRect {
    CellRect {
        x0: rect.min_x().floor() as i32,
        y0: rect.min_y().floor() as i32,
        x1: rect.max_x().ceil() as i32 - 1,
        y1: rect.max_y().ceil() as i32 - 1,
    }
}

/// The cell-space clip region for a layer, intersected with the screen bounds.
fn layer_clip(layer: &Layer, bounds: &CellRect) -> CellRect {
    match layer.clip_bounds {
        Some(clip) => rect_to_cells(clip).intersect(bounds),
        None => *bounds,
    }
}

/// Picks the box-drawing character for a perimeter cell, given which sides of
/// the border are enabled. Corners are rounded only when both adjoining sides
/// are present; otherwise they fall back to the single enabled side.
fn border_char(edge: EdgePosition, border: &Border) -> Option<char> {
    let (t, b, l, r) = (border.top, border.bottom, border.left, border.right);
    if edge.top && edge.left {
        return corner(t && l, BOX_TOP_LEFT, t, l);
    }
    if edge.top && edge.right {
        return corner(t && r, BOX_TOP_RIGHT, t, r);
    }
    if edge.bottom && edge.left {
        return corner(b && l, BOX_BOTTOM_LEFT, b, l);
    }
    if edge.bottom && edge.right {
        return corner(b && r, BOX_BOTTOM_RIGHT, b, r);
    }
    if edge.top {
        return t.then_some(BOX_HORIZONTAL);
    }
    if edge.bottom {
        return b.then_some(BOX_HORIZONTAL);
    }
    if edge.left {
        return l.then_some(BOX_VERTICAL);
    }
    if edge.right {
        return r.then_some(BOX_VERTICAL);
    }
    None
}

/// Resolves a corner cell: the rounded glyph when both sides meet, else the
/// horizontal/vertical edge for whichever single side is enabled.
fn corner(both: bool, rounded: char, horizontal_side: bool, vertical_side: bool) -> Option<char> {
    if both {
        Some(rounded)
    } else if horizontal_side {
        Some(BOX_HORIZONTAL)
    } else if vertical_side {
        Some(BOX_VERTICAL)
    } else {
        None
    }
}
