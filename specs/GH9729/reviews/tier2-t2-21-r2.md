---
item: tier2-t2-21
commit: d28f6f3
reviewer: R2-quality
spec_ref: tech.md §698
verdict: pass-with-nits
---

# Findings

- [pass] **Pure-helper extraction is exemplary.** `double_tap_zoom_target(zoom_old, pan_old, tap_offset_from_center) -> (f32, Vector2F)` is a textbook pure function: three `Copy` inputs in, one `Copy` tuple out, no `self`, no `ctx`, no app state, no allocation. Same pattern previously used for `step_zoom` (per t2-7-r2) and `format_metadata_line` (per t2-11) — this is now a consistent idiom across `lightbox_view.rs`. Easy to unit-test, easy to reason about, easy to relocate later if the math ever wants to move into `ui_components`.

- [pass] **Action-with-coordinates is the right shape.** `LightboxViewAction::DoubleTapZoom { tap_offset_from_center_x, tap_offset_from_center_y }` carries the centring coordinates so zoom and pan apply atomically against the same `zoom_factor` value. The commit message and the doc-comment on the variant both explain *why* splitting into `ZoomIn + Pan` would miscentre (pan would be computed against the post-zoom factor). This is exactly the kind of comment that prevents a well-meaning refactorer from re-introducing the bug. The same pattern (action with payload fields rather than a struct wrapper, so `Debug` derives cleanly) matches the existing `Pan { offset_x, offset_y }` arm — internally consistent.

- [pass] **Two `f32` scalars vs `Vector2F` on the action.** Mirroring the `Pan` variant's `offset_x` / `offset_y` shape is the right call. The doc-comment on `Pan` (lines 84-87) explicitly states the reason: `Debug` derives cleanly without an extra impl on a wrapping type. `DoubleTapZoom` quietly follows the same convention. Pathfinder's `Vector2F` does implement `Debug` so the constraint is softer than the comment suggests, but staying consistent with the neighbouring variant is the right tradeoff.

- [pass] **Centring math comment.** The doc-comment on `double_tap_zoom_target` derives the formula from first principles (image-coordinate of the tap → screen offset after zoom → pan that lands it at viewport centre) and includes the closed form in a fenced text block. The commit body also reproduces the formula. Anyone debugging an off-by-2× centring bug in three months will be able to retrace the derivation.

- [pass] **Convention citations.** `ZOOM_STEP`'s new doc-comment lists Preview / Safari / Chrome explicitly and quantifies the keypress count to reach the clamps (`~10 +` to max, `~7 -` to min) — this is the right level of "why 1.25" justification. `DOUBLE_TAP_TARGET_ZOOM`'s doc-comment cites macOS Preview and iOS Photos for the 2× target and the native-toggle behaviour. The commit body also names Photoshop. Conventions are sourced, not asserted.

- [pass] **Toggle semantics correctly favour user intent over tap coordinates on zoom-out.** When `zoom_old > 1.0`, the helper resets pan to zero and ignores the tap location — matching Preview / iOS Photos, where the second double-tap reverts to native and re-centres. The doc-comment ("Tap position is irrelevant") states this explicitly and the second test pins the behaviour. Correct: at native zoom the image fits the viewport, so the stored pan is meaningless.

- [pass] **Defensive guards layered correctly.** The helper rejects non-finite / non-positive `zoom_old` (test 4); the dispatch handler in `LightboxView` separately rejects non-finite `tap` coordinates before calling the helper (lines 597-600); the no-op guard `if next_zoom != self.zoom_factor || next_pan != self.pan_offset` avoids a spurious `ctx.notify()`. Three layers, each guarding a different failure mode — none is redundant.

- [pass] **Test rigor on the helper, 4 tests.** Covers (a) native → 2× with a non-centre tap, asserting the exact pan magnitude derivable by hand; (b) zoomed-in → 1.0 toggle ignoring tap; (c) sub-native (0.5) start with non-zero `pan_old`, exercising the `pan_old * scale_ratio` term that the simpler tests don't; (d) parameterised non-finite / non-positive `zoom_old`. Boundary mathematics, toggle direction, composition with existing pan, and pathological inputs — that's the right four-test cross-section for a pure tuple-returning helper. Asserts use `< f32::EPSILON` for the centring math and `==` for the deterministic toggle output, which is the right discrimination.

