---
item: tier2-t2-19
commit: 67f014b
reviewer: R1-correctness
spec_ref: tech.md §698 (fully addressed per bullet)
verdict: concerns
---

# Spec

`tech.md` line 698 (deferred follow-up bullet): "Zoom and pan controls:
extend `lightbox::Params` with zoom state and `lightbox_view.rs`
keybindings (`+`, `-`, `0`, drag-to-pan)." The t2-7-r1 review documented
the load-bearing gotcha that closed-loop zoom (t2-7 → t2-18) bounced
off without addressing: `ConstrainedBox::layout` clamps its child's
max by the parent's max (`constrained_box.rs:70`), so an image already
fit-window-size at zoom 1.0 cannot grow visibly via the layout path.
t2-19 finally addresses §698 in full by introducing a custom
`PanClippedImage` element that strict-sizes the child past the parent
max, paints clipped to the viewport, and wires drag-to-pan plus the
relocated cmd+scroll-zoom into a single hit-test region.

# Findings

- **[pass] The strict-sizing trick is correct, and provably defeats
  the t2-7-r1 gotcha.** `PanClippedImage::layout` passes
  `SizeConstraint::strict(self.desired_size)` to the child
  (`lightbox.rs` new element, layout fn). `presenter.rs:766-771`
  shows `strict` sets both min and max to the same vector. `Image`'s
  `layout` returns `constraint.max` verbatim (`image.rs:281-282`), so
  `child.size()` is `zoom * native_size` regardless of the
  `PanClippedImage`'s own `constraint.max`. Compared to the pre-t2-19
  `ConstrainedBox(image_builder).with_max_*(zoom*native)` shape:
  `ConstrainedBox::layout` does `constraint.max =
  constraint.max.min(self.constraint.max)`
  (`constrained_box.rs:70`) which is precisely the line that defeated
  every earlier polish round. `PanClippedImage` is the right shape.
- **[pass] Centering and pan-clamp math are correct.** Child origin
  is `viewport_origin + (viewport - child)/2 + clamp(pan)`. The
  half-extent `max_pan = max(0, (child - viewport) / 2)` correctly
  collapses to zero when child fits the viewport (zoom == 1.0 on a
  small image, or any zoom where the aspect-constrained child is
  smaller along an axis), and the per-axis clamp pins the visible
  edge to viewport centre at the boundary. The arithmetic does not
  rely on `child > viewport`.
- **[pass] Clip-layer mechanics are well-aligned with the existing
  `Clipped` element pattern.** `scene.rs:441-457` shows
  `visible_rect` returns the intersection of `(origin, size)` with the
  active layer's clip bounds, then `ClipBounds::BoundedBy(visible)`
  starts a new layer pinned to that already-clamped rectangle
  (`scene.rs:488-505`). This matches `clipped.rs:50-66` line-for-line.
  Drop shadows and transparent borders on the child render only inside
  the started layer, so they correctly clip at viewport — no scrim
  bleed.
- **[blocking] Drag state is on the per-render element struct, so
  drag breaks after the first delta.** `last_drag_position:
  Option<Vector2F>` lives on `PanClippedImage` itself. The element is
  rebuilt from `Component::render` every `ctx.notify()`, including
  the `notify()` the new `Pan` action triggers in `lightbox_view.rs:
  588-590`. So `LeftMouseDown` sets `last_drag_position`; the first
  `LeftMouseDragged` computes a delta and dispatches `Pan`; that
  re-renders the whole component; the second `LeftMouseDragged` hits
  a freshly-constructed `PanClippedImage` with `last_drag_position
  == None`, falls into the `else` and returns false; subsequent
  drag is silently dropped. This is the exact same `Arc<Mutex<_>>`-
  shaped mistake that `DragResizeElement` already avoids
  (`drag_resize.rs:14-18, 87-90`) and that `Button`'s
  `MouseStateHandle` avoids. **Confirmed fixed in t2-20 (`c102817`)**
  by hoisting `drag_state: Arc<Mutex<Option<Vector2F>>>` onto the
  persistent `Lightbox` struct. Flagging here as the load-bearing
  defect of this commit-in-isolation; if t2-19 had shipped without
  t2-20, drag would have been visibly broken.
