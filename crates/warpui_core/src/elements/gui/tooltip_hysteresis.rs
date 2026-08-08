//! A pure state machine implementing the browser title-tooltip behavior as a
//! *fade animation*, extracted so it can be exhaustively unit tested with a
//! virtual clock, free of any UI framework or async harness.
//!
//! # The model (a fade-animated browser `title` tooltip)
//!
//! Let `D` be a single delay reused for both the fade-in and the fade-out paths.
//!
//! The tooltip's opacity continuously animates toward a *target*:
//!
//! - `1.0` (fully opaque) while the pointer is **at rest** over the target (no
//!   movement beyond a small jitter threshold since the last sample).
//! - `0.0` (fully transparent) while the pointer is **moving** (beyond the
//!   jitter threshold) or has **left** the target.
//!
//! The fade rate is symmetric: a full `0 → 1` fade takes exactly `D` — the
//! fade-in *is* the rest delay, so it begins the instant the pointer comes to
//! rest, with no invisible waiting period. A fade-out from opacity `p` takes
//! `p · D` (same constant rate `1/D`). When the target flips mid-fade, the
//! animation **reverses from the current opacity**, not from a fixed endpoint:
//! nudging the pointer while the tooltip is at 30% starts fading it *out* from
//! 30% (a `0.3 · D` fade-out), and coming to rest again resumes fading *in* from
//! wherever it had reached.
//!
//! The word "rest" is load-bearing: this is not "hovered for `D`". The pointer
//! can sit over the target the entire time and the tooltip still fades away
//! while it keeps moving — exactly like a browser `title` tooltip.
//!
//! # Position
//!
//! The anchor position is **captured when a fade-in starts from zero opacity**
//! (the pointer's rest position). While opacity is `> 0` the position stays
//! fixed — the tooltip never slides. Only once it has fully faded to `0` is the
//! position released, so the next fade-in from zero can capture a fresh spot.
//!
//! This is the single-tooltip-instance consequence of the framework's overlay
//! machinery: the build closure emits at most one positioned overlay child from
//! one [`TooltipState`], so two simultaneously-fading instances at different
//! positions cannot be expressed. If the pointer comes to rest at a new spot
//! while the old tooltip is still mid-fade-out, the old one finishes fading to
//! `0` at its position first, and only then does a fresh fade-in begin at the
//! new rest position.
//!
//! # Exit
//!
//! Leaving the target entirely fades out at a **faster** rate than symmetric
//! (see [`EXIT_FADE_RATE_MULTIPLIER`]) so a dismissed tooltip clears promptly
//! rather than lingering — snappier than the in-target move-to-dismiss, but
//! still animated (not an instant snap, which read as janky in the live build).
//!
//! # Driving the machine
//!
//! Callers feed pointer samples in via [`TooltipHysteresis::on_pointer_moved`]
//! (a move within the target), [`TooltipHysteresis::on_pointer_left`] (exited
//! the target), and read [`TooltipHysteresis::state`] / the opacity at a given
//! `now`. Because the opacity animates continuously, a driver re-samples it each
//! frame while [`TooltipHysteresis::is_animating`] is true (arming a short
//! ~frame-length timer), and stops re-arming once the opacity settles at its
//! target. [`TooltipHysteresis::next_deadline`] reports when the current fade
//! completes, for callers that prefer a single settle timer.
//!
//! The machine holds no real clock: the caller supplies a monotonically
//! non-decreasing `now` on every call. In production `now` is `Instant::now()`;
//! in tests it is a virtual clock the test advances by hand.

use std::time::Duration;

use instant::Instant;

use pathfinder_geometry::vector::Vector2F;

/// The framework's tooltip fade duration `D`, reused across every hover tooltip
/// so they feel uniform. A full `0 → 1` opacity fade takes exactly this long,
/// and it doubles as the fade-out span from full opacity. This matches the
/// hidden-section tooltip's long-standing 500 ms show delay.
pub const TOOLTIP_SHOW_DELAY: Duration = Duration::from_millis(500);

