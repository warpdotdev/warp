//! A pure state machine implementing the browser title-tooltip behavior with
//! symmetric show/dismiss hysteresis, extracted so it can be exhaustively unit
//! tested with a virtual clock, free of any UI framework or async harness.
//!
//! # The model (matching a browser's `title` tooltip)
//!
//! Let `D` be a single delay reused for both the show and the dismiss paths.
//!
//! - The pointer coming to **rest** over the target (no movement beyond a small
//!   jitter threshold for `D`) shows the tooltip at the pointer position.
//! - Once visible, the pointer **moving** starts a dismissal timer of the same
//!   `D`. If the pointer is still moving when it fires, the tooltip dismisses.
//! - If the pointer comes to **rest again before** that dismissal fires, the
//!   dismissal is cancelled and the *visible* tooltip is **relocated** to the
//!   new pointer position with no additional show delay (it was already paid).
//! - Resting again *after* a dismissal takes the normal show path (delay `D`,
//!   appear at the new position).
//! - The pointer **leaving** the target dismisses immediately, with no delay.
//!
//! The word "rest" is load-bearing: this is not "hovered for `D`". The pointer
//! can sit over the target the entire time and the tooltip still hides while it
//! keeps moving — exactly like a browser `title` tooltip.
//!
//! # Driving the machine
//!
//! Callers feed pointer samples in via [`TooltipHysteresis::on_pointer_moved`]
//! (a move within the target), [`TooltipHysteresis::on_pointer_left`] (exited
//! the target), and call [`TooltipHysteresis::tick`] whenever the clock may have
//! advanced far enough to cross a pending deadline (e.g. from a timer callback,
//! or on the next re-dispatch of the last pointer sample). Every method returns
//! the current [`TooltipState`] so the caller can rebuild only when it changes.
//!
//! The machine holds no real clock: the caller supplies a monotonically
//! non-decreasing `now` on every call. In production `now` is `Instant::now()`;
//! in tests it is a virtual clock the test advances by hand.

use std::time::Duration;

use instant::Instant;

use pathfinder_geometry::vector::Vector2F;

/// The framework's tooltip show delay `D`, reused across every hover tooltip so
/// they feel uniform. Also the dismiss delay for the browser-`title` hysteresis
/// model (a single `D` governs both the rest-to-show and move-to-dismiss paths).
/// This matches the hidden-section tooltip's long-standing 500 ms show delay.
pub const TOOLTIP_SHOW_DELAY: Duration = Duration::from_millis(500);

/// Default jitter tolerance (in pixels) for pointer-hysteresis tooltips: inter-
/// sample movement at or below this counts as the pointer still being at rest,
/// so hand tremor neither dismisses nor relocates a visible tooltip.
pub const TOOLTIP_JITTER_THRESHOLD: f32 = 3.0;

/// The visible outcome of the hysteresis machine, recomputed after every input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TooltipState {
    /// The tooltip is not shown.
    Hidden,
    /// The tooltip is shown, anchored at this pointer position (relative to the
    /// target's origin — the same space the samples are fed in).
    Visible { at: Vector2F },
}

impl TooltipState {
    /// Whether the tooltip is currently shown.
    pub fn is_visible(&self) -> bool {
        matches!(self, TooltipState::Visible { .. })
    }

    /// The anchor position if visible.
    pub fn position(&self) -> Option<Vector2F> {
        match self {
            TooltipState::Visible { at } => Some(*at),
            TooltipState::Hidden => None,
        }
    }
}

/// Internal phase tracking, distinct from the externally observable
/// [`TooltipState`] because "hidden" splits into "idle" vs "waiting to show" and
/// "visible" splits into "steady" vs "waiting to dismiss".
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Pointer is over the target but has not yet come to rest, and nothing is
    /// shown. This is the state after a dismissal while the pointer keeps
    /// moving, and the initial state.
    Idle,
    /// Pointer came to rest at `at`; the tooltip will show when `deadline` is
    /// reached (provided the pointer has not moved again since).
    PendingShow { at: Vector2F, deadline: Instant },
    /// Tooltip is visible and anchored at `at`; the pointer is at rest.
    Visible { at: Vector2F },
    /// Tooltip is visible and anchored at `at`, but the pointer has started
    /// moving; it will dismiss when `deadline` is reached unless the pointer
    /// comes to rest again first.
    PendingDismiss { at: Vector2F, deadline: Instant },
}

