//! [`TuiText`]: styled text that wraps (or truncates) to the width it is laid
//! out at, built on ratatui's `Paragraph`.
//!
//! # Construction
//! Build with [`TuiText::new`] (one uniformly-styled run) or
//! [`TuiText::from_spans`] (multiple styled runs flowing as one paragraph)
//! and chain builders:
//! - [`with_style`](TuiText::with_style) sets the base [`TuiStyle`] beneath
//!   every glyph; span styles patch over it.
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

use std::mem;

use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use super::{
    TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiScreenPoint,
    TuiScreenPosition, TuiSize, TuiStyle,
};
use crate::AppContext;

pub struct TuiText {
    /// Styled runs that concatenate into the full text. Runs may contain hard
    /// newlines, which split rows exactly as they would in a single run.
    spans: Vec<(String, TuiStyle)>,
    /// Base style beneath every span; span styles patch over it.
    style: TuiStyle,
    wrap: bool,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiText {
    /// A wrapping text element holding `text` with default styling.
    pub fn new(text: impl Into<String>) -> Self {
        Self::from_spans([(text.into(), TuiStyle::default())])
    }

    /// A wrapping text element composed of styled runs that flow as one
    /// paragraph (a run is never a wrap boundary by itself). Each run's style
    /// patches over the base style set by [`with_style`](Self::with_style).
    pub fn from_spans(spans: impl IntoIterator<Item = (String, TuiStyle)>) -> Self {
        Self {
            spans: spans.into_iter().collect(),
            style: TuiStyle::default(),
            wrap: true,
            size: None,
            origin: None,
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
        if self.is_empty() {
            return 0;
        }
        u16::try_from(self.paragraph().line_count(width)).unwrap_or(u16::MAX)
    }

    /// Whether this element holds no text at all (and so occupies no rows).
    fn is_empty(&self) -> bool {
        self.spans.iter().all(|(text, _)| text.is_empty())
    }

    /// The spans re-grouped into ratatui `Line`s: hard newlines inside any
    /// span split lines; between newlines, consecutive (sub)spans share a line.
    fn text(&self) -> Text<'_> {
        let mut lines = Vec::new();
        let mut current_line = Vec::new();
        for (content, style) in &self.spans {
            let mut parts = content.split('\n');
            // `split` always yields at least one part; parts after the first
            // are each preceded by a newline, i.e. a completed line.
            if let Some(first) = parts.next() {
                if !first.is_empty() {
                    current_line.push(Span::styled(first, *style));
                }
            }
            for part in parts {
                lines.push(Line::from(mem::take(&mut current_line)));
                if !part.is_empty() {
                    current_line.push(Span::styled(part, *style));
                }
            }
        }
        lines.push(Line::from(current_line));
        Text::from(lines)
    }

    /// The ratatui `Paragraph` backing this element's measure and paint.
    fn paragraph(&self) -> Paragraph<'_> {
        let paragraph = Paragraph::new(self.text()).style(self.style);
        if self.wrap {
            paragraph.wrap(Wrap { trim: false })
        } else {
            paragraph
        }
    }
}

impl TuiElement for TuiText {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let size = if self.is_empty() {
            constraint.clamp(TuiSize::ZERO)
        } else {
            let paragraph = self.paragraph();
            let height =
                u16::try_from(paragraph.line_count(constraint.max.width)).unwrap_or(u16::MAX);
            let content_width = u16::try_from(paragraph.line_width()).unwrap_or(u16::MAX);
            TuiSize::new(
                constraint.constrain_width(content_width),
                constraint.constrain_height(height),
            )
        };
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let Some(size) = self.size else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }
        surface.render_widget(self.paragraph(), origin, size);
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