/// Fade duration for pointer-anchored (at-cursor) tooltips. Tighter than
/// [`TOOLTIP_SHOW_DELAY`] because a fade that begins the moment the pointer
/// rests reads sluggish at 500 ms; the hidden-section tooltip keeps the
/// longer delay since it appears without an animation.
pub const TOOLTIP_POINTER_FADE_DELAY: Duration = Duration::from_millis(300);

/// Default jitter tolerance (in pixels) for pointer-hysteresis tooltips: inter-
/// sample movement at or below this counts as the pointer still being at rest,
/// so hand tremor neither fades out nor relocates a visible tooltip. Raised from
/// the original 3 px to absorb a little more real-hand tremor without the
/// tooltip flickering toward a fade-out.
pub const TOOLTIP_JITTER_THRESHOLD: f32 = 6.0;

/// How much faster than the symmetric rate the tooltip fades when the pointer
/// leaves the target entirely. Exit should feel snappier than an in-target
/// move-to-dismiss, but still animate rather than snap. At 3×, a full-opacity
/// tooltip clears in `D / 3`.
pub const EXIT_FADE_RATE_MULTIPLIER: f32 = 3.0;

/// The visible outcome of the fade machine, recomputed for a given `now`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TooltipState {
    /// The tooltip is fully faded out (opacity 0) and not shown.
    Hidden,
    /// The tooltip is shown at `opacity` (in `(0.0, 1.0]`), anchored at `at`
    /// (relative to the target's origin — the same space samples are fed in).
    /// The caller scales the tooltip's colors by `opacity`.
    Visible { at: Vector2F, opacity: f32 },
}

impl TooltipState {
    /// Whether the tooltip is currently shown at all (opacity strictly positive).
    pub fn is_visible(&self) -> bool {
        matches!(self, TooltipState::Visible { .. })
    }

    /// The anchor position if visible.
    pub fn position(&self) -> Option<Vector2F> {
        match self {
            TooltipState::Visible { at, .. } => Some(*at),
            TooltipState::Hidden => None,
        }
    }

    /// The current opacity in `[0.0, 1.0]` (0 when hidden).
    pub fn opacity(&self) -> f32 {
        match self {
            TooltipState::Visible { opacity, .. } => *opacity,
            TooltipState::Hidden => 0.0,
        }
    }
}

/// The fade-animated browser-`title` tooltip machine. See the module docs.
///
/// Opacity is modeled as a piecewise-linear function of time anchored at
/// `(anchor_time, anchor_opacity)` and moving at `rate` (opacity units per
/// second, signed toward `target`). Sampling clamps the linear extrapolation
/// into `[0, target]` (for a rising fade) or `[target, anchor]` (for a falling
/// one), so the value never overshoots its target.
#[derive(Clone, Debug)]
pub struct TooltipHysteresis {
    /// The symmetric fade duration `D`: a full `0 → 1` fade (and a full `1 → 0`
    /// in-target fade) takes this long.
    delay: Duration,
    /// Movement strictly beyond this many pixels between two samples counts as
    /// motion; at or below it, the pointer is treated as still (jitter).
    jitter_threshold: f32,

    /// Opacity value at `anchor_time`, the start of the current linear segment.
    anchor_opacity: f32,
    /// The instant `anchor_opacity` was captured; the segment's origin.
    anchor_time: Instant,
    /// Signed opacity-per-second slope of the current segment (positive fading
    /// in, negative fading out, zero when settled at the target).
    rate: f32,
    /// The opacity the current segment is heading toward (`0.0` or `1.0`).
    target: f32,

    /// The anchor position, captured when a fade-in began from zero opacity and
    /// held fixed until the tooltip fully fades out. [`None`] only while fully
    /// hidden with nothing fading.
    position: Option<Vector2F>,

    /// The most recent pointer position sampled while over the target, used to
    /// measure inter-sample movement for jitter rejection. [`None`] before the
    /// first sample and after the pointer leaves.
    last_position: Option<Vector2F>,
}

