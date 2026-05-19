---
item: tier2-t2-20
commit: c102817
reviewer: R1-correctness
spec_ref: tech.md §698
verdict: pass-with-nits
---

# Spec

`tech.md` §698 deferred drag-to-pan, delivered as a chain ending at
t2-19 ("custom `PanClippedImage` lets zoom grow past viewport"). t2-19
shipped pan but stored `last_drag_position` on the per-render
`PanClippedImage` element; every `Pan` action's `ctx.notify()` rebuilt
the element with `None`, so the second `LeftMouseDragged` saw no prior
position and dragging froze after one imperceptible step. t2-20's job is
narrow: hoist drag state onto the persistent `Lightbox` struct via
`Arc<Mutex<Option<Vector2F>>>` (the same shape `Button`'s
`MouseStateHandle = Arc<Mutex<MouseState>>` and `DragResizeElement`
already use) so state survives the re-render.

# Findings

- **[pass] Spec-faithful, minimal fix.** `drag_state:
  Arc<Mutex<Option<Vector2F>>>` lives on `Lightbox`
  (`lightbox.rs:439`), is cloned into `PanClippedImage` on each
  `render()` (`lightbox.rs:634`), and `PanClippedImage::dispatch_event`
  reads/writes via the shared lock on all three arms (down/dragged/up).
  Across the inevitable `ctx.notify()` rebuild between consecutive
  `LeftMouseDragged` events, the new `PanClippedImage` instance
  receives the same `Arc<Mutex<…>>`, so the second-and-later deltas
  observe the position written by the first. The t2-19 R1
  [blocking] finding is closed.
- **[pass] Pattern matches the framework's idiomatic precedent.**
  `MouseStateHandle = Arc<Mutex<MouseState>>` in
  `warpui_core/src/elements/hoverable.rs:160`, used by `Button`
  (`ui_components/src/button.rs:20`) and `Switch`
  (`ui_components/src/switch.rs:23-24`), and the documented
  "stateful element" pattern in `ui_components/src/lib.rs:17, 71`
  all confirm the same shape — persistent struct holds an
  `Arc<Mutex<…>>` handle, cloned into transient elements per render.
- **[pass] Lock poisoning is handled gracefully.** All five
  `.lock()` call sites use `.ok()` / `if let Ok(mut state) = …`
  (`lightbox.rs:230, 238, 244-248, 254, 263-267, 269`); none use
  `.unwrap()`. A poisoned mutex (only possible if a previous holder
  panicked mid-write) silently degrades to "no drag" — drag breaks
  for the rest of the session but no further panic. Defensible for
  a UI-only element. Note that `Hoverable::state()`
  (`hoverable.rs:353`) does use a raw `MutexGuard` (effectively
  `.unwrap`); the t2-20 stance is strictly safer.
- **[pass] `#[derive(Default)]` correctly initialises `drag_state`
  to `Arc::new(Mutex::new(None))`.** `lightbox.rs:418` keeps the
  existing derive; `Arc<Mutex<Option<T>>>` defaults compose
  (`Arc::default()` → `Arc::new(T::default())`, `Mutex::default()`
  → `Mutex::new(T::default())`, `Option::default()` → `None`). So
  every fresh `Lightbox` starts with a clean handle without any
  explicit `fn new`. No ordering/initialisation bug.
- **[pass] Same-frame double-lock is safe.** The `LeftMouseDragged`
  arm acquires the lock to read, releases (the
  `.ok().and_then(|state| *state)` chain drops the `MutexGuard`
  before the `if let Some(last) = last` body runs), then
  re-acquires to write. No risk of self-deadlock because `Mutex`
  is not re-entrant but the guard is released between the two
  calls. Two acquisitions per drag delta is a tolerable cost
  (single-threaded UI path).
- **[pass] Fresh `LightboxView::new` / close-and-reopen path.**
  `app/src/workspace/lightbox_view.rs:140` calls
  `lightbox::Lightbox::default()` in `LightboxView::new`, which
  yields a fresh handle. The `Lightbox` is owned by `LightboxView`
  (`lightbox_view.rs:107`), so closing and re-creating the view
  reseeds `drag_state` to `None`. No state leak across openings.
- **[pass] LeftMouseUp clears the handle.** The up arm sets
  `*state = None` whenever `was_dragging` is true
  (`lightbox.rs:268-272`), so a complete down/drag/up cycle leaves
  the handle in its starting condition. The `click_count >= 2`
  double-tap arm at `lightbox.rs:230` also clears state, matching
  the t2-21 wiring done in the next commit.
- **[minor] Navigation / `update_params` mid-drag does not clear
  `drag_state`.** `LightboxView::reset_per_image_state`
  (`lightbox_view.rs:165-169`) zeroes `animation_start_time`,
  `zoom_factor`, and `pan_offset`, but does not touch
  `self.lightbox.drag_state` (no setter exists; the field is
  private). If a user is mid-drag and a keystroke or async refresh
  triggers `reset_per_image_state`, the next `LeftMouseDragged`
  on the new image will compute a delta from the previous image's
  cursor position. Practically unreachable through any current UI
  affordance (no navigation hotkey fires mid-`LeftMouseDragged`
  inside the lightbox window) but worth a `pub fn
  reset_drag_state(&self) { *self.drag_state.lock() = None; }` for
  hygiene if/when async-driven image swaps land.
