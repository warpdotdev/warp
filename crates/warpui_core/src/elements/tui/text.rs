//! [`TuiText`]: a styled run of text that wraps (or truncates) to the width it
//! is laid out at.
//!
//! # Construction
//! Build with [`TuiText::new`] and chain builders:
//! - [`with_style`](TuiText::with_style) sets the [`TuiStyle`] applied to every
//!   glyph.
//! - [`truncate`](TuiText::truncate) switches from the default word-wrapping
//!   policy to single-row-per-line truncation.
//!
//! # Layout policy
//! The text is first split into *hard lines* on `'\n'`. Each hard line is then
//! laid out against the available width in one of two modes:
//!
//! - **Wrap** (default): tokens (space-separated words) are packed greedily,
//!   separated by a single space, starting a new row whenever the next token
//!   would overflow. A token wider than the whole width is hard-broken at
//!   grapheme boundaries. Runs of spaces collapse to a single separator.
//! - **Truncate**: each hard line becomes exactly one row; glyphs past the
//!   width are dropped by the buffer when painted.
//!
//! Column widths are measured with `unicode-width`, so a wide (CJK) glyph
//! occupies two columns and is never split across rows.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{TuiBuffer, TuiConstraint, TuiElement, TuiRect, TuiSize, TuiStyle};

pub struct TuiText {
    text: String,
    style: TuiStyle,
    wrap: bool,
}

impl TuiText {
    /// A wrapping text element holding `text` with default styling.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: TuiStyle::default(),
            wrap: true,
        }
    }

    pub fn with_style(mut self, style: TuiStyle) -> Self {
        self.style = style;
        self
    }

    /// Lays each hard line out as a single (clipped) row instead of wrapping.
    pub fn truncate(mut self) -> Self {
        self.wrap = false;
        self
    }

    /// The rows this text occupies when laid out at `width` columns, under the
    /// active wrap/truncate policy. This is the single source of truth shared by
    /// [`layout`](TuiElement::layout), [`render`](TuiElement::render), and
    /// [`desired_height`](TuiElement::desired_height).
    fn rows(&self, width: u16) -> Vec<String> {
        if self.text.is_empty() {
            return Vec::new();
        }
        if self.wrap {
            self.text
                .split('\n')
                .flat_map(|line| wrap_hard_line(line, width))
                .collect()
        } else {
            self.text.split('\n').map(str::to_owned).collect()
        }
    }
}

impl TuiElement for TuiText {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        let rows = self.rows(constraint.max.width);
        let content_width = rows.iter().map(|row| text_width(row)).max().unwrap_or(0);
        let height = u16::try_from(rows.len()).unwrap_or(u16::MAX);
        TuiSize::new(
            constraint.constrain_width(content_width),
            constraint.constrain_height(height),
        )
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        if area.is_empty() {
            return;
        }
        for (offset, row) in self.rows(area.width).iter().enumerate() {
            let Ok(offset) = u16::try_from(offset) else {
                break;
            };
            if offset >= area.height {
                break;
            }
            buffer.set_str(area.x, area.y + offset, area.width, row, self.style);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        u16::try_from(self.rows(width).len()).unwrap_or(u16::MAX)
    }
}

/// Greedily wraps a single newline-free line to `width` columns. Returns one
/// `String` per visual row (empty when `width` is zero).
fn wrap_hard_line(line: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut rows = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for token in line.split(' ').filter(|token| !token.is_empty()) {
        let token_width = text_width(token);

        if !current.is_empty() && current_width + 1 + token_width <= width {
            current.push(' ');
            current.push_str(token);
            current_width += 1 + token_width;
            continue;
        }

        if !current.is_empty() {
            rows.push(std::mem::take(&mut current));
            current_width = 0;
        }

        if token_width <= width {
            current = token.to_owned();
            current_width = token_width;
            continue;
        }

        // The token is wider than a full row: hard-break it at grapheme
        // boundaries, carrying the final fragment into `current`.
        let chunks = hard_break(token, width);
        let last = chunks.len().saturating_sub(1);
        for (index, chunk) in chunks.into_iter().enumerate() {
            if index == last {
                current_width = text_width(&chunk);
                current = chunk;
            } else {
                rows.push(chunk);
            }
        }
    }

    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// Splits `token` into fragments each at most `width` columns wide, breaking
/// only on grapheme boundaries (a single glyph wider than `width` is kept whole
/// and clipped later by the buffer).
fn hard_break(token: &str, width: u16) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for grapheme in token.graphemes(true) {
        let glyph_width = grapheme_width(grapheme);
        if !current.is_empty() && current_width + glyph_width > width {
            chunks.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push_str(grapheme);
        current_width += glyph_width;
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn text_width(text: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(text)).unwrap_or(u16::MAX)
}

/// The column width of a single grapheme, floored at 1 to mirror the buffer's
/// own measurement of zero-width clusters.
fn grapheme_width(grapheme: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(grapheme).max(1)).unwrap_or(u16::MAX)
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