impl TooltipHysteresis {
    /// Creates a machine fully hidden (opacity 0), with no position captured.
    ///
    /// `delay` is `D` (the symmetric fade duration). `jitter_threshold` is the
    /// pixel radius at/below which inter-sample movement counts as rest.
    pub fn new(delay: Duration, jitter_threshold: f32) -> Self {
        Self {
            delay,
            jitter_threshold,
            anchor_opacity: 0.0,
            anchor_time: Instant::now(),
            rate: 0.0,
            target: 0.0,
            position: None,
            last_position: None,
        }
    }

    /// The symmetric fade rate, in opacity units per second (`1 / D`).
    fn base_rate(&self) -> f32 {
        1.0 / self.delay.as_secs_f32()
    }

    /// The opacity at `now`, clamping the current linear segment so it never
    /// overshoots its target. Pure: does not mutate the machine.
    fn opacity_at(&self, now: Instant) -> f32 {
        let elapsed = now
            .saturating_duration_since(self.anchor_time)
            .as_secs_f32();
        let raw = self.anchor_opacity + self.rate * elapsed;
        // Clamp into the interval bounded by the anchor and the target, so a
        // matured fade sits exactly at its target rather than running past it.
        let (lo, hi) = if self.anchor_opacity <= self.target {
            (self.anchor_opacity, self.target)
        } else {
            (self.target, self.anchor_opacity)
        };
        raw.clamp(lo, hi)
    }

    /// The current externally observable state at `now`.
    pub fn state(&self, now: Instant) -> TooltipState {
        let opacity = self.opacity_at(now);
        match self.position {
            Some(at) if opacity > 0.0 => TooltipState::Visible { at, opacity },
            _ => TooltipState::Hidden,
        }
    }

    /// The current opacity at `now`, in `[0.0, 1.0]`.
    pub fn opacity(&self, now: Instant) -> f32 {
        self.opacity_at(now)
    }

    /// Whether a fade is in progress at `now` (opacity not yet settled at its
    /// target). A driver re-samples each frame while this is true.
    pub fn is_animating(&self, now: Instant) -> bool {
        (self.opacity_at(now) - self.target).abs() > f32::EPSILON
    }

    /// The instant the current fade completes (opacity reaches its target), if a
    /// fade is in progress; [`None`] when already settled. A caller can use this
    /// to schedule a single settle timer instead of polling every frame.
    pub fn next_deadline(&self) -> Option<Instant> {
        if self.rate == 0.0 || self.anchor_opacity == self.target {
            return None;
        }
        let remaining = (self.target - self.anchor_opacity) / self.rate;
        if remaining <= 0.0 {
            return None;
        }
        Some(self.anchor_time + Duration::from_secs_f32(remaining))
    }

    /// Whether `to` is within the jitter threshold of the last sampled position
    /// (i.e. the pointer is effectively still). The first sample after the
    /// pointer arrives has no predecessor and so counts as movement, correctly
    /// leaving the tooltip faded out until the *next* (resting) sample.
    fn is_jitter(&self, to: Vector2F) -> bool {
        match self.last_position {
            Some(from) => (to - from).length() <= self.jitter_threshold,
            None => false,
        }
    }

    /// Re-anchor the animation to the current opacity at `now` and aim it at
    /// `target` with slope `rate` (magnitude, always positive; sign is derived
    /// from the direction to the target). A no-op-preserving helper: sampling at
    /// `now` immediately after yields the same opacity it did just before.
    fn retarget(&mut self, now: Instant, target: f32, rate_magnitude: f32) {
        let current = self.opacity_at(now);
        self.anchor_opacity = current;
        self.anchor_time = now;
        self.target = target;
        self.rate = if target >= current {
            rate_magnitude
        } else {
            -rate_magnitude
        };
    }

    /// Release the captured position if the tooltip has fully faded out, so the
    /// next fade-in from zero can capture a fresh rest position. Called after
    /// sampling so a matured fade-out drops its anchor.
    fn release_position_if_hidden(&mut self, now: Instant) {
        if self.opacity_at(now) <= 0.0 && self.target <= 0.0 {
            self.position = None;
        }
    }

