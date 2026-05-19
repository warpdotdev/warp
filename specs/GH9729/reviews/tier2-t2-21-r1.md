---
item: tier2-t2-21
commit: d28f6f3
reviewer: R1-correctness
spec_ref: tech.md §698
verdict: pass-with-nits

---

# Spec

`tech.md:698`: "Zoom and pan controls: extend `lightbox::Params` with
zoom state and `lightbox_view.rs` keybindings (`+`, `-`, `0`,
drag-to-pan)." t2-21 is a polish bullet refining the cmd-+/cmd--
zoom step to the mainstream 1.25x convention (Preview / Safari /
Chrome / Photoshop) and adding the macOS-Preview / iOS-Photos
double-tap-to-zoom-and-center gesture, with the centering math
extracted as a pure helper and four new unit tests.

# Findings

- [pass] Spec fidelity. `ZOOM_STEP: f32 = 1.25` lands at
  `crates/ui_components/src/lightbox.rs:351`; `DOUBLE_TAP_TARGET_ZOOM:
  f32 = 2.0` at line 357; new `LightboxViewAction::DoubleTapZoom`
  variant carries combined coordinates
  (`app/src/workspace/lightbox_view.rs:88-99`); the
  `double_tap_zoom_target` pure helper is at
  `app/src/workspace/lightbox_view.rs:264-308`; four new tests added
  (`:710-770`). Every bullet of the spec line is delivered.
- [pass] Step roundtrip preserved at the new 1.25 step. `1.0 * 1.25 =
  1.25` exactly in IEEE-754 single, and `1.25 / 1.25 = 1.0` exactly,
  so `step_zoom(step_zoom(1.0, In), Out) == 1.0` still holds bit-for-
  bit. The t2-13 "reset disabled when `zoom == 1.0`" check is
  unaffected. (Verified empirically with f32 round-trip arithmetic;
  also `1.0 / 1.25 * 1.25 == 1.0` exactly, so a `-` followed by `+`
  from native also returns exactly to 1.0.)
- [pass] Boundary semantics intact. With `ZOOM_STEP = 1.25`,
  `MAX_ZOOM_FACTOR = 8.0` is reached in 10 `+` presses (`log(8)/log(1.25)
  ~ 9.32`, rounded up by the `clamp`), and `MIN_ZOOM_FACTOR = 0.25`
  in 7 `-` presses (`log(0.25)/log(0.8) ~ 6.21`). The commit message's
  "~10 / ~7" figures are accurate. The existing
  `step_zoom_in_clamps_to_max` and `step_zoom_out_clamps_to_min`
  tests at `:692-707` loop 20 times then assert equality — they still
  pass because both bounds remain reachable; the larger loop bound is
  not load-bearing.
- [pass] Centering math is correct. Verified algebraically:
  the image-coordinate of a tap at viewport-relative offset `tap`
  under (`s_old`, `pan_old`) is `(tap - pan_old) / s_old`. After
  zooming to `s_new`, that point's screen offset from the image
  centre is `(tap - pan_old) * (s_new / s_old)`. To pin the tapped
  point at viewport centre, image centre (= `pan_new` from viewport
  centre) must equal `-((tap - pan_old) * (s_new / s_old))`, i.e.
  `pan_new = -tap * (s_new / s_old) + pan_old * (s_new / s_old)`.
  This matches the helper at `lightbox_view.rs:307` and the
  docstring derivation at `:269-289`. The two
  numerically-loaded tests (native → 2x and shrunk-with-pan)
  exercise both terms of the formula and both pass numerical
  spot-check.
- [pass] Atomicity is real. `DoubleTapZoom` carries both
  `tap_offset_*` coordinates and the handler computes `(next_zoom,
  next_pan)` in one helper call, then assigns both before
  `ctx.notify()` (`lightbox_view.rs:593-607`). No intermediate render
  can observe zoom-without-corresponding-pan. The commit message's
  rationale ("splitting into ZoomIn + Pan would miscenter") is
  correct: a split would compute the pan delta against the
  post-zoom `s_new` and the formula would degenerate.
- [pass] Toggle semantics match Preview / Photos. Helper at
  `lightbox_view.rs:300-303`: any `zoom_old > 1.0` returns `(1.0,
  Vector2F::zero())` with tap ignored; otherwise zooms to
  `DOUBLE_TAP_TARGET_ZOOM` and applies the centering math. That
  matches the "double-tap toggles between native and 2x" convention.
  Subtle but correct: `zoom_old < 1.0` (a shrunk image) also routes
  to the zoom-in branch, which feels right — a user shrunk past
  native and double-tapping clearly wants to magnify, not toggle
  back to the same shrunken state.
- [pass] Defensive sanitisation. `double_tap_zoom_target` guards
  `!zoom_old.is_finite() || zoom_old <= 0.0` and returns the safe
  default `(1.0, Vector2F::zero())` at `:296-298`. The
  `DoubleTapZoom` action handler also guards `tap` non-finite at
  `:598-600`. Test `double_tap_zoom_sanitises_non_positive_zoom`
  at `:761-770` covers 0.0, -1.0, NaN, and INFINITY — meaningful
  edge coverage. The four-input loop is a slight idiom departure
  (parameterised tests in this codebase are uncommon) but the
  `for bad in [...]` pattern is readable.
