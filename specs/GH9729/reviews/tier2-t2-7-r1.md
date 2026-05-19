---
item: tier2-t2-7
commit: 6aee220
reviewer: R1-correctness
spec_ref: tech.md §698
verdict: pass-with-nits
---

# Spec

> - **Zoom and pan controls**: extend `lightbox::Params` with zoom state and `lightbox_view.rs` keybindings (`+`, `-`, `0`, drag-to-pan).

(`specs/GH9729/tech.md` line 698, single bullet under the deferred-follow-ups list.)

# Findings

- **Pan deferral rationale verified.** Searched
  `crates/warpui_core/src/elements/` for any element type whose name or
  surface looks like a translate / offset / transform primitive
  (`grep -lE "pub struct (Translate|Offset|Transform)\b"` over the
  elements directory: empty result). The closest neighbours are
  `Clipped` (which clips a layer but paints the child at the same
  origin it received — `clipped.rs:64` calls `self.child.paint(origin,
  …)` with no offset) and the scrollable family (which translates
  *content* via internal scroll offsets, not arbitrary children). The
  commit's claim that this fork has no `Translate`/`Offset`/`Transform`
  primitive is accurate, and `t2-7-pan` correctly captures both
  blocker and unblocking design (new element vs. paint-origin bias on
  `PaintContext`). Splitting the work this way is correct.
- **`step_zoom` numeric correctness.** Verified manually:
  `1.0 * 1.5^5 = 7.59375` (clamped to 8.0 on press 6),
  `1.0 / 1.5^4 = 0.1975…` (clamped to 0.25 on press 5). The 50-iteration
  test bound `step_zoom_in_clamps_to_max` /
  `step_zoom_out_clamps_to_min` is well over the 5- / 4- press
  saturation point and verifies the `==` equality at `MAX_ZOOM_FACTOR`
  / `MIN_ZOOM_FACTOR` (no float drift at the saturated value because
  `clamp` returns the bound exactly).
- **Non-finite sanitisation in `step_zoom` is correct.** `f32::NAN` and
  `f32::INFINITY` both fail `is_finite()` and short-circuit to `1.0`.
  Direct verification: `f32::NAN.is_finite() == false`,
  `f32::INFINITY.is_finite() == false`. The test asserts both branches.
- **Defence-in-depth at the renderer is incomplete for NaN.** Concern,
  not blocking. `lightbox.rs:212` does
  `params.zoom_factor.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR)`. Per
  Rust stdlib: `f32::NAN.clamp(0.25, 8.0) == NaN` (verified by running
  a small rustc binary). The `clamp` only protects against out-of-range
  finite inputs; a NaN passes straight through. Internal callers in
  this repo can never set NaN because `step_zoom` sanitises and `new`
  / `reset_per_image_state` hard-init to `1.0`, so this is currently
  unreachable. But `Params::zoom_factor` is `pub` and could be set by
  a future external caller (e.g. a downstream embedder); a poisoned
  float would NaN-propagate into
  `ConstrainedBox::with_max_width(native_size.x() * NaN) = NaN`, and
  `constraint.max.min(NaN)` returns NaN as well, corrupting the layout.
  Suggestion in the section below.
- **Negative inputs are contained but lose direction-of-step
  semantics.** `is_finite(-1.0) == true`, so a negative zoom passes
  the sanitiser; `(-1.0) * 1.5 = -1.5` then clamps to `0.25`. The
  state is recovered (no NaN escape, no panic), but a `ZoomIn`
  keystroke from a negative state produces *zoom-out* visually
  (negative magnitude got clamped up to MIN). In the current closed
  call graph this is unreachable (zoom is `1.0`-init and only mutated
  through `step_zoom` and the explicit `1.0` reset paths), so this is
  a future-resilience nit rather than a bug.