    /// Feed a pointer sample taken while the pointer is over the target, at
    /// position `at` (relative to the target's origin) at time `now`.
    ///
    /// Movement at or within the jitter threshold of the previous sample is
    /// treated as rest: the fade heads toward full opacity. Real movement aims
    /// the fade toward zero. The animation reverses from the *current* opacity,
    /// so quick rest/move alternations don't snap.
    pub fn on_pointer_moved(&mut self, at: Vector2F, now: Instant) -> TooltipState {
        let jitter = self.is_jitter(at);
        let had_predecessor = self.last_position.is_some();
        self.last_position = Some(at);

        // "At rest" = a jitter sample (held still since the last one). The very
        // first sample after arrival has no predecessor and so is motion, which
        // keeps the tooltip faded out until the pointer actually holds still.
        let at_rest = jitter && had_predecessor;

        if at_rest {
            self.rest_at(at, now);
        } else {
            // Real motion (or first-sample arrival): fade toward hidden. Keep the
            // captured position pinned so the fading-out tooltip does not slide.
            self.retarget(now, 0.0, self.base_rate());
        }

        self.release_position_if_hidden(now);
        self.state(now)
    }

    /// Note that the pointer has come to rest at `at` at time `now` (an explicit
    /// "settled" signal, e.g. from a rest-detection timer). Aims the fade toward
    /// full opacity when resting at (or near) the captured position, or lets an
    /// old tooltip finish fading out before capturing a new spot; see [`Self::rest_at`].
    pub fn on_pointer_rested(&mut self, at: Vector2F, now: Instant) -> TooltipState {
        self.last_position = Some(at);
        self.rest_at(at, now);
        self.release_position_if_hidden(now);
        self.state(now)
    }

    /// Shared "the pointer is at rest at `at`" handling, honoring the
    /// single-instance relocation rule:
    ///
    /// - Fully hidden (opacity 0): capture `at` and fade in from zero.
    /// - Visible at (or within jitter of) the captured position: reverse toward
    ///   full opacity in place — this is a re-rest at the same spot.
    /// - Visible but `at` is a *different* spot: do **not** reverse; keep the
    ///   fade heading toward zero so the old tooltip finishes fading out at its
    ///   position, after which a later rest sample (now from zero) captures the
    ///   new spot. This is the single-tooltip-instance consequence — two
    ///   simultaneously-visible instances at different positions can't be shown.
    fn rest_at(&mut self, at: Vector2F, now: Instant) {
        let opacity = self.opacity_at(now);
        if opacity <= 0.0 {
            // Fade in from zero at the fresh rest position.
            self.position = Some(at);
            self.retarget(now, 1.0, self.base_rate());
            return;
        }

        // Visible: reverse toward full only if resting at the pinned position.
        let same_spot = self
            .position
            .is_some_and(|pinned| (at - pinned).length() <= self.jitter_threshold);
        if same_spot {
            self.retarget(now, 1.0, self.base_rate());
        } else {
            // Rest at a new spot while the old is still visible: let it finish
            // fading out first (do not reverse, do not slide).
            self.retarget(now, 0.0, self.base_rate());
        }
    }

    /// The pointer has left the target entirely: fade toward hidden at the faster
    /// exit rate ([`EXIT_FADE_RATE_MULTIPLIER`]×), reversing from the current
    /// opacity, and forget the last sampled position. The captured anchor
    /// position stays pinned until the fade-out completes.
    pub fn on_pointer_left(&mut self, now: Instant) -> TooltipState {
        self.last_position = None;
        self.retarget(now, 0.0, self.base_rate() * EXIT_FADE_RATE_MULTIPLIER);
        self.release_position_if_hidden(now);
        self.state(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Duration = Duration::from_millis(500);
    const JITTER: f32 = 6.0;

    fn machine() -> TooltipHysteresis {
        TooltipHysteresis::new(D, JITTER)
    }

    fn at(x: f32, y: f32) -> Vector2F {
        Vector2F::new(x, y)
    }

    /// Opacity helper: assert two f32s are within a small epsilon. The tolerance
    /// absorbs the float rounding of `Duration::as_secs_f32` scaled by the rate
    /// (a 500 ms fade lands within a few thousandths of its target).
    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 5e-3, "expected {a} ≈ {b}");
    }

