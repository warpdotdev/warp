//! The in-progress `⋮ Warping (Ns)` indicator row rendered between the
//! transcript and the input box while the selected conversation is in
//! progress — the TUI counterpart of the GUI's warping indicator.
//!
//! All animation state (spinner frame, shimmer phase, elapsed counter) is
//! derived from one [`AnimationClock`] carrying the exchange's elapsed time,
//! so it advances on cached-element repaints and survives element-tree
//! rebuilds.
//! The spinner and counter are [`TuiAnimated`] leaves and the shimmer label
//! self-schedules, so the row keeps requesting repaints while it is part of
//! the painted tree and stops as soon as the session view re-renders without
//! it.

use std::sync::LazyLock;
use std::time::Duration;

use warpui_core::elements::animation::{AnimationClock, Keyframe, KeyframeTimeline};
use warpui_core::elements::shimmer_math::ShimmerConfig;
use warpui_core::elements::tui::{
    Modifier, TuiAnimated, TuiElement, TuiFlex, TuiShimmeringText, TuiText,
};
use warpui_core::AppContext;

use crate::tui_builder::TuiUiBuilder;

/// The spinner choreography from the Figma prototype: a 180° rotation right,
/// a 180° rotation back left, then a few fast full spins right, restarting.
///
/// Terminal cells can't rotate glyphs, so each 45° step maps to the nearest
/// three-dot orientation glyph (`⋮ ⋰ ⋯ ⋱`, repeating every 180°). The hold
/// durations are tuned by eye — the prototype's timings aren't
/// machine-readable.
static SPINNER_TIMELINE: LazyLock<KeyframeTimeline<&'static str>> = LazyLock::new(|| {
    KeyframeTimeline::new([
        // 180° right (clockwise), one 45° step per frame.
        Keyframe::from_millis("⋮", 200),
        Keyframe::from_millis("⋰", 200),
        Keyframe::from_millis("⋯", 200),
        Keyframe::from_millis("⋱", 200),
        // 180° back left (counter-clockwise).
        Keyframe::from_millis("⋮", 200),
        Keyframe::from_millis("⋱", 200),
        Keyframe::from_millis("⋯", 200),
        Keyframe::from_millis("⋰", 200),
        // Rest at vertical before the fast spins.
        Keyframe::from_millis("⋮", 200),
        // Fast spins right: one and a half turns (12 × 45° steps = 540°, three
        // glyph cycles), ending back at vertical — the loop's restarting `⋮`
        // doubles as the final step.
        Keyframe::from_millis("⋰", 50),
        Keyframe::from_millis("⋯", 50),
        Keyframe::from_millis("⋱", 50),
        Keyframe::from_millis("⋮", 50),
        Keyframe::from_millis("⋰", 50),
        Keyframe::from_millis("⋯", 50),
        Keyframe::from_millis("⋱", 50),
        Keyframe::from_millis("⋮", 50),
        Keyframe::from_millis("⋰", 50),
        Keyframe::from_millis("⋯", 50),
        Keyframe::from_millis("⋱", 50),
    ])
});

/// Renders the `⋮ Warping (Ns)` row for an exchange that has been running for
/// `elapsed`.
pub(crate) fn render_warping_indicator(elapsed: Duration, app: &AppContext) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    // One clock, already `elapsed` into the exchange, drives all three parts
    // so they stay phase-locked; each repaint reads its current elapsed time.
    let clock = AnimationClock::starting_at(elapsed);

    // The spinner repaints at its timeline's shortest hold so the fast spins
    // don't skip frames; repaint requests coalesce to the earliest deadline.
    let spinner_style = builder.warping_spinner_style();
    let spinner = TuiAnimated::new(Duration::from_millis(50), move || {
        TuiText::new(*SPINNER_TIMELINE.value_at(clock.elapsed()))
            .with_style(spinner_style)
            .truncate()
            .finish()
    });

    let label = TuiShimmeringText::new(
        "Warping",
        builder.warping_base_color(),
        builder.warping_shimmer_color(),
        ShimmerConfig::default(),
        clock,
    )
    .with_modifier(Modifier::BOLD);

    let counter_style = builder.muted_text_style();
    let counter = TuiAnimated::new(Duration::from_secs(1), move || {
        TuiText::new(format!("({}s)", clock.elapsed().as_secs()))
            .with_style(counter_style)
            .truncate()
            .finish()
    });

    TuiFlex::row()
        .child(spinner.finish())
        .child(TuiText::new(" ").truncate().finish())
        .child(label.finish())
        .child(TuiText::new(" ").truncate().finish())
        .child(counter.finish())
        .finish()
}

#[cfg(test)]
#[path = "warping_indicator_tests.rs"]
mod tests;
