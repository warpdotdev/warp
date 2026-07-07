//! [`TuiShimmeringText`]: a single-line text run whose glyphs lerp from a base
//! color toward a shimmer color as a highlight band sweeps left→right — the
//! terminal-cell mirror of the GUI's
//! [`ShimmeringTextElement`](crate::elements::shimmering_text), sharing its
//! band math ([`crate::elements::shimmer_math`]).
//!
//! Colors are derived at paint time from the caller's [`AnimationClock`], so
//! the animation advances on cached-element repaints and survives
//! element-tree rebuilds. Each paint requests the next repaint; the animation
//! stops as soon as the element leaves the painted tree. Intended for short
//! single-width-glyph strings (one cell per char).

use std::time::Duration;

use super::{
    Color, Modifier, TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext,
    TuiRect, TuiSize, TuiStyle,
};
use crate::color::ColorU;
use crate::elements::animation::AnimationClock;
use crate::elements::shimmer_math::{self, ShimmerConfig};
use crate::AppContext;

/// How often the shimmer repaints. The band moves about one cell per ~140ms
/// with the default config on short strings, so 100ms keeps it smooth without
/// the GUI's 30fps cadence (a full-frame TUI repaint serves every animation).
const REPAINT_INTERVAL: Duration = Duration::from_millis(100);

pub struct TuiShimmeringText {
    text: String,
    base_color: ColorU,
    shimmer_color: ColorU,
    config: ShimmerConfig,
    /// The clock the band's phase is derived from.
    clock: AnimationClock,
    modifier: Modifier,
}

impl TuiShimmeringText {
    /// A shimmering text element displaying `text` in `base_color`, lerping
    /// toward `shimmer_color` under the band. The band's phase is derived from
    /// `clock`'s elapsed time.
    pub fn new(
        text: impl Into<String>,
        base_color: ColorU,
        shimmer_color: ColorU,
        config: ShimmerConfig,
        clock: AnimationClock,
    ) -> Self {
        Self {
            text: text.into(),
            base_color,
            shimmer_color,
            config,
            clock,
            modifier: Modifier::empty(),
        }
    }

    /// Adds `modifier` (e.g. [`Modifier::BOLD`]) to every painted cell.
    pub fn with_modifier(mut self, modifier: Modifier) -> Self {
        self.modifier = self.modifier.union(modifier);
        self
    }
}

impl TuiElement for TuiShimmeringText {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        if self.text.is_empty() {
            return constraint.clamp(TuiSize::ZERO);
        }
        let width = u16::try_from(self.text.chars().count()).unwrap_or(u16::MAX);
        TuiSize::new(
            constraint.constrain_width(width),
            constraint.constrain_height(1),
        )
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        if area.is_empty() {
            return;
        }

        let glyph_count = self.text.chars().count();
        let center = shimmer_math::shimmer_center(glyph_count, self.clock.elapsed(), &self.config);

        for (index, char) in self.text.chars().enumerate() {
            let x = area
                .x
                .saturating_add(index.min(usize::from(u16::MAX)) as u16);
            if x >= area.right() {
                break;
            }
            let intensity = shimmer_math::intensity_at(index, center, &self.config);
            let color =
                shimmer_math::shimmer_color_at(self.base_color, self.shimmer_color, intensity);
            let style = TuiStyle::default()
                .fg(Color::Rgb(color.r, color.g, color.b))
                .add_modifier(self.modifier);
            if let Some(cell) = buffer.cell_mut((x, area.y)) {
                cell.set_symbol(char.to_string().as_str()).set_style(style);
            }
        }

        ctx.repaint_after(REPAINT_INTERVAL);
    }
}

#[cfg(test)]
#[path = "shimmering_text_tests.rs"]
mod tests;