/// The browser-`title` tooltip hysteresis state machine. See the module docs.
#[derive(Clone, Debug)]
pub struct TooltipHysteresis {
    /// The single delay `D` reused for the show and dismiss paths.
    delay: std::time::Duration,
    /// Movement below this many pixels between two samples is treated as jitter
    /// (i.e. still "at rest"), not motion.
    jitter_threshold: f32,
    phase: Phase,
    /// The most recent pointer position sampled while over the target, used to
    /// measure inter-sample movement for jitter rejection. [`None`] before the
    /// first sample and after the pointer leaves.
    last_position: Option<Vector2F>,
}

impl TooltipHysteresis {
    /// Creates a machine in the [`Phase::Idle`] state.
    ///
    /// `delay` is `D` (reused for both show and dismiss). `jitter_threshold` is
    /// the pixel radius below which movement counts as rest.
    pub fn new(delay: std::time::Duration, jitter_threshold: f32) -> Self {
        Self {
            delay,
            jitter_threshold,
            phase: Phase::Idle,
            last_position: None,
        }
    }

    /// The current externally observable state.
    pub fn state(&self) -> TooltipState {
        match self.phase {
            Phase::Idle | Phase::PendingShow { .. } => TooltipState::Hidden,
            Phase::Visible { at } | Phase::PendingDismiss { at, .. } => {
                TooltipState::Visible { at }
            }
        }
    }