- **`zoom_factor` reset paths are correct.** Verified all four paths:
  `LightboxView::new` (line 112: direct `zoom_factor: 1.0`),
  `update_params` (line 125 calls `reset_per_image_state`),
  `NavigatePrevious` (line 363), `NavigateNext` (line 370). The fifth
  candidate, `update_image_at`, does **not** reset, and that is
  correct: `update_image_at` overwrites the source slot at a given
  index without changing `current_index` (its caller is the post-load
  rewrite spawned from `start_asset_load`, which fires when the load
  completes / fails; this is a load-state transition, not a
  user-driven image change). Resetting zoom at that callback would
  yank a user-set zoom away the moment a load resolves, which is
  wrong. The current implementation matches the doc comment on
  `reset_per_image_state` ("called from every site that changes
  *which image is currently displayed*"). Good.
- **`handle_action` is exhaustive without a wildcard arm.** All six
  variants (`Dismiss`, `NavigatePrevious`, `NavigateNext`, `ZoomIn`,
  `ZoomOut`, `ZoomReset`) appear as explicit arms (lightbox_view.rs
  lines 357–393). A future variant addition will be a compile error.
- **Modal capture: no key conflict.**
  `grep -rn 'FixedBinding::new\b' app/src/workspace/` outside
  `lightbox_view.rs` finds no binding on plain `=`, `-`, or `0`
  (workspace zoom is `cmdorctrl-=` / `cmdorctrl-0` per
  `app/src/util/bindings.rs:295-298`, modifier-required). Bindings are
  scoped to `id!(LightboxView::ui_name())`, the lightbox is full-window
  modal with the scrim intercepting input, and the action chain walks
  `LightboxView` first. No conflict.
- **`MAX_ZOOM_FACTOR = 8.0` is effectively non-binding for large
  images.** Concern, not blocking. `ConstrainedBox::layout` does
  `constraint.max = constraint.max.min(self.constraint.max)` — it
  *caps* the max constraint but never exceeds the parent's available
  size. With `Image::contain()` (image.rs:260, ratio = `min(ratio_x,
  ratio_y)`), the image scales to fit inside the bounding-box ∧
  parent-window intersection. So for a 4K image in a 1080p window:
  the image already filled the window at `zoom = 1.0`; `ZoomIn`
  multiplies the bounding box, but `constraint.max.min(...)` clamps
  the layout back down to window size, and the visual result is
  identical. Zoom-in is *only visually effective for images
  smaller than the window* (and zoom-out always works because it
  shrinks the cap below window size). This is exactly why §698
  bundled `+ - 0` with `drag-to-pan`: magnifying a 4K image to 8x
  is only useful with pan. The deferral correctly identifies the
  pan-element blocker, but the t2-7 tracking row should make explicit
  that zoom-in for already-window-sized images is a *visual no-op*
  until pan ships, so reviewers don't read this as a regression. See
  Suggestions.
- **`Image::contain()` interaction.** `image.rs:255-271` resolves the
  rendered image to `original * min(ratio_x, ratio_y)` where
  `ratio_{x,y} = dest_{x,y} / original_{x,y}` and `dest` is the
  `ConstrainedBox` size. A box of `2 × native_size` produces ratio 2
  in both axes ⇒ image renders at 2x. The "image renders at native
  size, surrounded by a larger empty box" interpretation does not
  apply. Behaviour matches the impl's mental model.
- **Layout-fragile keys.** Nit. Spec says `+`. The diff binds `=`
  (unmodified) and `shift-=` (which on US-style ANSI keyboards is
  literally the `+` key). On non-US layouts this is fragile: e.g.
  on a German DE-ISO layout `+` is unshifted (its own physical key),
  `=` requires a shift modifier on a different position, and
  `shift-=` doesn't reach `+`. The lightbox modal will accept these
  keystrokes literally. The workspace-level zoom binding has the same
  property (`bindings.rs:296` is `cmdorctrl-=`), so this matches an
  existing convention in the codebase rather than introducing a new
  fragility — but the convention itself is layout-fragile and the
  spec text says `+`. Worth a callout.
- **`reset_per_image_state` ownership boundary.** The helper resets
  both §697's `animation_start_time` and §698's `zoom_factor`. This is
  the right factoring: both pieces of state are scoped to "the
  currently-displayed image" and have identical reset triggers. The
  code comment (lines 128-132) is accurate.
- **Performance.** Each zoom keystroke calls `ctx.notify()`, which
  rebuilds the render tree. `Image` re-resolves its `AssetSource`
  through `AssetCache` on each render; the cache is keyed on the
  source so the post-load payload is hit, not re-decoded. The
  expensive op is the per-paint resampling inside `Image`, which
  `contain()` already does and which scales with output pixels (the
  commit message correctly notes this). At 8x on a typical screenshot
  the cost is bounded by the window constraint anyway (see the
  `MAX_ZOOM_FACTOR` finding above), so there's no realistic explosion.
  Nothing to flag.

# What I checked

- `git show 6aee220` and `git show --stat 6aee220` for the commit
  diff, message, and surface area.
- `specs/GH9729/tech.md:698` for the verbatim spec.
- `crates/warpui_core/src/elements/` listing and a structural-name
  grep (`pub struct (Translate|Offset|Transform)\b`) to verify the
  pan-deferral rationale.
- `crates/warpui_core/src/elements/clipped.rs:49-67` to confirm
  `Clipped` does not bias the child's paint origin.
- `crates/warpui_core/src/elements/image.rs:240-272` for
  `dimensions()` / `FitType::Contain` semantics.
- `crates/warpui_core/src/elements/constrained_box.rs:62-74` for
  `ConstrainedBox::layout` constraint composition.
- `app/src/workspace/lightbox_view.rs:18-39` (binding registration),
  `:65-79` (action enum), `:81-99` (struct), `:104-114` (`new`),
  `:116-126` (`update_params`), `:128-136` (`reset_per_image_state`),
  `:138-156` (`update_image_at`), `:217-241` (`step_zoom` +
  `ZoomDirection`), `:332` (renderer wire-up of `zoom_factor`),
  `:355-395` (action handler), and `:461-509` (the five new tests).
- `crates/ui_components/src/lightbox.rs:18-31` (constants),
  `:116-130` (`Params::zoom_factor`), `:208-225` (renderer-side
  clamp + multiply).
- `app/src/util/bindings.rs:295-298` for the `cmdorctrl-=` / `-0`
  workspace zoom convention referenced in the diff comment.
- A scratch `rustc -O /tmp/test_clamp.rs` to verify the f32 facts:
  `NaN.clamp(0.25, 8.0) = NaN`, `INF.clamp(0.25, 8.0) = 8.0`,
  `(-1.5).clamp(0.25, 8.0) = 0.25`.
- A grep over `app/src/workspace/` and `crates/` for
  `FixedBinding::new("-")` / `("0")` to verify modal-capture
  non-conflict.

# Suggestions

- **Belt-and-braces NaN guard at the renderer.** Replace the
  `lightbox.rs:212` clamp with a sanitiser that mirrors `step_zoom`:

  ```rust
  let zoom = if params.zoom_factor.is_finite() {
      params.zoom_factor.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR)
  } else {
      1.0
  };
  ```

  This closes the public-API surface against an external caller
  passing NaN through `Params`, with no behaviour change for the
  in-tree call sites. Optional — no in-tree caller can hit it today.
- **Document the "zoom-in is a visual no-op for window-sized images"
  limitation.** Add a one-line note either to the
  `Params::zoom_factor` doc-comment in `crates/ui_components/src/lightbox.rs`
  or to `TIER2_TODO.md::t2-7` ("zoom-in only takes visible effect
  when the image is currently smaller than the window; full zoom-in
  on large images requires `t2-7-pan`"). Without this note, a
  reviewer pressing `=` on a 4K screenshot in a 1080p window will
  see no change and assume the keybinding is broken.
- **Negative-input nit.** `step_zoom` could collapse `current <= 0.0`
  to `1.0` alongside the `!is_finite()` arm to remove the
  direction-of-step ambiguity for any future caller that ends up
  with a negative zoom. Trivial code change; not load-bearing today.
- **Layout-fragility comment.** The §698 binding comment on
  `lightbox_view.rs:30-34` could mention that `=` / `shift-=` reach
  `+` on US-style keyboards specifically (matching the existing
  workspace `cmdorctrl-=` convention) and that non-US keyboards
  fall through. This is a doc-only nit.
