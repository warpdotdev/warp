---
item: tier2-t2-15
commit: 6623e0e
reviewer: R1-correctness
spec_ref: tech.md §698 (supplemental)
verdict: concerns
---

# Spec

The tracker bullet for t2-15 calls for two structural changes: (a) zoom-out and zoom-in icon buttons must sit adjacent in a tight visual cluster, and (b) the "100%" reset label is rendered only when `zoom != 1.0` (hidden, not greyed), replacing t2-13's `disabled` flag. The bullet also asserts a hypothesis: conditional-rendering of the wider Label button sidesteps a suspected hit-test overlap where the middle "100%" slot was bleeding into the Plus slot at native zoom. The tech.md spec text at §698 is one line and gives no implementation guidance; this is a polish row.

# Findings

- [pass] Layout intent is delivered: Minus and Plus are placed at adjacent narrow `ZOOM_ICON_BUTTON_SLOT = 32.` slots starting at `SCRIM_BUTTON_INSET`, and the "100%" reset is gated on `zoom != 1.0` rather than rendered greyed. Constants are documented in-place with §698 / t2-15 attribution.
- [pass] The `disabled: zoom == 1.0` flag from t2-13 is correctly removed (the conditional render makes it redundant). No stale `disabled` plumbing left behind.
- [major] **Hypothesis is unverified and the fix may not actually solve the reported `+` click bug.** The follow-on commit on this branch (t2-16, currently at lines 793–803 of `crates/ui_components/src/lightbox.rs`) documents the real root cause: "separate `add_positioned_child` siblings each report their own bbox to the Stack including the button's interactive padding (hover hit-area beyond visual edge). Stack dispatches first-added-first, so `−`'s extended hit-area claimed clicks intended for `+`." If correct, t2-15 did not eliminate the overlap at native zoom — the Minus button at offset 12 with extended padding still covers the Plus button at offset 44 (only 32 px away). The commit message itself flags this as unproven, but the bullet states the layout is "provably overlap-free at native zoom," which is too strong a claim given the button hit-area extends past the visual 24-ish-px footprint. Treat as a major correctness concern for the stated intent (fix the `+` click), even though the layout-preference half of the bullet does land.
- [minor] **Strict-equality float compare on `zoom_factor`.** Both call sites (`disabled: zoom == 1.0` in the old code, and `if zoom != 1.0` in the new conditional render) use exact float equality. If the caller drives `zoom_factor` purely through the multiplicative 1.25x step (t2-21), 1.0 is exactly representable and round-tripping through `* 1.25 / 1.25` will land on 1.0 only when explicitly reset (`ZoomDirection::Reset`); otherwise floating-point drift could leave a near-1.0 value (e.g. `0.9999...` or `1.0000001`) that renders the reset button without the visual at-100% state matching. Worth either an epsilon-tolerant check (e.g. `(zoom - 1.0).abs() > f32::EPSILON`) or a documented caller invariant that Reset always snaps exactly. Not blocking because the current zoom stepping path in this branch appears to clamp/snap, but the gate is fragile to future changes.
- [minor] **NaN gate.** `params.zoom_factor` is `f32` with no NaN guard. `NaN != 1.0` is `true` in Rust IEEE-754, so a NaN zoom factor would render the reset button. There is a `.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR)` at line 616 for the image-scaling code path, but the toolbar branch reads `params.zoom_factor` directly without clamping. Low practical likelihood, but the gate could re-use the already-clamped value.
- [minor] **Reset-button offset is hard-coded from button slot math, not from measured icon-cluster width.** The reset is placed at `SCRIM_BUTTON_INSET + 2. * ZOOM_ICON_BUTTON_SLOT + ZOOM_RESET_GAP_FROM_ICONS`. If the small-icon button's rendered width changes (theme/icon-set update), the two slots may no longer line up with the actual `[−][+]` cluster, and the gap before "100%" becomes either too tight or visibly off. The follow-on t2-16 work moved the icon pair into a `Flex::row` for exactly this reason; t2-15 is a transitional state that depends on the assumed slot width.
- [minor] **Re-render churn on every drag tick.** The `zoom_reset_button` is conditionally constructed and added inside `render`, which means the persistent `self.zoom_reset_button` field is invoked only on some renders. If the underlying `button::Button` keeps per-render state (hover, press), there's a potential glitch when the button disappears mid-interaction (e.g. cursor is over "100%" when the user zooms back to exactly 1.0 via a reset). Likely benign — the click that triggers Reset is processed before the next render — but worth noting.
- [pass] No interaction with the error-scrim path: the zoom toolbar branch is gated on `params.options.on_zoom.is_some()`, independent of `LightboxImageSource::Error`. That existed before t2-15 and is not regressed.
- [nit] Tab order / keyboard focus: GPUI buttons are mouse-click only here (no `keystroke` on the zoom buttons) so the conditional render does not perturb a tab cycle. No regression, but flagging because the bullet's "Regressions" axis was called out.
- [nit] Commit message says "Tests: lightbox_view tests untouched, still 18/18" — but no test was added that exercises the conditional render at exactly `zoom == 1.0` vs. `zoom != 1.0`. A two-line unit test asserting the reset button appears/disappears would lock the conditional in place.

# What I checked

- Full diff of `6623e0e` via `git show 6623e0e -- crates/ui_components/src/lightbox.rs`.
- `specs/GH9729/tech.md` §698 (single bullet; no further spec text on toolbar layout).
- Current state of `crates/ui_components/src/lightbox.rs` around the zoom toolbar (lines 420–440, 490–565, 705–900) to confirm the post-t2-15 follow-on (t2-16) and the documented root cause of the hit-test bug.
- `add_positioned_child` call sites in `crates/ui_components/src/lightbox.rs` to compare the new zoom-toolbar placement against the prev/next/close button placements (same `OffsetPositioning::offset_from_parent` shape, consistent with existing patterns).
- `ZoomDirection` enum and `on_zoom` handler signature: unchanged, no API break.
- Error-state branch (`LightboxImageSource::Error`) layout interaction: toolbar render is gated solely on `on_zoom.is_some()`, no error-path regression introduced.
- Tab / focus axis: no `keystroke` plumbing on the zoom buttons, no keyboard-navigation regression to flag.

# Suggestions

- Replace `zoom != 1.0` with an epsilon-tolerant check, or document and enforce that `ZoomDirection::Reset` snaps exactly to `1.0_f32` and all multiplicative stepping originates from that snapped value. Clamping `params.zoom_factor` at the top of `render()` once (rather than re-clamping inside the image-scaling branch) would also remove the NaN edge.
- If the hit-test root cause documented in t2-16 is correct, retroactively note in the t2-15 commit (or in TIER2_TODO.md) that the `+` click fix landed at t2-16, not here — t2-15 delivered the layout-preference half of the bullet.
- Consider a regression test: `Lightbox::render` with `zoom_factor = 1.0` versus `0.8` should produce a different child count under `add_positioned_child` (or whatever the lightbox_view test harness can observe). Locks the conditional render against a future refactor that quietly re-introduces the greyed-out state.