- [pass] Tap-near-edges is correctly *not* a special case. The
  centering math holds geometrically anywhere in viewport
  coordinates; clamping is handled at paint time by
  `PanClippedImage`'s existing `clamp_pan` logic (t2-19), which
  pins the visible edge to viewport centre at the boundary. So a
  double-tap in a corner stores a pan_offset that overshoots the
  clamp; the renderer absorbs that. (This does, however, intersect
  with the existing t2-19 [major] carry-over — see below.)
- [pass] Native-size tap branch. At `zoom_old == 1.0` the
  `zoom_old > 1.0` toggle-guard is false, so the helper proceeds to
  the zoom-in branch and centres on the tap. Confirmed by the
  `double_tap_zoom_from_native_targets_2x_and_centers_on_tap` test
  at `:711-724`.
- [minor] **Stale comment in `format_metadata_line_rounds_zoom_to_integer`
  test.** `lightbox_view.rs:797` still reads "// ZOOM_STEP = 1.5 →
  after one zoom-in from 1.0 the factor is // 1.5 exactly, but
  accumulated multiplications produce // irrational-looking values
  (1.5 * 1.5 = 2.25, so "225%")." After the t2-21 change `ZOOM_STEP`
  is 1.25 and the example arithmetic no longer matches the constant.
  The test body itself uses literal `1.5` and `1.0 / 1.5` so the
  assertion is still mechanically correct (it's about rounding,
  not about `ZOOM_STEP`), but the doc comment is now misleading.
  One-line cleanup.
- [minor] **`DOUBLE_TAP_TARGET_ZOOM = 2.0` is not on the 1.25 step
  lattice, so post-double-tap `cmd--` will never re-land on exactly
  1.0.** Concretely: after a double-tap at native the user sits at
  `zoom_factor == 2.0`. Pressing `-` yields the sequence
  `2.0 → 1.6 → 1.28 → 1.024 → 0.8192 → …`, never `1.0`. The reset
  button (or a second double-tap, or `0` key) still recovers to
  exactly 1.0, so this isn't a soft-lock — but the "reset disabled
  at exactly 100%" affordance from t2-13 will never re-disable
  through stepwise zoom-out from a double-tapped state. Users who
  rely on `cmd-+` / `cmd--` to walk back to native are mildly
  surprised. Mitigations: (a) snap `step_zoom` to 1.0 within a small
  epsilon, (b) keep as-is and rely on the visible "100%" label going
  back to `102%` / `81%` to cue the user toward reset. Not blocking;
  worth noting for product polish.
- [minor] **`cmd-+`/`cmd--` after double-tap-to-2.0 produces no-op
  step on first press if user expects "step from native".** Same
  observation phrased from the user's mental model: pressing `+`
  from 2.0 goes to 2.5 (correct), pressing `-` goes to 1.6 (correct
  by formula, surprising if the user thought they were at native).
  Documented in t2-13 R1 as a non-issue when ZOOM_STEP was 1.5 (1.5
  / 1.5 = 1.0 exactly); now it's a real shape, because 2.0 is not
  a power of 1.25. Same mitigation options.
- [minor] **Test naming is verbose / slightly inconsistent.** The
  helper-tests use full sentence names (`double_tap_zoom_from_native_targets_2x_and_centers_on_tap`)
  while the surrounding `step_zoom_*` tests use shorter forms.
  Stylistic; not blocking.