- **[minor] Two lock acquisitions per LeftMouseUp.** The up arm
  locks once to read `is_some()`, then locks again to write `None`
  — could be one `if let Ok(mut state) = … { if state.is_some() {
  *state = None; return true; } }`. Tiny perf win, slight clarity
  win.
- **[minor] No new unit test covers persistence.** Element-tree
  integration testing in this framework needs a presenter harness,
  which `PanClippedImage` (per t2-19 R1) is not equipped with. The
  shape is small enough that the precedent of `DragResizeElement`
  and `MouseStateHandle` shipping without unit tests is acceptable.
  A doc-test or a hand-rolled mock of the `Lock → Render → Lock`
  cycle would still be a cheap regression guard.
- **[carry-over major, NOT t2-20's scope] `pan_offset` is still
  stored unclamped in `LightboxView`.** Confirmed in current tree
  at `lightbox_view.rs:588-589`: only `is_finite()` is checked,
  then `self.pan_offset = next`. The direction-reversal deadband
  documented in t2-19 R1 [major] is still live. t2-20's spec
  bullet does not claim to address it, and the commit description
  is honest about scope — but the verdict on the t2-19/t2-20 pair
  in aggregate cannot rise above "pass-with-nits" until the
  carry-over is closed (next-tier item, presumably).
- **[carry-over major, NOT t2-20's scope] Whole-viewport hit-test
  still suppresses scrim Dismiss.** Confirmed at `lightbox.rs:216`
  — the `LeftMouseDown` arm still gates on `point_in_viewport`,
  not on image bounds. Identical to t2-19. Carry-forward.
- **[carry-over minor, NOT t2-20's scope] Child never sees
  events** (`PanClippedImage::dispatch_event` never calls
  `self.child.dispatch_event(…)`). Unchanged from t2-19. The
  element-contract violation is the same scope.
- **[pass] Tests untouched and passing per commit message
  ("lightbox_view tests untouched, 18/18 passing").** The diff
  only touches `crates/ui_components/src/lightbox.rs` and the
  tracker, so the existing `lightbox_view` suite still exercises
  the parent path; the per-element pan/drag-state path has no
  direct test, in keeping with framework precedent.

# What I checked

- `git show c102817 --stat` and the full diff (single file:
  `crates/ui_components/src/lightbox.rs`).
- `crates/ui_components/src/lightbox.rs:82-117` (PanClippedImage
  struct + ctor with new `drag_state` param) and
  `:205-275` (dispatch_event arms post-fix, with the t2-21
  double-tap branch layered on top).
- `crates/ui_components/src/lightbox.rs:418-440` for the
  `#[derive(Default)]` on `Lightbox` and the new `drag_state`
  field.
- `crates/ui_components/src/lightbox.rs:585-634` for the
  `Lightbox::render` call site that clones `self.drag_state` into
  `PanClippedImage`.
- `crates/warpui_core/src/elements/hoverable.rs:14, 54-95, 160,
  184, 353` for `MouseStateHandle = Arc<Mutex<MouseState>>` —
  the framework's canonical "persistent state handle" pattern.
- `crates/ui_components/src/button.rs:20` and
  `crates/ui_components/src/switch.rs:23-24` for the equivalent
  field declarations on persistent components.
- `crates/ui_components/src/lib.rs:17, 71` for the documented
  expectation that stateful elements own a persistent
  `MouseStateHandle`-style handle and clone it into the per-render
  tree.
- `app/src/workspace/lightbox_view.rs:107, 140, 150-169` for
  `Lightbox` ownership, `LightboxView::new`'s `Lightbox::default()`
  call, and `reset_per_image_state` (which does not touch
  `drag_state`).
- `app/src/workspace/lightbox_view.rs:585-590` to confirm the
  carry-over [major] from t2-19 R1 (unclamped `pan_offset`) is
  still live post-t2-20.
- `crates/warpui_core/src/elements/drag_resize.rs:14-90` to cross-
  check the idiomatic `Arc<Mutex<…>>` drag-state shape.
- `specs/GH9729/reviews/tier2-t2-19-r1.md, tier2-t2-19-r2.md` for
  the prior-issue context, especially the [blocking] finding
  t2-20 was scoped to close.

# Suggestions

- **Add `Lightbox::reset_drag_state(&self)`** (or expose a setter
  invoked from `LightboxView::reset_per_image_state`) so that
  future async-driven image swaps mid-drag don't leak a stale
  cursor position. Cheap insurance; tiny surface area.
- **Collapse the two-lock LeftMouseUp pattern.** One acquisition:
  `if let Ok(mut state) = self.drag_state.lock() { if
  state.is_some() { *state = None; return true; } }`. Reads
  clearer, drops one redundant lock.
- **Cleanup pass: drop the `last_drag_position` field comment
  if/when t2-19's diff comments are rebased away.** The t2-20
  doc-comment on `drag_state` already explains the why; no need
  to leave the t2-19 corpse in tree commentary.
- **Carry-overs (out of t2-20 scope, repeat from t2-19 R1):**
  clamp `pan_offset` before storing in `LightboxView`; hit-test
  image bounds rather than viewport; forward events to child;
  forward `parent_data`. These are tracked in the t2-19 R1
  suggestions list; flagging here only so reviewers of the
  t2-19/t2-20 pair don't think t2-20 silently closed them.
