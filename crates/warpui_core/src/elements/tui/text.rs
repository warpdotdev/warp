//! [`TuiText`]: a styled run of text that wraps (or truncates) to the width it
//! is laid out at, built on ratatui's `Paragraph`.
//!
//! # Construction
//! Build with [`TuiText::new`] and chain builders:
//! - [`with_style`](TuiText::with_style) sets the [`TuiStyle`] applied to every
//!   glyph.
//! - [`truncate`](TuiText::truncate) switches from the default word-wrapping
//!   policy to single-row-per-hard-line truncation.
//!
//! # Layout policy
//! Wrapping and measurement defer to `Paragraph`, so layout, render, and
//! `desired_height` always agree:
//! - **Wrap** (default): word-wrapped with whitespace preserved
//!   (`Wrap { trim: false }`); a word wider than the row is broken at grapheme
//!   boundaries.
//! - **Truncate**: each hard line becomes one row, clipped to the width.
//!
//! Height is `Paragraph::line_count` and the natural width is
//! `Paragraph::line_width`; both are column-accurate for wide (CJK) glyphs, so a
//! wide glyph occupies two columns and is never split across rows. An empty
//! string occupies no rows.

use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::{TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext, TuiRect, TuiSize, TuiStyle};

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

    /// The number of terminal rows this text occupies when laid out at `width`
    /// columns. Matches what `layout` would return as the height component.
    pub fn desired_height(&self, width: u16) -> u16 {
        if self.text.is_empty() {
            return 0;
        }
        u16::try_from(self.paragraph().line_count(width)).unwrap_or(u16::MAX)
    }

    /// The ratatui `Paragraph` backing this element's measure and paint.
    fn paragraph(&self) -> Paragraph<'_> {
        let paragraph = Paragraph::new(self.text.as_str()).style(self.style);
        if self.wrap {
            paragraph.wrap(Wrap { trim: false })
        } else {
            paragraph
        }
    }
}

impl TuiElement for TuiText {
    fn layout(&mut self, constraint: TuiConstraint, _ctx: &mut TuiLayoutContext) -> TuiSize {
        if self.text.is_empty() {
            return constraint.clamp(TuiSize::ZERO);
        }
        let paragraph = self.paragraph();
        let height = u16::try_from(paragraph.line_count(constraint.max.width)).unwrap_or(u16::MAX);
        let content_width = u16::try_from(paragraph.line_width()).unwrap_or(u16::MAX);
        TuiSize::new(
            constraint.constrain_width(content_width),
            constraint.constrain_height(height),
        )
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {
        if area.is_empty() {
            return;
        }
        Widget::render(self.paragraph(), area, buffer);
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