    /// The fade-in *is* the rest delay: opacity rises linearly from 0 the instant
    /// the pointer comes to rest, reaching full at exactly `D` — no invisible
    /// waiting period first.
    #[test]
    fn fade_in_is_the_rest_delay_monotonic_from_zero() {
        let mut m = machine();
        let t0 = Instant::now();

        // Arrival is motion (no predecessor): still hidden, target 0.
        assert_eq!(m.on_pointer_moved(at(10., 20.), t0), TooltipState::Hidden);
        // Hold still: the pointer is now at rest and the fade-in begins at once
        // (opacity is exactly 0 at the retarget instant, then rises immediately).
        let t_rest = t0 + Duration::from_millis(1);
        m.on_pointer_moved(at(11., 21.), t_rest);
        // A hair after the rest instant, opacity is already positive — no
        // invisible waiting period before the fade begins.
        assert!(
            m.opacity(t_rest + Duration::from_millis(1)) > 0.0,
            "fade-in begins immediately on rest, not after a delay"
        );

        // Monotonically rising toward full over D, measured from the rest instant.
        approx(m.opacity(t_rest + Duration::from_millis(125)), 0.25);
        approx(m.opacity(t_rest + Duration::from_millis(250)), 0.50);
        approx(m.opacity(t_rest + Duration::from_millis(500)), 1.0);
        // Clamped at full past the deadline.
        approx(m.opacity(t_rest + Duration::from_millis(900)), 1.0);
        // The captured anchor is the pointer's rest position (11,21).
        assert_eq!(
            m.state(t_rest + Duration::from_millis(500)).position(),
            Some(at(11., 21.))
        );
        approx(m.state(t_rest + Duration::from_millis(500)).opacity(), 1.0);
    }

