//! The in-progress `⋮ Warping (Ns)` indicator row rendered at the bottom of a
//! streaming agent block — the TUI counterpart of the GUI's warping indicator.
//!
//! All animation state (spinner frame, shimmer phase, elapsed counter) is
//! derived from the wall clock against a single anchor, so it advances on
//! cached-element repaints and survives element-tree rebuilds. The row is
//! wrapped in a [`TuiLiveElement`], so it keeps requesting repaints while it
//! is part of the painted tree and stops as soon as the block re-renders
//! without it.

use std::time::Duration;

use instant::Instant;
use warpui_core::elements::shimmer_math::ShimmerConfig;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiConstraint, TuiElement, TuiFlex, TuiLayoutContext, TuiLiveElement,
    TuiPaintContext, TuiRect, TuiShimmeringText, TuiSize, TuiStyle, TuiText,
};
use warpui_core::AppContext;

use crate::tui_builder::TuiUiBuilder;

/// One spinner keyframe: the glyph shown and how long it holds.
struct SpinnerFrame {
    glyph: &'static str,
    hold: Duration,
}

/// Builds a spinner keyframe holding `glyph` for `hold_ms`.
const fn frame(glyph: &'static str, hold_ms: u64) -> SpinnerFrame {
    SpinnerFrame {
        glyph,
        hold: Duration::from_millis(hold_ms),
    }
}

/// The spinner choreography from the Figma prototype: a 180° rotation right,
/// a 180° rotation back left, then a few fast full spins right, restarting.
///
/// Terminal cells can't rotate glyphs, so each 45° step maps to the nearest
/// three-dot orientation glyph (`⋮ ⋰ ⋯ ⋱`, repeating every 180°). The hold
/// durations are tuned by eye — the prototype's timings aren't
/// machine-readable.
const SPINNER_TIMELINE: &[SpinnerFrame] = &[
    // 180° right (clockwise), one 45° step per frame.
    frame("⋮", 200),
    frame("⋰", 200),
    frame("⋯", 200),
    frame("⋱", 200),
    // 180° back left (counter-clockwise).
    frame("⋮", 200),
    frame("⋱", 200),
    frame("⋯", 200),
    frame("⋰", 200),
    // Rest at vertical before the fast spins.
    frame("⋮", 200),
    // Fast spins right: one and a half turns (12 × 45° steps = 540°, three
    // glyph cycles), ending back at vertical — the loop's restarting `⋮`
    // doubles as the final step.
    frame("⋰", 50),
    frame("⋯", 50),
    frame("⋱", 50),
    frame("⋮", 50),
    frame("⋰", 50),
    frame("⋯", 50),
    frame("⋱", 50),
    frame("⋮", 50),
    frame("⋰", 50),
    frame("⋯", 50),
    frame("⋱", 50),
];

/// Repaint cadence for the spinner and elapsed counter, matching the spinner
/// timeline's shortest frame so the fast spins don't skip frames. The shimmer
/// label requests its own repaints too; requests coalesce to the earliest
/// deadline.
const REPAINT_INTERVAL: Duration = Duration::from_millis(50);

/// The shimmering label text.
const WARPING_LABEL: &str = "Warping";

/// Renders the `⋮ Warping (Ns)` row for an exchange that has been running for
/// `elapsed`.
pub(crate) fn render_warping_indicator(elapsed: Duration, app: &AppContext) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let anchor = Instant::now() - elapsed;

    let spinner = LiveText {
        text: Box::new(move || spinner_frame_at(anchor.elapsed()).to_owned()),
        style: builder.warping_spinner_style(),
    };

    let label = TuiShimmeringText::new(
        WARPING_LABEL,
        builder.warping_base_color(),
        builder.warping_shimmer_color(),
        ShimmerConfig::default(),
        anchor,
    )
    .with_modifier(Modifier::BOLD);

    let counter = LiveText {
        text: Box::new(move || format!("({}s)", anchor.elapsed().as_secs())),
        style: builder.muted_text_style(),
    };

    let row = TuiFlex::row()
        .child(spinner.finish())
        .child(TuiText::new(" ").truncate().finish())
        .child(label.finish())
        .child(TuiText::new(" ").truncate().finish())
        .child(counter.finish());
    TuiLiveElement::new(row.finish(), REPAINT_INTERVAL).finish()
}

/// The spinner glyph shown after `elapsed` of animation: walks the looped
/// [`SPINNER_TIMELINE`] by each keyframe's hold duration.
fn spinner_frame_at(elapsed: Duration) -> &'static str {
    let period: Duration = SPINNER_TIMELINE.iter().map(|frame| frame.hold).sum();
    let mut into_loop = Duration::from_nanos((elapsed.as_nanos() % period.as_nanos()) as u64);
    for frame in SPINNER_TIMELINE {
        if into_loop < frame.hold {
            return frame.glyph;
        }
        into_loop -= frame.hold;
    }
    // Unreachable: `into_loop` is within the summed period.
    SPINNER_TIMELINE[0].glyph
}

/// A single-row text leaf whose content is recomputed from the wall clock on
/// every layout/paint pass, so cached-element repaints show the current frame.
struct LiveText {
    /// Produces the current text.
    text: Box<dyn Fn() -> String>,
    style: TuiStyle,
}

impl TuiElement for LiveText {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let width = u16::try_from((self.text)().chars().count()).unwrap_or(u16::MAX);
        TuiSize::new(
            constraint.constrain_width(width),
            constraint.constrain_height(1),
        )
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {
        if area.is_empty() {
            return;
        }
        buffer.set_stringn(
            area.x,
            area.y,
            (self.text)(),
            usize::from(area.width),
            self.style,
        );
    }
}

#[cfg(test)]
#[path = "warping_indicator_tests.rs"]
mod tests;