- **[major] `pan_offset` is stored unclamped in `LightboxView`, only
  clamped at paint, which produces a "deadband" on direction reversal
  past the clamp.** `LightboxViewAction::Pan` handler at
  `lightbox_view.rs:585-590` stores `next` verbatim after only a
  finiteness check; `PanClippedImage::paint` clamps for rendering.
  The on_pan callback dispatches `self.pan_offset + delta` from the
  raw stored value (`PanClippedImage::dispatch_event` /
  `LeftMouseDragged` arm), so after a long drag past the boundary
  the raw offset can be e.g. `2000` with `max_pan == 400`. When the
  user reverses direction, the raw offset has to drift back from
  `2000` to `400` before any visible motion resumes — the image
  appears frozen for that interval. The commit comment at
  `lightbox_view.rs:582-584` even claims "the model state to mirror
  the clamped value so subsequent drag deltas accumulate sanely",
  but the implementation does not actually clamp before storing.
  Fix is one line: clamp before assignment, or store the clamped
  paint offset back via a feedback path. Note: this finding is
  **not** addressed by t2-20.
- **[major] Element-contract violation: child never sees events.**
  The `Element` trait doc at `elements/mod.rs:142-152` is explicit:
  "Each parent Element will unconditionally pass the event to its
  children by calling `dispatch_event` on them, which allows the
  children to make their own determination of whether or not the
  event applies." `PanClippedImage::dispatch_event` never invokes
  `self.child.dispatch_event(...)` on any branch — including the
  catch-all `_ => false`. `DragResizeElement::dispatch_event`
  follows the contract correctly (`drag_resize.rs:120-121` always
  calls `child.dispatch_event` first). In practice the child is a
  raw `Image` element whose `dispatch_event` does nothing
  observable, so no current behaviour breaks. But this is a forward-
  compatibility hazard: any future composition (e.g. an overlay
  badge on the image, an animation control, an a11y-focusable
  region) inside the child tree will silently lose events. The
  contract is described as expectation, not invariant; still worth
  fixing.