    /// The position is captured at the pointer's rest spot when the fade-in
    /// starts from zero, and stays fixed while opacity > 0 — no sliding.
    #[test]
    fn position_captured_at_zero_start_and_pinned_while_visible() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        // Rest: captures the rest-sample position (12,12) as the anchor and begins
        // fading in (the capture is where the pointer is seen to hold still).
        let t_rest = t0 + Duration::from_millis(1);
        m.on_pointer_moved(at(12., 12.), t_rest);
        approx(m.opacity(t_rest + Duration::from_millis(250)), 0.5);
        // Another rest sample nearby (within jitter) must NOT move the anchor.
        let s = m.on_pointer_moved(at(14., 14.), t_rest + Duration::from_millis(300));
        assert_eq!(
            s.position(),
            Some(at(12., 12.)),
            "anchor pinned while visible"
        );
    }

    /// Fade-out from full opacity takes the full `D` (`p·D` with `p = 1`), rising
    /// then falling symmetrically.
    #[test]
    fn fade_out_from_full_takes_full_delay() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        m.on_pointer_moved(at(11., 11.), t0 + Duration::from_millis(1)); // rest → fade in
        approx(m.opacity(t0 + D), 1.0);

        // Real move at full opacity → fade out begins from 1.0.
        let t_move = t0 + D;
        m.on_pointer_moved(at(60., 60.), t_move);
        approx(m.opacity(t_move + Duration::from_millis(250)), 0.5);
        approx(m.opacity(t_move + D), 0.0);
        assert_eq!(m.state(t_move + D), TooltipState::Hidden);
    }

    /// Fade-out from a *partial* opacity `p` takes `p·D`: moving while the
    /// tooltip is only 40% faded-in clears it in `0.4·D`, reversing from 0.4 (not
    /// from 1.0).
    #[test]
    fn fade_out_from_partial_takes_p_times_delay_reversing_from_current() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        m.on_pointer_moved(at(11., 11.), t0 + Duration::from_millis(1)); // rest → fade in
        // At +200ms, opacity ≈ 0.4.
        let t_move = t0 + Duration::from_millis(201);
        approx(m.opacity(t_move), 0.4);

        // Move → reverse from 0.4 toward 0 at the same rate: reaches 0 in 0.4·D = 200ms.
        m.on_pointer_moved(at(80., 80.), t_move);
        approx(m.opacity(t_move), 0.4);
        approx(m.opacity(t_move + Duration::from_millis(100)), 0.2);
        approx(m.opacity(t_move + Duration::from_millis(200)), 0.0);
        assert_eq!(
            m.state(t_move + Duration::from_millis(200)),
            TooltipState::Hidden
        );
    }

    /// Re-resting at the *same* captured position mid-fade-out reverses back
    /// toward full from the current opacity, keeping the pinned position (a
    /// jitter-scale wobble that resolves as staying put).
    #[test]
    fn rest_at_same_spot_mid_fade_out_reverses_from_current() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        // The rest sample (11,11) is captured as the pinned anchor.
        let t_rest0 = t0 + Duration::from_millis(1);
        m.on_pointer_moved(at(11., 11.), t_rest0); // rest → fade in
        approx(m.opacity(t_rest0 + D), 1.0);

        // A brief wobble beyond jitter starts a fade-out (target 0).
        let t_move = t_rest0 + D;
        m.on_pointer_moved(at(21., 11.), t_move); // 10px from (11,11) → motion
        approx(m.opacity(t_move + Duration::from_millis(250)), 0.5);

        // Rest again back at the pinned spot (within jitter of (11,11)) at +250ms
        // (opacity 0.5) → reverse toward full from 0.5, same position.
        let t_rest = t_move + Duration::from_millis(250);
        let s = m.on_pointer_rested(at(11., 11.), t_rest);
        approx(s.opacity(), 0.5);
        assert_eq!(
            s.position(),
            Some(at(11., 11.)),
            "position stays pinned while visible"
        );
        // Rises back to full over the next 0.5·D = 250ms.
        approx(m.opacity(t_rest + Duration::from_millis(250)), 1.0);
    }

    /// Single-instance relocation: resting at a *new* spot while the old tooltip
    /// is still visible does NOT reverse it in place — the old finishes fading
    /// out at its position, and only a rest sample taken once fully hidden
    /// captures the new spot and fades in there.
    #[test]
    fn rest_at_new_spot_while_visible_defers_until_faded_out() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        // The rest sample (11,11) is captured as the pinned anchor.
        let t_rest0 = t0 + Duration::from_millis(1);
        m.on_pointer_moved(at(11., 11.), t_rest0); // rest at (11,11)
        approx(m.opacity(t_rest0 + D), 1.0);

        // Move to a far spot, then hold still there while the old is still up.
        let t_move = t_rest0 + D;
        m.on_pointer_moved(at(200., 200.), t_move); // motion → fade out from full
        // Hold still at the new spot mid-fade-out: must NOT reverse (old still
        // pinned at (11,11)); keeps fading out toward 0.
        let s = m.on_pointer_moved(at(201., 201.), t_move + Duration::from_millis(100));
        assert_eq!(
            s.position(),
            Some(at(11., 11.)),
            "old position pinned, not relocated yet"
        );
        assert!(s.opacity() < 1.0, "still fading out, not reversed");

        // Let it fully fade out.
        assert_eq!(m.state(t_move + D), TooltipState::Hidden);

        // Now (fully hidden) a rest sample at the new spot captures it and fades in.
        let t_new = t_move + D;
        m.on_pointer_moved(at(202., 202.), t_new + Duration::from_millis(1)); // rest (jitter vs 201) → capture
        let s = m.state(t_new + Duration::from_millis(2));
        assert_eq!(
            s.position(),
            Some(at(202., 202.)),
            "new position captured after full fade-out"
        );
        assert!(s.opacity() > 0.0, "fading in at the new spot");
    }

    /// Leaving the target fades out faster than symmetric (3× rate): a
    /// full-opacity tooltip clears in `D / 3`, still animated (not instant).
    #[test]
    fn exit_fades_out_at_faster_rate() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        m.on_pointer_moved(at(11., 11.), t0 + Duration::from_millis(1)); // rest → fade in
        approx(m.opacity(t0 + D), 1.0);

        // Leave: fade out at 3× → reaches 0 in D/3 ≈ 166.7ms.
        let t_left = t0 + D;
        let s = m.on_pointer_left(t_left);
        approx(s.opacity(), 1.0);
        approx(m.opacity(t_left + Duration::from_millis(83)), 0.5);
        approx(m.opacity(t_left + Duration::from_millis(167)), 0.0);
        assert_eq!(
            m.state(t_left + Duration::from_millis(167)),
            TooltipState::Hidden
        );
    }

    /// Sub-threshold movement (≤ jitter) counts as rest, so a visible tooltip
    /// keeps fading toward full and is not relocated by hand-tremor samples. The
    /// 6px threshold tolerates a larger tremor than the original 3px.
    #[test]
    fn jitter_within_six_px_counts_as_rest() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        // The rest sample (11,11) is captured as the pinned anchor.
        let t_rest0 = t0 + Duration::from_millis(1);
        m.on_pointer_moved(at(11., 11.), t_rest0); // rest → fade in
        approx(m.opacity(t_rest0 + D), 1.0);

        // A ~4.5px move (≤ 6px threshold) is jitter: stays at rest (target 1),
        // not faded out, anchor unchanged at the pinned (11,11).
        let s = m.on_pointer_moved(at(15., 13.), t_rest0 + D + Duration::from_millis(50)); // dist=~4.47
        assert_eq!(s.position(), Some(at(11., 11.)));
        approx(s.opacity(), 1.0);

        // A ~7px move (> 6px) IS motion: begins a fade-out.
        let t_move = t_rest0 + D + Duration::from_millis(100);
        let s = m.on_pointer_moved(at(20., 18.), t_move); // dist from (15,13) = ~7.07
        approx(s.opacity(), 1.0);
        assert!(
            m.opacity(t_move + Duration::from_millis(250)) < 1.0,
            "fading out after real move"
        );
    }

    /// `on_pointer_rested` on a hidden machine begins a fade-in from zero (does
    /// not snap to full), capturing the rest position.
    #[test]
    fn explicit_rest_while_hidden_begins_fade_in() {
        let mut m = machine();
        let t0 = Instant::now();
        let s = m.on_pointer_rested(at(5., 5.), t0);
        approx(s.opacity(), 0.0);
        approx(m.opacity(t0 + Duration::from_millis(250)), 0.5);
        let s = m.state(t0 + D);
        assert_eq!(
            s,
            TooltipState::Visible {
                at: at(5., 5.),
                opacity: 1.0
            }
        );
    }

    /// `next_deadline` reports when the current fade completes, and nothing once
    /// settled at the target, so a driver can schedule exactly one settle timer.
    #[test]
    fn next_deadline_reports_fade_completion() {
        let mut m = machine();
        let t0 = Instant::now();
        assert_eq!(m.next_deadline(), None, "settled hidden: no fade");

        m.on_pointer_moved(at(0., 0.), t0);
        let t_rest = t0 + Duration::from_millis(1);
        m.on_pointer_moved(at(1., 1.), t_rest); // rest → fade in over D
        let deadline = m.next_deadline().expect("fade in progress");
        // Completes ~D after the rest sample.
        let expected = t_rest + D;
        assert!(
            deadline.saturating_duration_since(expected) < Duration::from_millis(2)
                && expected.saturating_duration_since(deadline) < Duration::from_millis(2),
            "fade-in completion deadline"
        );

        // Once fully faded in, no deadline.
        // (Sample past the deadline re-anchors nothing; next_deadline is derived
        // from the segment, which still points at target 1 with anchor < 1 until
        // re-sampled — so advance via a rest sample at full to settle it.)
        m.on_pointer_moved(at(2., 2.), t_rest + D); // still rest (jitter) at full
        assert_eq!(
            m.next_deadline(),
            None,
            "settled at full opacity: no further fade"
        );
    }

    /// `is_animating` is true during a fade and false once settled at the target.
    #[test]
    fn is_animating_tracks_fade_progress() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(0., 0.), t0);
        let t_rest = t0 + Duration::from_millis(1);
        m.on_pointer_moved(at(1., 1.), t_rest); // fade in
        assert!(
            m.is_animating(t_rest + Duration::from_millis(100)),
            "mid fade-in"
        );
        assert!(!m.is_animating(t_rest + D), "settled at full");
    }
}