- [carry-over major from t2-19 R1, NOT t2-21's scope]
  **`pan_offset` is still stored unclamped in `LightboxView` and
  only clamped at paint.** Confirmed at `lightbox_view.rs:585-591`.
  After a corner double-tap the stored `pan_new` can exceed
  `max_pan`, producing the direction-reversal deadband documented
  in t2-19 R1 [major]. The double-tap path makes this slightly
  more reachable in practice (one click vs many drag deltas), but
  doesn't introduce the bug. Still needs the t2-19 R1 fix
  (option 1: expose `clamp_pan` and clamp in handler).
- [carry-over major from t2-19 R1, NOT t2-21's scope]
  **Whole-viewport hit-test still suppresses scrim-Dismiss in the
  scrim-toned area around an under-fitted image.** Confirmed at
  `lightbox.rs:216`. The new double-tap branch *inside* that
  `point_in_viewport`-gated arm inherits the same hit-test issue
  — a double-tap on the dead area around a small image will fire
  the zoom-and-center on a tap location outside the actual image
  pixels. The math still works (the tapped point is in viewport
  coords, not image coords), but it's surprising UX: clicking the
  visually-empty scrim around the image triggers a zoom-in. Best
  fix is to tighten the hit-test to the painted child bounds
  (already a t2-19 R1 suggestion); t2-21 doesn't make it worse.
- [carry-over minor from t2-19 R1, NOT t2-21's scope] **Child
  never sees events**: same as t2-19. The new `click_count >= 2`
  branch also doesn't forward to the child, consistent with the
  rest of `dispatch_event`. No regression.

# What I checked

- `git show d28f6f3 --stat` and full diff
  (`app/src/workspace/lightbox_view.rs +142`,
  `crates/ui_components/src/lightbox.rs +56/-8`,
  `crates/ui_components/examples/library.rs +2`,
  `specs/GH9729/TIER2_TODO.md +11`).
- `specs/GH9729/tech.md:698` for the spec line.
- `specs/GH9729/reviews/tier2-t2-13-r1.md` for the "step_zoom
  roundtrip to exactly 1.0" claim and the "reset button disabled
  when `zoom == 1.0`" t2-13 disabled-check.
- `specs/GH9729/reviews/tier2-t2-19-r1.md` and `tier2-t2-20-r1.md`
  for the carry-over [major]s (unclamped pan_offset, viewport
  hit-test, child not receiving events) — confirmed all three are
  still live after t2-21 by re-reading the current
  `lightbox.rs:205-300` and `lightbox_view.rs:585-608`.
- f32 round-trip arithmetic: `1.0 * 1.25 / 1.25 == 1.0` exactly,
  `1.0 / 1.25 * 1.25 == 1.0` exactly. Both confirmed via single-
  precision IEEE-754 evaluation.
- Step-count to clamp boundary: `log(8.0) / log(1.25) ~ 9.32`,
  `log(0.25) / log(0.8) ~ 6.21`. Confirms the commit message's
  ~10/~7 claim.
- Post-double-tap step sequence: 2.0 / 1.25^n for n = 1..6 yields
  {1.6, 1.28, 1.024, 0.8192, 0.65536, 0.524288}. Confirms the
  [minor] "never re-lands on 1.0" finding.
- `crates/ui_components/src/lightbox.rs:205-300` for the
  `dispatch_event` arms (LeftMouseDown branches into double-tap
  at `click_count >= 2`, drag state cleared, returns true;
  single-click sets drag_state; ScrollWheel arm unchanged).
- `crates/ui_components/src/lightbox.rs:351-358` for the constants.
- `crates/ui_components/src/lightbox.rs:540-558` for the
  `Options.on_double_tap_zoom` callback field and `Options::default`
  setting it to `None`.
- `crates/ui_components/src/lightbox.rs:622-636` for the `Lightbox::render`
  call site that clones `on_double_tap_zoom` into `PanClippedImage`.
- `app/src/workspace/lightbox_view.rs:88-99` for the
  `LightboxViewAction::DoubleTapZoom` variant.
- `app/src/workspace/lightbox_view.rs:264-308` for the
  `double_tap_zoom_target` pure helper.
- `app/src/workspace/lightbox_view.rs:510-525` for the
  `on_double_tap_zoom` closure dispatched from `View::render`.
- `app/src/workspace/lightbox_view.rs:593-608` for the
  `DoubleTapZoom` handler arm — confirms atomic assignment.
- `app/src/workspace/lightbox_view.rs:710-770` for the four new
  unit tests; each verified for meaningful coverage (centering
  math, toggle, scaled compose, non-finite guard).
- `app/src/workspace/lightbox_view.rs:795-807` for the stale
  "ZOOM_STEP = 1.5" doc-comment in
  `format_metadata_line_rounds_zoom_to_integer`.

# Suggestions

- **Fix the stale comment at `lightbox_view.rs:797`** mentioning
  "ZOOM_STEP = 1.5". Either drop the `ZOOM_STEP` reference (the
  test is about rounding behaviour, not about the step constant)
  or update to "ZOOM_STEP = 1.25 → after one zoom-in from 1.0 the
  factor is 1.25 exactly, but `1.0 / 1.25 = 0.8` exact in f32 and
  rounds to 80% in the footer; this test exercises the older 1.5
  reciprocal `0.6667 → 67%` rounding path which still applies."
- **Consider snapping `step_zoom` to exactly `1.0` within a small
  epsilon.** After `DOUBLE_TAP_TARGET_ZOOM = 2.0`, stepwise
  zoom-out walks through `1.6, 1.28, 1.024, …` and never re-lands
  on 1.0, so the t2-13 "reset disabled at 100%" affordance never
  reactivates via `cmd--`. A trivial guard
  (`if (new - 1.0).abs() < 0.05 { 1.0 } else { new }`) would close
  the loop. Not blocking — reset / `0` / second double-tap still
  recover to exact 1.0.
- **Pick up the t2-19 R1 carry-overs as a single follow-up commit
  before tier-2 close-out.** `pan_offset` clamping in the handler,
  image-bounds (not viewport) hit-test for both single- and
  double-click consumption, and child event forwarding. The
  double-tap path inherits all three, so closing them now also
  hardens t2-21.
- **Optional: add a centering-math integration test that uses the
  full action dispatch path** (not just the pure helper). The
  helper covers the formula; an end-to-end test would cover the
  `LeftMouseDown { click_count: 2 }` → callback →
  `LightboxViewAction::DoubleTapZoom` → handler path, catching
  any future regression in the plumbing layer. Element-tree tests
  in this framework need a presenter harness, which is a known
  gap (see t2-19 R1 / t2-20 R1) — so this remains a follow-up
  rather than a t2-21 blocker.
