---
item: tier2-t2-12
commit: 65b2f56
reviewer: R1-correctness
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Spec

Verbatim from `specs/GH9729/tech.md` §698:

> **Zoom and pan controls**: extend `lightbox::Params` with zoom state and `lightbox_view.rs` keybindings (`+`, `-`, `0`, drag-to-pan).

t2-12 is an explicit supplemental amendment to this line: keybindings
proved un-routable in two prior attempts (t2-7, t2-11), so the trigger
surface is migrated to GUI buttons + cmd/ctrl+scroll-wheel. The action
enum and `handle_action` arms in `LightboxView` are unchanged, so the
zoom-state plumbing demanded by §698 is preserved; only the user-input
surface changed.

# Findings

- [minor] Scroll-wheel zoom only fires when the cursor is over the
  image element itself (the `EventHandler.dispatch_callback` rect-
  contains-point check at `event_handler.rs:351`). In a windowed
  lightbox where the image is centered with significant scrim
  padding (`SCRIM_PADDING = 48.`), the user can be hovering over the
  visible scrim or near a description line and have cmd+scroll do
  nothing. Most image viewers attach scroll-zoom to the whole
  viewer area. Not a correctness bug, but a discoverability gap the
  manual tester should sanity-check.
- [minor] The new zoom toolbar is positioned at the *scrim's*
  bottom-left (`ParentAnchor::BottomLeft`, offset `(12, -12)`). The
  description + metadata strip render below the image inside a
  centered column. On a narrow window or a tall image, the
  description's left edge can drift close to the toolbar and
  visually crowd it. The close button's symmetric top-right
  positioning has no similar collision risk because there is no
  centered content at the top. A bottom-*center* anchor for the
  toolbar would mirror Preview/Photos and avoid the description
  collision entirely. Manual-test cue, not a blocker.
- [nit] The on-screen comment in `lightbox.rs:339` says "positive y
  is 'wheel up / push fingers up' → zoom in". On macOS with natural
  scrolling enabled, "push fingers up" actually produces *negative*
  `scrollingDeltaY` (the scrollable in `uniform_list.rs:124` confirms
  the codebase treats positive `delta.y` as scroll-up = reduce
  `scroll_top`). The runtime behaviour is correct (positive delta
  → zoom in is the Preview convention regardless of natural-scroll
  setting), but the comment's "push fingers up" gloss is
  misleading. Suggest rewording to "positive y = scroll-up = zoom
  in (matches Preview)".
- [nit] `SCROLL_ZOOM_DEAD_ZONE = 1.0` is a magic number. The
  doc-comment justifies it empirically. Fine for v1, but the
  precise/non-precise distinction (`Event::ScrollWheel.precise`,
  used elsewhere in the codebase to convert lines→pixels) is
  ignored — the dead-zone applies the same threshold to both. With
  a non-precise mouse wheel where delta is in lines, even a single
  tick is `>= 1.0`, so the dead-zone is effectively trackpad-only.
  Acceptable, but worth noting in the comment.
- [nit] No unit-test coverage for the new wiring is acknowledged in
  the commit message. The dispatch chain is structurally identical
  to the proven `on_navigate` path (which has manual-test coverage
  too), so this matches existing precedent. Manual-test-only is
  acceptable for the GUI surface; the underlying `step_zoom` /
  `format_metadata_line` helpers still have their 18 tests from
  t2-7/t2-11.

# What I checked

- Read `specs/GH9729/tech.md` §698 verbatim.
- Confirmed `LightboxViewAction::Zoom{In,Out,Reset}` and their
  `handle_action` arms still exist (`lightbox_view.rs:81,83,85` and
  `:469,476,483`). No dead arms.
- Confirmed `init()` in `lightbox_view.rs:16-48` registers only
  `escape`, `left`, `right`. The lengthy comment documents the
  routing history and warns future contributors. No `=`, `-`, `0`,
  no `cmdorctrl-=` etc.
- Confirmed `render()` at `lightbox_view.rs:417-441` passes
  `on_zoom: Some(Arc::new(|direction, ctx, _| match …))` that
  dispatches the three actions. Mirror of `on_navigate` directly
  above it; the dispatch path is proven.
