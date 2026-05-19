---
item: tier2-t2-20
commit: c102817
reviewer: R2-quality
spec_ref: tech.md §698
verdict: pass-with-nits
---

# Findings

- [pass] Pattern alignment is real, not gestured. `Arc<Mutex<Option<Vector2F>>>` literally mirrors `pub type MouseStateHandle = Arc<Mutex<MouseState>>;` (`crates/warpui_core/src/elements/hoverable.rs:160`), which is the canonical persistent-state idiom the `ui_components` crate-level doc already prescribes for components rebuilt every render (`crates/ui_components/src/lib.rs:13-18`, `:71`). Choosing it over a lighter primitive (`Rc<Cell<Option<Vector2F>>>` / `Rc<RefCell<…>>`) is defensible specifically because `Button` and `Switch` already standardize on the `Arc<Mutex<…>>` flavor here; deviating would create a one-off.
- [pass] Persistence model is correct. The field is placed on the persistent `Lightbox` (component) struct, not on `LightboxView` or on the per-render `PanClippedImage` — matching how `Button` holds `mouse_state: MouseStateHandle` on the persistent component (`crates/ui_components/src/button.rs:18-22`). `Lightbox::render` clones the handle into the rebuilt `PanClippedImage`, so the inner element keeps a view into shared state without ever owning it.
- [pass] Doc-comments are unusually good for this kind of fix. Both the `Lightbox::drag_state` field (`:431-439`) and the `PanClippedImage::drag_state` field (`:87-92`) cite t2-20, name the t2-19 bug they fix (re-render losing the prior position), and contrast with the transient `origin` / `viewport_size` fields that legitimately live on the element. This is exactly the "comment cites the bug it prevents" quality bar a future reader needs to avoid undoing the change.
- [pass] Scope is tight. The transient layout/paint fields (`origin`, `viewport_size`) were correctly left alone — they are written and read within a single layout/paint cycle and don't need to survive re-renders. Resisting the temptation to lift them too is the right call.
- [nit] No type alias for the handle. `Button` hides the primitive behind `MouseStateHandle`; this commit leaves `Arc<Mutex<Option<Vector2F>>>` spelled out at three sites (the `Lightbox` field, the `PanClippedImage` field, the `new()` parameter). A local `type DragStateHandle = Arc<Mutex<Option<Vector2F>>>;` (or named newtype) would make the intent self-documenting and shorten signatures, matching the convention the rest of the crate follows. Pure ergonomics, not a correctness issue.
- [nit] The `lock()` / read / re-`lock()` / write dance in the `LeftMouseDragged` and `LeftMouseUp` arms (`:243-256`, `:261-272`) takes the mutex twice per event when once would do — e.g. `if let Ok(mut state) = self.drag_state.lock() { if let Some(last) = *state { … *state = Some(*position); } }`. Functionally identical (the UI is single-threaded so no contention is possible mid-handler), just less re-entrant-looking. Worth a follow-up tidy.
- [nit] `if let Ok(mut state) = self.drag_state.lock()` swallows a poisoned-mutex silently in three places. For UI-thread-only state this is the same defensive shape `Hoverable` callsites use (`state.lock().unwrap()` in `hoverable.rs:188`), so the choice is internally inconsistent within the codebase rather than wrong. Either `expect("drag_state mutex poisoned")` for a fail-fast (matches `Hoverable`) or a single `// poisoned mutex is unreachable on the UI thread` comment would resolve the inconsistency.
- [minor] No unit test was added for the persistence property. The whole point of the commit is "state survives a `ctx.notify()` round-trip"; that's a unit-testable invariant — exercise `LeftMouseDown` → drop and rebuild the `PanClippedImage` with the same `drag_state` handle → `LeftMouseDragged` → assert `on_pan` saw a non-zero delta. The commit message says "lightbox_view tests untouched, 18/18 passing", which only shows the regression isn't observable from existing tests — exactly the gap the bug exploited in t2-19. A test would prevent a future refactor that "simplifies" the handle back to a struct field from silently re-introducing the freeze.

# What I checked

- `git show c102817` — full diff, commit message, scope.
- `tech.md` line 698 — the "drag-to-pan" deliverable this item delivers under.
- `Button::MouseStateHandle` definition (`crates/warpui_core/src/elements/hoverable.rs:160`) and usage (`crates/ui_components/src/button.rs:18-34`, `crates/ui_components/src/switch.rs:21-24`).
- `ui_components` crate-level doc on persistent-component state (`crates/ui_components/src/lib.rs:13-90`).
- `Arc<Mutex<…>>` vs `RefCell<…>` precedent in the immediate neighborhood (`crates/warpui_core/src/elements/event_handler.rs:47-57` uses `RefCell<Handler>` for handlers but `Arc<Mutex<…>>` is the established convention for mouse/interaction state).
- The final `lightbox.rs` shape after the commit (`Lightbox` field, `PanClippedImage` field, the three event-arm handlers, the `Component::render` clone site at `:634`).
- That no new unit test or integration test was added alongside the fix.

# Suggestions (Deferred R2 follow-up)

1. Introduce `type DragStateHandle = Arc<Mutex<Option<Vector2F>>>;` (or a small newtype) in `lightbox.rs` and use it in the field declarations, `PanClippedImage::new` signature, and the `Component::render` clone site. Mirrors `MouseStateHandle`.
2. Collapse the double-lock-per-event pattern in `LeftMouseDragged` / `LeftMouseUp` to a single `if let Ok(mut state) = …` scope.
3. Decide between `expect("drag_state mutex poisoned")` and a comment justifying the silent `Ok(mut state) = …` fallthrough; align with whichever convention the rest of the lightbox file ultimately settles on.
4. Add a unit test in `lightbox.rs` that constructs two successive `PanClippedImage` instances sharing one `drag_state`, dispatches `LeftMouseDown` to the first and `LeftMouseDragged` to the second, and asserts the `on_pan` callback fires with a non-zero delta. Locks in the persistence invariant against future refactors.