- [pass] **`click_count >= 2` rather than `== 2`.** Triple-clicks fall through into the double-tap branch rather than spawning a third interpretation. Idiomatic for this kind of gesture handler — the alternative (`== 2`) would silently degrade triple-clicks into single-click drags, which is a worse user experience.

- [pass] **Drag state cleared on double-click.** `self.drag_state.lock() = None` after handling the double-tap; without this, the second click of the double-tap would have set `drag_state = Some(position)` on the first click, then the second's `LeftMouseDragged` events would pan during the zoom gesture. The comment ("Skip drag-tracking so the second-click's drag delta doesn't pan during the gesture") names the bug being prevented.

- [nit] **Stale `ZOOM_STEP = 1.5` comment in `format_metadata_line_rounds_zoom_to_integer`.** `app/src/workspace/lightbox_view.rs:797-799`:

  ```rust
  // ZOOM_STEP = 1.5 → after one zoom-in from 1.0 the factor is
  // 1.5 exactly, but accumulated multiplications produce
  // irrational-looking values (1.5 * 1.5 = 2.25, so "225%").
  ```

  The test body uses literal `1.5` / `1.0 / 1.5` (not the `ZOOM_STEP` constant), so the assertions still pass — but the prose comment now misstates the production constant. Either (a) rewrite the comment to drop the ZOOM_STEP reference and just say "1.5 is a convenient zoom factor with a finite-but-non-trivial percentage" or (b) reword the test to use `ZOOM_STEP` and pick an example value that still exercises rounding under 1.25× (e.g. `1.0 / 1.25 = 0.8 → "80%"` — but that's not actually a rounding case, so option (a) is cleaner). Tiny follow-up.

- [nit] **Fully-qualified `pathfinder_geometry::vector::Vector2F` in the helper signature.** `app/src/workspace/lightbox_view.rs:290-292` writes out the full path three times in the signature and once in the `use` inside the body. The file already imports `pathfinder_geometry::vector::{Vector2F, vec2f}` at line 4. Using the short name in the signature would match the rest of the file's idiom and would also make the signature one line per parameter rather than two. Pure style — non-blocking.

- [nit] **`DOUBLE_TAP_TARGET_ZOOM` lives in `ui_components::lightbox` but is only consumed by `app::workspace::lightbox_view`.** `ZOOM_STEP`, `MIN_ZOOM_FACTOR`, `MAX_ZOOM_FACTOR` have the same shape — all are zoom-policy constants consumed only by the `LightboxView` handler — and `tier2-t2-7-r2:39-43` already flagged this. Moving them to `lightbox_view.rs` would be a strict simplification (one fewer `pub` boundary, the constants live next to the only code that touches them) but it's the same call across all four constants; deferring as a single cleanup is reasonable. Don't expand the surface; don't shrink it in isolation either.

- [nit] **Resolvable t2-7-r2 follow-up: round-trip cancellation now reads more naturally with 1.25.** `step_zoom(step_zoom(1.0, In), Out)` should equal `1.0` exactly. `1.25` is exactly representable in IEEE-754 (it is `5/4 = 1.0100…₂`), so `1.0 * 1.25 / 1.25 == 1.0` bit-exactly. The t2-7-r2 review (lines 138-149) flagged this as a cheap-but-missing one-liner — its motivation is *stronger* now that the constant has been tuned, because the next person who retunes (e.g. to 1.4) would benefit from a test that catches drift. Defer to a separate commit; t2-21 is the right place to note that the prerequisite is now stable.

- [nit] **Resolvable t2-7-r2 follow-up: near-min clamp boundary test.** `step_zoom(0.3, Out)` returns exactly `MIN_ZOOM_FACTOR` (since `0.3 / 1.25 = 0.24 < 0.25 = MIN_ZOOM_FACTOR`). The t2-7-r2 review (lines 127-136) wanted this as a pinned boundary test instead of the 50-iteration spam. Still applies, still cheap, and now has a clean `0.3 / 1.25 = 0.24` arithmetic that lands one ULP under the floor rather than the much-looser `0.3 / 1.5 = 0.2`. Recommend folding into the same follow-up commit as the round-trip test.

- [minor] **`on_double_tap_zoom` callback site cannot observe missing layout info.** In `lightbox.rs:221-229`, if either `self.origin` or `self.viewport_size` is `None`, the callback is silently skipped (the `if let (Some, Some) = …` arm just exits) but the function still `return true`s and clears `drag_state`. This is correct (we don't have a viewport centre to compute the tap-offset against) but the silent skip is mildly worrying for debuggability: the test that doubles-taps on an unmeasured `PanClippedImage` would see "double-tap consumed but did nothing." Layout always populates `origin` and `viewport_size` before paint, so in practice this branch shouldn't fire — but a one-line `log::warn!` or `debug_assert!` would make a future regression obvious. Non-blocking; the silent fall-through matches how the existing `on_pan` path handles the same missing-layout case.

- [minor] **Bare arrows render as em-dashes in the commit message.** The commit body uses `→` between "1.0 default" and "a more granular feel" (line 12 of `git show`). Not a code concern; flagging only because a future grep for em-dashes in commit prose (which the repo's user-level CLAUDE.md prohibits in drafted prose) would catch this. Acceptable as written.

# What I checked

- `git show d28f6f3 --stat` and the full diff against both `app/src/workspace/lightbox_view.rs` and `crates/ui_components/src/lightbox.rs`. Verified the new helper, action variant, options field, element callback, and 4 tests are all in place.
- `tech.md` §698 — confirmed "Zoom and pan controls" wording and that the keybinding step is non-prescriptive (no specific `ZOOM_STEP` value mandated; "use mainstream conventions" is a reasonable interpretation of the spec).
- `grep -n "1\.5\|ZOOM_STEP"` across both files — only `format_metadata_line_rounds_zoom_to_integer:797-799` carries a stale `ZOOM_STEP = 1.5` comment. The test body uses literal 1.5 (not the constant), so the test still passes but the comment is misleading.
- Compared `LightboxViewAction::DoubleTapZoom { tap_offset_from_center_x, tap_offset_from_center_y }` against the existing `Pan { offset_x, offset_y }` shape — same scalar-fields-not-struct-wrapper convention; the explanatory comment on `Pan` (lines 84-87) covers both arms.
- `PanClippedImage::dispatch_event` mouse-down branch (lightbox.rs:212-242) — verified the `click_count >= 2` early return, the drag-state clear, the viewport-centre computation, and the single-click fall-through are all wired correctly.
- `Options.on_double_tap_zoom` and its `None` default in `impl crate::Options::default()` — verified the field is optional, defaults to `None`, and that `None` cleanly disables the gesture (the callback site is `if let Some(cb)` rather than `expect`).
- The 4 helper unit tests for boundary semantics: native→2×, zoomed→native toggle, 0.5×→2× composition with pre-existing pan, parameterised non-finite/non-positive zoom_old. Each pins a distinct branch of the helper.
- `specs/GH9729/reviews/tier2-t2-7-r2.md:118-149` for the two deferred boundary-test bullets (near-min clamp and round-trip cancellation). Both remain resolvable; the round-trip test is more motivated now that ZOOM_STEP has been retuned.
- `specs/GH9729/TIER2_TODO.md:78-87` — confirmed t2-21 bullet text matches the implementation (1.25× zoom, double-tap toggle, atomic action carrying coords, 4 new tests).

# Suggestions

1. In a tiny follow-up commit (not this one): rewrite the stale `ZOOM_STEP = 1.5` comment on `format_metadata_line_rounds_zoom_to_integer` at `app/src/workspace/lightbox_view.rs:797-799` to drop the reference to the production constant. The test exercises `1.5` and `1.0/1.5` as zoom-factor values directly; the comment doesn't need to claim those values come from `ZOOM_STEP`.
2. Optionally simplify the `double_tap_zoom_target` signature to use the file-level `Vector2F` and `vec2f` imports (drop the fully-qualified `pathfinder_geometry::vector::` prefix in the signature and the `use` inside the body). Pure style.
3. Fold the two deferred t2-7-r2 boundary tests (round-trip `step_zoom(step_zoom(1.0, In), Out) == 1.0` and near-min clamp `step_zoom(0.3, Out) == MIN_ZOOM_FACTOR`) into a follow-up commit now that the constant has been tuned. Both are stronger as boundary pins with `ZOOM_STEP = 1.25` than they were with 1.5.
4. (Future) When the zoom-policy constants migrate out of `ui_components::lightbox` (per t2-7-r2's earlier nit), move `DOUBLE_TAP_TARGET_ZOOM` with them — same shape, same consumer, same justification.