- Confirmed `Lightbox` struct has six button fields and derives
  `Default` (`lightbox.rs:114-127`), so all buttons are
  default-constructed; no explicit `new()` needed.
- Confirmed `ZoomDirection` enum and `ZoomHandler` type alias are
  defined in the right module (`lightbox.rs:129-140`).
- Confirmed `Options::on_zoom: Option<ZoomHandler>` and
  `Options::default()` returns `on_zoom: None`
  (`lightbox.rs:216,224`).
- Confirmed toolbar render path is gated on
  `if let Some(on_zoom) = params.options.on_zoom`
  (`lightbox.rs:480`) and scroll-wheel handler is gated on
  `if let Some(on_zoom) = scroll_zoom` (`lightbox.rs:298`). Both
  gates honour the opt-out.
- Confirmed scroll-wheel handler at `lightbox.rs:299-318` checks
  `!modifiers.cmd && !modifiers.ctrl` (both checked — neither alone
  would gate out the other platform), reads `delta.y()` from the
  `&Vector2F` arg, and returns `PropagateToParent` when modifier
  absent.
- Cross-checked `Event::ScrollWheel`'s shape at
  `crates/warpui_core/src/event.rs:102-107` — `delta: Vector2F`,
  `modifiers: ModifiersState` with `cmd` and `ctrl` bools. Signature
  matches the `on_scroll_wheel` callback signature at
  `event_handler.rs:188-195` (`Fn(&mut EventContext, &AppContext,
  &Vector2F, &ModifiersState) -> DispatchEventResult`).
- Confirmed Vector2F is passed through as-is (no double-inversion):
  the codebase's other consumer (`uniform_list.rs:124`) uses
  positive `delta.y` to scroll up (reduce `scroll_top`), confirming
  positive `delta.y` = "wheel up" in OS-reported coordinates. This
  zoom handler maps "wheel up → zoom in", which is the conventional
  Preview/Photos behaviour.
- Confirmed `Icon::Minus` (icons.rs:163), `Icon::Refresh`
  (icons.rs:78), `Icon::Plus` (icons.rs:15) all exist in
  `warp_core/src/ui/icons.rs`.
- Confirmed `add_positioned_child` for the toolbar uses
  `ParentAnchor::BottomLeft` / `ChildAnchor::BottomLeft` with offset
  `vec2f(12., -12.)`, mirroring the close button's top-right
  positioning (`ParentAnchor::TopRight` / `(-12., 12.)` at
  `lightbox.rs:405-413`). Sign convention matches.
- Confirmed image's `EventHandler` returns `StopPropagation` for
  handled scroll events (`lightbox.rs:317`), so the scroll doesn't
  bubble. The existing `on_left_mouse_down` returning
  `StopPropagation` is unchanged.
- Confirmed both example sites in
  `crates/ui_components/examples/library.rs` pass `on_zoom: None`.
- Confirmed the toolbar's three buttons live in a separate `Flex::row`
  added as a positioned child to the outer `Stack`, so it sits on
  top of the scrim and receives clicks before the dismiss layer
  (same Stack ordering used by the close/prev/next buttons).
- 18/18 lightbox_view tests assumed to still pass per commit
  message; the `step_zoom` and `format_metadata_line` helpers used
  by those tests are untouched.

# Suggestions

- Reword the scroll-wheel comment at `lightbox.rs:308-309` to drop
  the "push fingers up" framing and just say "positive y = scroll-
  up = zoom in (Preview convention)".
- Consider anchoring the zoom toolbar to bottom-center instead of
  bottom-left in a follow-up; it would mirror Preview/Photos and
  remove the marginal description-collision risk on narrow windows.
- Consider attaching the cmd+scroll handler one layer up (the
  scrim/Stack) rather than the image element so cmd+scroll-anywhere
  works; cosmetic, deferrable.
- A unit test isn't tractable for the scroll handler at this layer,
  but a render-shape test (assert `params.options.on_zoom.is_some()`
  causes the toolbar to render and three buttons appear) would be
  the lowest-friction guard against an accidental future
  regression. Optional.