- **[major] LeftMouseDown hit-test consumes anywhere inside viewport,
  not anywhere inside the image.** The pre-t2-19 wiring was
  `EventHandler::on_left_mouse_down(StopPropagation)` around the
  image only; clicks on the scrim area outside the image
  bounding box fell through to scrim Dismiss. After t2-19,
  `point_in_viewport` covers the entire viewport rect (which equals
  the lightbox content area passed as `constraint.max`). At
  zoom 1.0 on an image smaller than the window, the image is
  centered with empty scrim-toned space on each side, all of which
  is now part of the viewport rect — so a click on that visually-
  empty area also consumes, suppressing the documented "click-on-
  scrim to dismiss" affordance for the area immediately surrounding
  the image. This is a behaviour regression relative to t2-18. The
  comment at the down arm ("consume so the scrim's Dismiss doesn't
  fire on the same down event") only justifies suppression when the
  click is actually on the image. Suggestion: hit-test against
  `clamp_pan`-adjusted child bounds (origin + child_size) rather
  than the viewport rect.
- **[minor] `LeftMouseDragged` / `LeftMouseUp` arms have no position
  guard.** Once `drag_state` (post-t2-20) holds a position, every
  subsequent drag/up event is consumed regardless of where the
  cursor is. This is correct for drag-tracking semantics (cursor
  can leave viewport mid-drag and the drag must continue), but the
  `LeftMouseUp` arm consumes *any* mouse-up while drag is active,
  which could in principle race with a sibling drag-target that
  also wants the up event. In the current lightbox there is no such
  sibling, so this is theoretical.
- **[minor] `point_in_viewport` uses a manual rect check rather
  than the framework's clip-aware `ctx.visible_rect(...)`.**
  Compare `event_handler.rs:349-357` for the idiomatic pattern.
  Manual checks ignore any active clip-bounds further out, so if
  a future parent element clips the lightbox (e.g. a future
  in-window-overlay design) the hit-test would still accept clicks
  the user can't see. Today the lightbox is full-window-modal with
  the scrim, so this is dormant.
- **[minor] No `parent_data` forwarding to child.** Default returns
  `None`; the child's `parent_data` is invisible to grandparents.
  `Clipped::parent_data` forwards via `self.child.parent_data()`
  (`clipped.rs:88-89`). For the current Image child this is moot,
  but the omission is the kind of detail that bites a year later.
- **[minor] `dy.abs() < SCROLL_ZOOM_DEAD_ZONE` returns `true`.** The
  old t2-12 wiring returned `StopPropagation` in the same case;
  t2-19 preserves that semantically (returns `true`). Correct.
- **[pass] Non-finite sanitisation parity with t2-7-r1's existing
  zoom-factor clamp.** `lightbox_view.rs:585-590` checks
  `next.x().is_finite() && next.y().is_finite()` before assigning
  pan offset, mirroring `step_zoom`'s pattern. NaN can't reach
  `self.pan_offset`. Good.
- **[pass] `ZoomReset` clears `pan_offset`.** Lightbox-view's
  `ZoomReset` arm now also sets `pan_offset = Vector2F::zero()`.
  At zoom 1.0 the image fits the viewport and `max_pan == 0`
  anyway, so without the explicit reset a stored non-zero offset
  would just be paint-clamped to zero — but persisting non-zero
  raw state across a "reset to native" feels wrong and the explicit
  zero is the safer shape.
- **[pass] `reset_per_image_state` resets `pan_offset`.** Navigation
  and `update_params` paths zero the offset. `update_image_at`
  correctly does *not* reset (matches the t2-7-r1 finding that the
  post-load rewrite is a load-state transition, not a user-driven
  image change).
- **[pass] Arithmetic-overflow margin.** `desired_size = native *
  zoom` with `MAX_ZOOM_FACTOR = 8.0` and the largest possible
  native (SVG capped at `MAX_SVG_RENDER_DIMENSION`, raster bounded
  by decoder limits) lands comfortably inside f32. No overflow
  realistic.
- **[pass] Native-size guard.** The `PanClippedImage` branch is
  only entered when both `LightboxImageSource::Resolved` and
  `Some(native_size)` match, so `native_size` is never `None`
  mid-load at this code path. The pre-load branch shows the
  loading element via `before_load(Align(loading_element))`.
- **[pass] Zero native size.** Theoretical — if native_size were
  `(0, 0)`, `desired_size` is zero, the child is laid out at zero
  size, `(viewport - 0)/2` centres at viewport mid, pan_clamp =
  zero, no panic. The strict-size element handles it.
- **[pass] Animated GIF / `paused_at` interaction.** Animated
  images are still wired through `enable_animation_with_start_time`
  on `image_builder` before being boxed into `PanClippedImage`.
  The strict-size constraint flows into `Image::layout`; per-frame
  animation rendering reads `self.size` which equals `desired_size`.
  Animation continues to repaint via the existing
  `paint_animated_image` schedule. No regression.
- **[pass] Tests untouched, 18/18 still pass.** Commit message
  states no new unit tests for the custom element; that is
  defensible because element-tree integration testing in this
  framework requires a presenter harness. The framework's existing
  custom elements (`DragResizeElement`, `Clipped`) likewise lack
  per-element unit tests; the pattern is precedented.

# What I checked

- `git show 67f014b --stat` and full diff
  (`crates/ui_components/src/lightbox.rs`,
  `app/src/workspace/lightbox_view.rs`,
  `crates/ui_components/examples/library.rs`).
- `specs/GH9729/tech.md:698` for the spec bullet, plus
  `tech.md:22` listing pan as deferred.
- `specs/GH9729/reviews/tier2-t2-7-r1.md` for the documented
  `ConstrainedBox::layout` gotcha and the
  `MAX_ZOOM_FACTOR`-is-non-binding-for-large-images finding that
  t2-19 closes.
- `crates/warpui_core/src/elements/constrained_box.rs:62-74` to
  confirm the parent-max-clamp at `constraint.max =
  constraint.max.min(self.constraint.max)`.
- `crates/warpui_core/src/presenter.rs:761-771` for
  `SizeConstraint::strict` semantics.
- `crates/warpui_core/src/elements/image.rs:274-289` to confirm
  `Image::layout` returns `constraint.max` verbatim and stores it
  on `self.size`, which `paint` then feeds to `dimensions(... ,
  bounds, FitType::Contain)`.
- `crates/warpui_core/src/elements/clipped.rs:35-95` for the
  reference clip-layer pattern (`visible_rect → start_layer →
  paint → stop_layer`), confirming PanClippedImage matches it.
- `crates/warpui_core/src/scene.rs:41-54, 441-505` for
  `ClipBounds` and `visible_rect` semantics.
- `crates/warpui_core/src/elements/drag_resize.rs:14-90, 94-173`
  for the idiomatic `Arc<Mutex<_>>` drag-state shape and the
  element-contract-correct "child first, then check own bounds"
  dispatch pattern.
- `crates/warpui_core/src/elements/event_handler.rs:342-358` for
  the idiomatic `ctx.visible_rect(...)` hit-test pattern that
  t2-19 bypasses.
- `crates/warpui_core/src/elements/mod.rs:106-179` for the
  `Element` trait surface, dispatch contract, and the
  `parent_data` default.
- `crates/warpui_core/src/event.rs:72-82` for `ModifiersState`
  fields (`cmd`, `ctrl`) confirming `modifiers_have_cmd_or_ctrl`
  is correct.
- `git show c102817` (t2-20) for the drag-state-on-persistent-
  struct fix, anchoring the [blocking] finding above.
- `app/src/workspace/lightbox_view.rs:585-606` (current tree) to
  confirm `pan_offset` is stored unclamped — the major UX bug is
  still live after t2-20.
- Param-call-site sweep:
  `app/src/workspace/lightbox_view.rs:468`,
  `crates/ui_components/examples/library.rs:559, 599` — all three
  updated to pass `pan_offset` and `on_pan`. No missed sites.

# Suggestions

- **Clamp `pan_offset` before storing in `LightboxView`.** Today
  the value is stored unclamped and only clamped at paint, which
  produces a direction-reversal deadband when the user drags past
  the boundary. Either:
  1. expose `clamp_pan` from the lightbox component and clamp in
     the action handler, OR
  2. have the on_pan callback receive both the proposed and the
     clamped paint offset and dispatch the clamped one, OR
  3. compute clamp in `LightboxView` from `current_image_native_size
     × zoom_factor` and the viewport (less ideal — duplicates
     knowledge).
  Option 1 is the cheapest and matches the existing pattern of
  exposing `MAX_ZOOM_FACTOR` etc. as `pub const`.
- **Hit-test the image, not the viewport.** Use
  `child_origin + child_size` (already computed in paint) for the
  `LeftMouseDown` consumption check, so clicks on the scrim-toned
  empty area around a smaller-than-viewport image continue to
  dismiss. Practically: cache `child_origin` / `child_size` on the
  element from paint, and bound-check against that rect.
- **Forward events to child.** Either prepend
  `let _ = self.child.dispatch_event(event, ctx, app);` (fire-and-
  forget) or follow `DragResizeElement`'s pattern of giving the
  child first refusal and only acting on `LeftMouseDown` when
  `!child_handled`. The latter is the contract-correct shape.
- **Switch `point_in_viewport` to `ctx.visible_rect(origin,
  viewport).map(|r| r.contains_point(p)).unwrap_or(false)`.**
  Matches every other element in this codebase and remains correct
  under any future parent clip.
- **Forward `parent_data`.** `fn parent_data(&self) -> Option<&dyn
  Any> { self.child.parent_data() }`. Cheap, future-proofs against
  flex / stack / grid placement of the lightbox tree.
- **Add a small comment on `t2-19`'s now-historical
  `last_drag_position` field** in the eventual cleanup pass, pointing
  to the t2-20 commit for why the state moved off the element. The
  next time someone touches this code, the rationale is one
  search away.