    /// The next instant at which [`Self::tick`] would change state, if any. A
    /// caller can use this to schedule a single timer rather than polling.
    pub fn next_deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::PendingShow { deadline, .. } | Phase::PendingDismiss { deadline, .. } => {
                Some(deadline)
            }
            Phase::Idle | Phase::Visible { .. } => None,
        }
    }

    /// Whether `to` is within the jitter threshold of the last sampled position
    /// (i.e. the pointer is effectively still). The first sample after the
    /// pointer arrives has no predecessor and so counts as movement, which
    /// correctly starts the initial rest timer via [`Self::on_pointer_moved`].
    fn is_jitter(&self, to: Vector2F) -> bool {
        match self.last_position {
            Some(from) => (to - from).length() <= self.jitter_threshold,
            None => false,
        }
    }

    /// Feed a pointer sample taken while the pointer is over the target, at
    /// position `at` (relative to the target's origin) at time `now`.
    ///
    /// Movement within the jitter threshold of the previous sample is treated as
    /// no movement: the pointer is still at rest, so a pending show/relocate is
    /// left to mature rather than being restarted. Real movement (re)arms the
    /// rest timer while hidden, and starts/keeps the dismissal timer while
    /// visible.
    pub fn on_pointer_moved(&mut self, at: Vector2F, now: Instant) -> TooltipState {
        // First, let any already-elapsed deadline resolve, so a sample that
        // arrives after the deadline still shows/dismisses deterministically.
        self.tick(now);

        let jitter = self.is_jitter(at);
        let previous = self.last_position;
        self.last_position = Some(at);

        if jitter {
            // The pointer held still since the last sample: it has come to rest.
            // While counting down to dismiss, that re-rest cancels the dismissal
            // and relocates the still-visible tooltip to the rest position with no
            // new delay (spec: rest again before dismissal → relocate). This is
            // how the re-rest is detected in production: the window re-dispatches
            // the last move on each redraw, and two same-position samples in a row
            // read as "stopped here".
            //
            // A jitter sample with no predecessor cannot happen (`is_jitter` is
            // false without one), but guard anyway.
            if previous.is_some()
                && let Phase::PendingDismiss { .. } = self.phase
            {
                self.phase = Phase::Visible { at };
            }
            // Otherwise the pointer is still at rest in a phase whose pending
            // deadline should simply mature (PendingShow) or that is already
            // steady (Visible/Idle); leave it for `tick`.
            return self.state();
        }

        // Real movement.
        self.phase = match self.phase {
            // Hidden and moving: (re)start the rest timer. When the pointer next
            // holds still, `tick` after `delay` will show at that held position.
            // We keep re-arming to the *latest* position so the tooltip shows
            // where the pointer actually came to rest.
            Phase::Idle | Phase::PendingShow { .. } => Phase::PendingShow {
                at,
                deadline: now + self.delay,
            },
            // Visible and starting/continuing to move: arm (or keep) the
            // dismissal timer. Keep the anchor pinned where the tooltip is now;
            // it must not follow the pointer while dismissing.
            Phase::Visible { at: shown_at } => Phase::PendingDismiss {
                at: shown_at,
                deadline: now + self.delay,
            },
            // Already counting down to dismiss and still moving: let the
            // existing deadline stand (do not extend it on every move — a moving
            // pointer should dismiss `delay` after motion *began*).
            Phase::PendingDismiss {
                at: shown_at,
                deadline,
            } => Phase::PendingDismiss {
                at: shown_at,
                deadline,
            },
        };
        self.state()
    }

    /// Advance the machine to time `now`, resolving any pending show/dismiss
    /// whose deadline has been reached. Idempotent and safe to call at any time.
    ///
    /// A `PendingShow` that matures becomes `Visible` at its resting position. A
    /// `PendingDismiss` that matures becomes `Idle` (hidden) — the pointer was
    /// still moving when the timer fired.
    pub fn tick(&mut self, now: Instant) -> TooltipState {
        self.phase = match self.phase {
            Phase::PendingShow { at, deadline } if now >= deadline => Phase::Visible { at },
            Phase::PendingDismiss { deadline, .. } if now >= deadline => Phase::Idle,
            other => other,
        };
        self.state()
    }

    /// Note that the pointer has come to rest at `at` at time `now` (an explicit
    /// "settled" signal, e.g. from a rest-detection timer). This is a
    /// convenience over [`Self::on_pointer_moved`] for callers that detect rest
    /// out-of-band: it relocates a visible tooltip immediately (cancelling any
    /// dismissal, no new delay) and, if hidden, arms the show timer.
    pub fn on_pointer_rested(&mut self, at: Vector2F, now: Instant) -> TooltipState {
        self.tick(now);
        self.last_position = Some(at);
        self.phase = match self.phase {
            // Rest before the dismissal fired: cancel it and relocate the
            // already-visible tooltip. No new show delay — it was already paid.
            Phase::PendingDismiss { .. } | Phase::Visible { .. } => Phase::Visible { at },
            // Hidden: normal show path. Arm the show timer at the resting spot.
            Phase::Idle | Phase::PendingShow { .. } => Phase::PendingShow {
                at,
                deadline: now + self.delay,
            },
        };
        self.state()
    }

    /// The pointer has left the target entirely: dismiss immediately, with no
    /// delay, and forget any pending timers and the last sampled position.
    pub fn on_pointer_left(&mut self) -> TooltipState {
        self.phase = Phase::Idle;
        self.last_position = None;
        self.state()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Duration = Duration::from_millis(500);
    const JITTER: f32 = 3.0;

    fn machine() -> TooltipHysteresis {
        TooltipHysteresis::new(D, JITTER)
    }

    fn at(x: f32, y: f32) -> Vector2F {
        Vector2F::new(x, y)
    }

    /// Spec point 2: pointer comes to rest over the target; after D the tooltip
    /// appears at the pointer position.
    #[test]
    fn rest_shows_after_delay_at_pointer() {
        let mut m = machine();
        let t0 = Instant::now();

        // Arrival is "movement" (no predecessor) → arms the show timer.
        assert_eq!(m.on_pointer_moved(at(10., 20.), t0), TooltipState::Hidden);
        // Held still (jitter) before the deadline: still hidden.
        assert_eq!(
            m.on_pointer_moved(at(11., 21.), t0 + Duration::from_millis(100)),
            TooltipState::Hidden
        );
        // Just before the deadline: still hidden.
        assert_eq!(
            m.tick(t0 + Duration::from_millis(499)),
            TooltipState::Hidden
        );
        // At the deadline: shows at the resting position.
        assert_eq!(m.tick(t0 + D), TooltipState::Visible { at: at(10., 20.) });
    }

    /// The show anchors at where the pointer actually came to rest, not where it
    /// first entered — re-arming tracks the latest resting spot.
    #[test]
    fn show_anchors_at_final_resting_position() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        // Real move to a new spot re-arms the timer at the new position.
        m.on_pointer_moved(at(50., 60.), t0 + Duration::from_millis(200));
        // Not yet D since the *second* rest began.
        assert_eq!(
            m.tick(t0 + Duration::from_millis(600)),
            TooltipState::Hidden
        );
        assert_eq!(
            m.tick(t0 + Duration::from_millis(700)),
            TooltipState::Visible { at: at(50., 60.) }
        );
    }

    /// Spec point 4: pointer moves while visible, then comes to rest again
    /// before the dismissal timer fires → dismissal cancelled and the tooltip
    /// relocates to the new position with no additional delay.
    #[test]
    fn rest_before_dismissal_relocates_without_delay() {
        let mut m = machine();
        let t0 = Instant::now();
        // Show it.
        m.on_pointer_moved(at(10., 10.), t0);
        assert!(m.tick(t0 + D).is_visible());

        // Move → arms dismissal at t = D + 300 + D.
        let t_move = t0 + D + Duration::from_millis(300);
        assert_eq!(
            m.on_pointer_moved(at(80., 90.), t_move),
            TooltipState::Visible { at: at(10., 10.) },
            "anchor stays pinned while dismissing"
        );

        // Rest again before the dismissal deadline: relocate immediately.
        let t_rest = t_move + Duration::from_millis(200);
        assert_eq!(
            m.on_pointer_rested(at(80., 90.), t_rest),
            TooltipState::Visible { at: at(80., 90.) },
            "relocated with no new delay"
        );
        // The prior dismissal deadline must not fire anymore.
        assert_eq!(
            m.tick(t_move + D + Duration::from_millis(1)),
            TooltipState::Visible { at: at(80., 90.) }
        );
    }

    /// Spec point 3: pointer keeps moving while visible; the dismissal timer
    /// completes and the tooltip dismisses.
    #[test]
    fn sustained_movement_dismisses_after_delay() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        assert!(m.tick(t0 + D).is_visible());

        // Start moving → arms dismissal D from now.
        let t_move = t0 + D + Duration::from_millis(100);
        m.on_pointer_moved(at(40., 40.), t_move);
        // Keep moving; deadline should not extend on each move.
        m.on_pointer_moved(at(70., 70.), t_move + Duration::from_millis(200));
        m.on_pointer_moved(at(100., 100.), t_move + Duration::from_millis(400));
        // Just before dismissal deadline (t_move + D): still visible.
        assert!(
            m.tick(t_move + D - Duration::from_millis(1)).is_visible(),
            "still visible just before dismissal"
        );
        // At the deadline: dismissed.
        assert_eq!(m.tick(t_move + D), TooltipState::Hidden);
    }

    /// A dismissal deadline is measured from when motion *began*, not extended by
    /// each subsequent move — a continuously-moving pointer dismisses on schedule.
    #[test]
    fn dismissal_deadline_not_extended_by_continued_motion() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(0., 0.), t0);
        assert!(m.tick(t0 + D).is_visible());

        let t_move = t0 + D;
        m.on_pointer_moved(at(20., 0.), t_move);
        // A later move well before the deadline must not push it out.
        m.on_pointer_moved(at(40., 0.), t_move + Duration::from_millis(499));
        assert_eq!(m.tick(t_move + D), TooltipState::Hidden);
    }

    /// Spec point 6: leaving the target dismisses immediately, no delay, from any
    /// visible or pending state.
    #[test]
    fn leaving_dismisses_immediately() {
        // From Visible.
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        assert!(m.tick(t0 + D).is_visible());
        assert_eq!(m.on_pointer_left(), TooltipState::Hidden);

        // From PendingShow.
        let mut m = machine();
        m.on_pointer_moved(at(10., 10.), t0);
        assert_eq!(m.on_pointer_left(), TooltipState::Hidden);

        // From PendingDismiss.
        let mut m = machine();
        m.on_pointer_moved(at(10., 10.), t0);
        m.tick(t0 + D);
        m.on_pointer_moved(at(40., 40.), t0 + D + Duration::from_millis(50));
        assert_eq!(m.on_pointer_left(), TooltipState::Hidden);
    }

    /// Spec point 5: after a dismissal, resting again takes the normal show path
    /// (full delay D, appears at the new position).
    #[test]
    fn rest_after_dismissal_uses_full_show_delay() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        assert!(m.tick(t0 + D).is_visible());

        // Dismiss via sustained movement.
        let t_move = t0 + D;
        m.on_pointer_moved(at(40., 40.), t_move);
        assert_eq!(m.tick(t_move + D), TooltipState::Hidden);

        // Rest at a new spot: must take the full delay again.
        let t_rest = t_move + D + Duration::from_millis(100);
        m.on_pointer_moved(at(200., 200.), t_rest);
        assert_eq!(
            m.tick(t_rest + D - Duration::from_millis(1)),
            TooltipState::Hidden,
            "must pay the full show delay after a dismissal"
        );
        assert_eq!(
            m.tick(t_rest + D),
            TooltipState::Visible { at: at(200., 200.) }
        );
    }

    /// Sub-jitter-threshold movement does not count as motion: a visible tooltip
    /// stays visible and is not relocated by hand-tremor-scale samples.
    #[test]
    fn jitter_does_not_dismiss_or_relocate() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        assert!(m.tick(t0 + D).is_visible());

        // A 2px move (< 3px threshold) is jitter: no dismissal armed, anchor
        // unchanged.
        assert_eq!(
            m.on_pointer_moved(at(12., 10.), t0 + D + Duration::from_millis(100)),
            TooltipState::Visible { at: at(10., 10.) }
        );
        assert_eq!(
            m.next_deadline(),
            None,
            "no dismissal timer armed by jitter"
        );
    }

    /// A jitter (hold-still) sample arriving while counting down to dismiss is
    /// read as the pointer coming to rest again, and relocates the still-visible
    /// tooltip with no new delay — the production re-rest path, since the window
    /// re-dispatches the last move on each redraw and two same-spot samples read
    /// as "stopped here".
    #[test]
    fn jitter_sample_during_pending_dismiss_relocates() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        assert!(m.tick(t0 + D).is_visible());

        // Real move → PendingDismiss (anchor still pinned at the old spot).
        let t_move = t0 + D;
        assert_eq!(
            m.on_pointer_moved(at(60., 60.), t_move),
            TooltipState::Visible { at: at(10., 10.) }
        );
        // Hold still near the new spot (within jitter) → re-rest → relocate to the
        // held position, before the dismissal deadline.
        assert_eq!(
            m.on_pointer_moved(at(61., 61.), t_move + Duration::from_millis(50)),
            TooltipState::Visible { at: at(61., 61.) }
        );
        assert_eq!(m.next_deadline(), None, "dismissal cancelled by re-rest");
    }

    /// `on_pointer_rested` on a hidden machine arms the show timer (does not show
    /// instantly) — the explicit-rest entry point still honors the show delay
    /// when nothing is visible yet.
    #[test]
    fn explicit_rest_while_hidden_arms_show_timer() {
        let mut m = machine();
        let t0 = Instant::now();
        assert_eq!(m.on_pointer_rested(at(5., 5.), t0), TooltipState::Hidden);
        assert_eq!(
            m.tick(t0 + D - Duration::from_millis(1)),
            TooltipState::Hidden
        );
        assert_eq!(m.tick(t0 + D), TooltipState::Visible { at: at(5., 5.) });
    }

    /// A pointer sample that arrives after its show deadline still resolves to a
    /// show at that sample time (deadlines are resolved on the next input, not
    /// only by an external tick).
    #[test]
    fn late_sample_resolves_pending_show() {
        let mut m = machine();
        let t0 = Instant::now();
        m.on_pointer_moved(at(10., 10.), t0);
        // Next sample arrives well after the deadline, still at rest (jitter).
        assert_eq!(
            m.on_pointer_moved(at(11., 11.), t0 + D + Duration::from_millis(50)),
            TooltipState::Visible { at: at(10., 10.) }
        );
    }

    /// `next_deadline` reports the pending show/dismiss instant and nothing in
    /// the steady states, so a driver can schedule exactly one timer.
    #[test]
    fn next_deadline_tracks_pending_phases() {
        let mut m = machine();
        let t0 = Instant::now();
        assert_eq!(m.next_deadline(), None, "idle has no deadline");

        m.on_pointer_moved(at(0., 0.), t0);
        assert_eq!(m.next_deadline(), Some(t0 + D), "pending-show deadline");

        m.tick(t0 + D);
        assert_eq!(m.next_deadline(), None, "visible steady state");

        let t_move = t0 + D + Duration::from_millis(10);
        m.on_pointer_moved(at(30., 0.), t_move);
        assert_eq!(
            m.next_deadline(),
            Some(t_move + D),
            "pending-dismiss deadline"
        );
    }
}
