# Spec: Prevent overshoot when scrolling top of code diff (GH-9808)

## Problem

While scrolling up in a code diff inside a long blocklist, when
the scroll hits the top of the diff it spills over into scrolling
the parent blocklist. Users want the inner-scroll boundary to
*absorb* the rest of an in-progress scroll gesture — only on
discrete subsequent scrolls should the parent take over.

The same overshoot issue occurs for other blocklist-embedded
scrollable containers (file trees, inline tables, log panels);
the fix in V1 covers all of them.

## Goal

When a scroll gesture starts inside any blocklist-embedded
scrollable container that can scroll in the gesture direction,
clamp the gesture to that container until the gesture ends. A new
scroll gesture that starts while the inner container is already
at its boundary is free to hit the parent.

## Scope (V1)

V1 applies to ALL blocklist-embedded scrollable containers, not
just code diffs. Affected container types include, at minimum:

- code-diff scroll containers embedded inside the blocklist (the
  original GH-9808 case): the inline-action diff view in
  `app/src/ai/blocklist/inline_action/code_diff_view.rs`.
- file-tree / directory-tree panels rendered inside a blocklist
  entry.
- inline data tables rendered inside a blocklist entry.
- inline log / output panels rendered inside a blocklist entry.

**Standalone (non-blocklist) diff viewers are explicitly out of
scope for V1.** The standalone diff viewer in
`app/src/code/diff_viewer.rs` is rendered inside the Code panel,
not as a child of the blocklist viewport, and therefore has no
parent blocklist gesture to clamp against. It follows native
scroll-chaining semantics and is unaffected by V1. (See "Out of
scope" below.) Implementation pointers below list ONLY blocklist-
embedded containers; the earlier standalone-diff entry has been
removed to keep scope consistent.

### Direction coverage (vertical AND horizontal)

The clamp applies to BOTH vertical and horizontal axes per
container, independently. Some blocklist-embedded containers
scroll only vertically (e.g., file trees and most diffs); others
support both axes (e.g., a diff with long unwrapped lines, or a
wide inline table). The behavior contract is per-axis:

- For each axis, "boundary" means at the top/left edge or
  bottom/right edge in that axis's direction.
- If the inner container can scroll in the gesture's primary axis
  direction, it owns the gesture for that axis.
- If the inner container is at its boundary in the primary axis at
  gesture start, the parent owns that axis for the gesture window.

#### Multi-axis (diagonal) gestures

For containers that can scroll on both axes, a diagonal gesture
must be locked to a single primary axis at gesture start so that
ownership is decided exactly once. There is **no mid-gesture
ownership transfer**, even for ambiguous diagonals.

1. **Primary-axis selection (decided at first event).** A
   gesture is classified by the accumulated delta of its first
   event. If `|dy| >= 0.7 * (|dx| + |dy|)` the gesture is
   **vertical-primary**; if `|dx| >= 0.7 * (|dx| + |dy|)` it is
   **horizontal-primary**.
2. **Ambiguous diagonal — axis lock at gesture start (NOT mid-
   gesture).** If neither axis crosses the 70% threshold on the
   first event, the primary axis is selected at that same
   instant by the inner container's boundary state at gesture
   start, in this fixed priority order:
   1. If the inner container is at its boundary on EXACTLY ONE
      axis at gesture start, that axis is the primary axis (so
      the parent owns the gesture for that axis, per B-OWN).
   2. If the inner container is at boundaries on BOTH axes at
      gesture start, the **vertical axis is the primary axis**
      (deterministic tie-break; vertical is the dominant scroll
      axis in the blocklist).
   3. If the inner container is at neither boundary at gesture
      start, the **vertical axis is the primary axis**
      (deterministic default; inner owns the gesture per B-OWN).
   In all three sub-cases the primary axis is chosen at gesture
   start, ownership is resolved exactly once per B-OWN from
   the gesture-start boundary state, and the binding does NOT
   change mid-gesture even if the inner container subsequently
   hits a boundary on either axis. Specifically: once an
   ambiguous diagonal has been classified at its first event,
   the primary-axis owner is FROZEN for the remainder of the
   gesture window. If the inner container reaches the primary-
   axis boundary mid-gesture while it was the primary-axis
   owner, remaining primary-axis delta is discarded (per B1a);
   ownership does NOT transfer to the parent. If the parent
   was the primary-axis owner from gesture start and the inner
   container later becomes able to scroll on the primary axis
   (e.g. the user scrolls the parent away from the inner's
   boundary on some other input), ownership still does NOT
   transfer back to the inner. The only way to re-evaluate
   ownership is a `gap >= 200ms` of input idleness ending the
   current gesture (per B2). Boundary collisions never re-bind
   gesture ownership; only time-based gesture ends do.
3. **Per-axis ownership and effects.** Once the primary axis is
   selected and ownership is resolved per B-OWN below using only
   that axis's gesture-start boundary state, the non-primary
   axis is **always** applied to the inner container (if it can
   scroll in that direction) for the duration of the gesture
   window. The lock controls scroll-chaining ownership of the
   primary axis only; it does not gate the non-primary axis.

## Behavior contract

### Gesture ownership

- B1. The first wheel/trackpad scroll event after a gesture-window
  gap determines the gesture owner. If the inner container can
  scroll in that event's direction (per the primary axis selected
  above), the inner container owns the gesture. If the inner
  container is already at its top/bottom (or left/right) boundary
  for that direction at the moment the gesture starts, the parent
  blocklist owns the gesture.
- B1a. When the inner container owns a gesture, subsequent scroll
  events in the same gesture window are consumed by the inner
  container even after it reaches its top/bottom (or left/right)
  edge. Remaining delta is discarded for the inner container and
  NOT propagated to the parent.
- B2. **Gesture-window timing (single canonical rule).** Two
  consecutive scroll events belong to the same gesture window if
  and only if the time gap between them is **strictly less than
  200ms** (`gap < 200ms`). A gap of **exactly 200ms or more**
  (`gap >= 200ms`) ends the gesture and starts a new one; the
  next event after such a gap is the first event of a new
  gesture and re-evaluates ownership per B-OWN. This matches
  macOS/Windows trackpad inertia conventions. Every other
  reference to the gesture-window threshold in this spec
  (B3, B-OWN, B-OWN-1, B-OWN-2, A2, T2, and the
  Implementation pointers paragraph on the
  `GestureOwnership` struct) uses exactly this `gap < 200ms`
  rule for "same gesture window" and exactly the
  `gap >= 200ms` rule for "ends the gesture / re-evaluates
  ownership". There are no other thresholds (`<= 200ms`,
  `> 200ms`, or any other phrasing) anywhere in this spec.
- B3. When the parent blocklist owns a gesture because the first
  event started while the inner container was already at its
  boundary, every subsequent scroll event whose gap from the
  previous event is `< 200ms` keeps bubbling to the parent.
  Once a `>= 200ms` gap occurs, ownership is re-evaluated from
  the next scroll event.
- B4. Same behavior at top AND bottom edges (vertical) AND at
  left AND right edges (horizontal).

### Gesture ownership at boundaries (binding rule)

- B-OWN. **Boundary state at gesture start is the sole binding
  decision point.** When a new wheel/trackpad gesture starts (the
  first event after a `>= 200ms` gap of input idleness, per B2),
  ownership is resolved exactly once from the inner container's
  boundary state in the gesture's primary-axis direction at that
  instant:
  - inner container can scroll in primary-axis direction → inner
    owns the entire gesture
  - inner container is at its boundary in that direction → parent
    owns the entire gesture
- B-OWN-1. Implementers MUST NOT bubble the first delta to the
  inner container and then later "discover" the boundary and
  re-route remaining deltas to the parent. There is no mid-gesture
  transfer of ownership, including for ambiguous diagonals (see
  Multi-axis (diagonal) gestures above). Once a gesture is bound
  to a container on its first event, it stays bound until a
  `>= 200ms` gap of input idleness ends the gesture.
- B-OWN-2. The gesture window (`gap < 200ms` rule from B2)
  applies only to gestures that have already started. It does
  not "split" a single gesture into inner-then-parent halves.

### Per-axis ownership representation (state model)

The owner is represented as **two independent per-axis owners**
plus a single shared gesture-window timer, NOT as a single enum
value:

```rust
struct GestureOwnership {
    vertical:   AxisOwner,       // Inner | Parent | None
    horizontal: AxisOwner,       // Inner | Parent | None
    last_event: Option<Instant>, // shared B2 gesture-window timer
}

enum AxisOwner { Inner, Parent, None }
```

This per-axis representation is required so that single-axis
gestures (vertical-only or horizontal-only) leave the unused
axis at `None` while the other axis carries `Inner` or `Parent`,
and so that the spec's "primary axis owns scroll-chaining; the
non-primary axis is always applied to the inner container"
contract from Multi-axis (diagonal) gestures is representable
without a special "mixed" state.

**Concrete example of a mixed per-axis ownership state** that
this struct must (and does) represent: during an ambiguous
diagonal gesture where the inner container was at its vertical
boundary at gesture start but not at its horizontal boundary,
per Multi-axis (diagonal) gestures rule 2.1 the vertical axis
is the primary axis and the parent owns it, while the
horizontal axis still scrolls the inner container. The
resulting state is:

```rust
GestureOwnership {
    vertical:   AxisOwner::Parent, // parent owns primary axis
    horizontal: AxisOwner::Inner,  // inner owns non-primary axis
    last_event: Some(t0),
}
```

A single flat enum like
`{ InnerVertical, InnerHorizontal, Parent, None }` is
explicitly rejected: it cannot represent the
`{ vertical: Parent, horizontal: Inner }` mixed state shown
above within a single gesture, which is the exact case
Multi-axis (diagonal) gestures requires. The earlier
implementation hint that listed
`InnerVertical | InnerHorizontal | Parent | None` is replaced
by this `GestureOwnership` struct.

### Scrollbar clicks (Warp custom scrollbar hit-test contract)

Warp uses a custom-rendered scrollbar
(`crates/warpui_core/src/elements/shared_scrollbar.rs` and the
new-scrollable family in
`crates/warpui_core/src/elements/new_scrollable/`). Hit-testing
goes through Warp's own scrollbar geometry, NOT browser ARIA
roles or `::-webkit-scrollbar*` pseudo-element selectors. The
`role="scrollbar"` / pseudo-element references in the previous
draft of this spec were incorrect for Warp's rendering model and
are dropped.

- B5. Click/drag on a scrollbar bypasses the wheel/trackpad
  gesture clamp because it is a pointer action, not a
  wheel/trackpad gesture. Specifically:
  - The parent blocklist's own scrollbar always responds to
    direct click/drag, regardless of any active inner gesture.
  - The inner container's scrollbar — when explicitly clicked or
    dragged via thumb or track — also bypasses the gesture clamp
    because click is a non-momentum input.
  - Wheel/trackpad input that happens to occur over the scrollbar
    area still respects the gesture-window contract above; only
    actual click/drag pointer input bypasses.
- B5a. **"Scrollbar target" defined (Warp scrollbar hit-test).**
  V1 introduces a NEW small hit-test helper on the existing
  scrollbar geometry. The helper is added in this spec because
  the equivalent function does not exist today in
  `crates/warpui_core/src/elements/shared_scrollbar.rs` (verified
  on the spec branch — `shared_scrollbar.rs` exposes
  `ScrollbarGeometry { track_bounds, thumb_bounds, ... }`,
  `compute_scrollbar_geometry(...)`,
  `scroll_delta_for_pointer_movement(...)`, etc., but does NOT
  expose a `ScrollbarTarget` enum or `hit_test()` function).
  The new helper is a pure function over the existing geometry
  fields, not new persistent state:

  ```rust
  // To be added to
  // crates/warpui_core/src/elements/shared_scrollbar.rs
  pub enum ScrollbarTarget {
      Track,   // pointer over track_bounds but NOT over thumb_bounds
      Thumb,   // pointer over thumb_bounds
      None,    // pointer not over scrollbar geometry
  }

  // Pure helper over the existing ScrollbarGeometry struct.
  // No new state; reads track_bounds / thumb_bounds only.
  pub fn hit_test(point: Vector2F, geometry: &ScrollbarGeometry)
      -> ScrollbarTarget {
      // Pseudocode reference:
      //   if geometry.thumb_bounds.contains(point) { Thumb }
      //   else if geometry.track_bounds.contains(point) { Track }
      //   else { None }
  }
  ```

  Pointer events whose hit-test returns `Track` or `Thumb` are
  scrollbar pointer events and bypass the gesture clamp. Pointer
  events that return `None` are routed to the inner container's
  content. Wheel/trackpad events are NEVER classified through
  this hit-test; they always route to the gesture-ownership
  state machine.
- B5b. Neither scrollbar click changes gesture ownership for
  later wheel/trackpad events.

## Acceptance criteria

In each criterion below, "inner container" means any
blocklist-embedded scrollable container in scope (see Scope). At
least two distinct container types must satisfy each criterion
in tests.

- A1. Scroll up in a long inner container; reach top; same
  gesture continues — blocklist does NOT scroll. Scrollbar at the
  inner container's top remains visible.
- A2. Stop scrolling for `>= 200ms` (per B2 canonical rule),
  scroll up again — blocklist scrolls normally.
- A3. Click or drag the inner container's scrollbar (Warp
  scrollbar `hit_test()` returns `Track` or `Thumb` per B5a) —
  the inner container scrolls as before, with no parent gesture
  clamp involved.
- A3-parent. Click or drag the parent blocklist scrollbar (same
  `hit_test()` definition) — the parent scrolls immediately even
  if an inner-owned wheel gesture is in flight.
- A4. Same as A1 but at the bottom edge.
- A4h. Horizontal-axis mirror of A1: scroll left in a wide inner
  container; reach left edge; same gesture continues — blocklist
  does NOT scroll horizontally.
- A4h-right. Horizontal-axis mirror of A4: scroll right; reach
  right edge; same gesture continues — blocklist does NOT scroll
  horizontally.
- A5. Start a new scroll gesture while the inner container is
  already at its top boundary — the parent blocklist scrolls for
  that whole gesture window. The first delta is NOT bubbled to
  the inner container.
- A6. Coverage: A1, A2, A4, A5 each pass for at least two
  distinct container types from Scope.
- A7. Diagonal-gesture axis lock: a diagonal trackpad gesture
  whose first event has `|dy| >= 0.7 * (|dx|+|dy|)` is treated
  as vertical-primary; ownership is resolved by the vertical
  boundary only. The horizontal component scrolls the inner
  container if it can. After the gesture ends, the next gesture
  re-classifies independently.
- A8. Wheel-over-scrollbar non-bypass: a wheel event whose cursor
  is positioned over the scrollbar track is processed by the
  gesture-ownership state machine (NOT bypassed), per B5a.

## Implementation pointers

A "blocklist-embedded scrollable container" is any descendant
scroll container of the blocklist whose root element matches one
of the following identifiers in the codebase. This list must be
extended as new containers are added.

| Container type    | File path                                                     | Component identifier                                         |
| ----------------- | ------------------------------------------------------------- | ------------------------------------------------------------ |
| Code diff (inline action) | `app/src/ai/blocklist/inline_action/code_diff_view.rs` | The diff view's outer scrollable wrapper (constructed via the shared scrollable element from `warpui_core`) |
| File tree (blocklist-embedded) | `app/src/code/file_tree/view.rs`, `view/render.rs`   | File-tree view's scrollable wrapper, when rendered inside a blocklist entry |
| Inline tables (markdown) | `crates/ai/src/gfm_table.rs`                            | GFM table rendered into `crates/editor/src/render/element/table.rs`; the table cell's scrollable wrapper when content overflows |
| Log / output panels | `app/src/terminal/block_list_viewport.rs`                   | Block-list viewport's embedded output regions                |

The standalone diff viewer (`app/src/code/diff_viewer.rs`) is
**not** in this table because it is rendered in the Code panel,
not inside the blocklist viewport, and is out of scope for V1
(see Scope and Out of scope).

The shared clamp lives in the `Scrollable` /
`new_scrollable` element family in
`crates/warpui_core/src/elements/`. The clamp behavior is added
as an opt-in flag on the scrollable wrapper (e.g.,
`clamp_to_parent_blocklist: bool`) so each container opts in by
constructing the wrapper with the flag set, rather than
duplicating gesture-window state across containers.

The 200ms gesture-window state is implementable as the
`GestureOwnership` struct above (per-axis owners +
`last_event: Option<Instant>` shared timer) on the wrapper's
view state. A new event whose gap from `last_event` is
`>= 200ms` (per B2) is treated as the first event of a new
gesture and re-evaluates per-axis ownership from boundary state
per B-OWN. A flat single-enum owner is explicitly rejected
because it cannot represent the per-axis cases required by
Multi-axis (diagonal) gestures (see "Per-axis ownership
representation" above).

Scrollbar pointer hit-testing resolves via the new
`ScrollbarTarget` enum + `hit_test()` helper function defined in
this spec at B5a. This helper is being **added** by V1 to
`crates/warpui_core/src/elements/shared_scrollbar.rs`; it does
not exist on `master` today. Implementers must add it as part of
V1; they must NOT route hit-testing through ARIA roles
(`role="scrollbar"`) or browser pseudo-element selectors
(`::-webkit-scrollbar*`), neither of which apply to Warp's
custom-rendered scrollbar. The helper is a pure function over
the existing `ScrollbarGeometry` struct (which DOES exist on
master); it adds no new persistent state.

## Test plan

Each test below runs against at least two container types from
Scope.

- T1. Synthetic scroll-event stream test: feed deltas with
  100ms spacing reaching the top, assert no propagation to
  parent.
- T2. Same stream with a 300ms gap mid-stream; assert deltas
  after the gap propagate.
- T3. Bottom-edge mirror of T1/T2.
- T4. A new stream whose first delta starts while the inner
  container is already at the boundary propagates ALL deltas in
  that stream to the parent — including the very first delta
  (no first-delta-to-inner bubble).
- T5. Discrete inner-container scrollbar-click jump (target
  matches B5a Warp `hit_test()` returning `Track` or `Thumb`)
  remains local to the inner container and does not affect later
  wheel/trackpad ownership.
- T6. Parent-scrollbar click during an active inner-owned wheel
  gesture immediately scrolls the parent without disturbing
  remaining inner-bound wheel events.
- T7. Multi-container coverage: T1, T2, T3, T4 are repeated
  across each container type listed in Implementation Pointers
  to verify the shared wrapper applies uniformly. Specifically:
  - T_diff_clamp: blocklist-embedded code-diff container ONLY
    (`app/src/ai/blocklist/inline_action/code_diff_view.rs`).
    The standalone diff viewer (`app/src/code/diff_viewer.rs`)
    is **explicitly excluded** from T_diff_clamp because it is
    out of scope for V1 (see Scope and Out of scope).
  - T_filetree_clamp: file-tree container
  - T_table_clamp: GFM-table container with horizontal overflow
  - T_log_clamp: block-list viewport output region
- T_horizontal_axis_clamp. Horizontal-axis mirror of T1: deltas
  at 100ms spacing along the X axis, reaching the left edge;
  assert no propagation to parent. Repeat at right edge.
- T_diagonal_axis_lock. Diagonal trackpad gesture: feed first
  event with `(dx, dy) = (10, 25)` (vertical-primary because
  `25/35 >= 0.7`); assert vertical boundary determines ownership
  and horizontal component still scrolls the inner container.
  Then feed `(dx, dy) = (25, 10)` and assert horizontal-primary
  classification. Then feed `(dx, dy) = (15, 15)` (ambiguous,
  neither axis crosses 70%); assert primary-axis selection at
  gesture start per Multi-axis (diagonal) gestures rule 2:
  - inner at vertical boundary only at gesture start → primary =
    vertical, parent owns vertical for the gesture, horizontal
    still scrolls inner.
  - inner at horizontal boundary only at gesture start →
    primary = horizontal, parent owns horizontal, vertical
    still scrolls inner.
  - inner at both boundaries → primary = vertical (deterministic
    tie-break), parent owns vertical, horizontal still scrolls
    inner if it can.
  - inner at neither boundary → primary = vertical (default),
    inner owns vertical, horizontal also scrolls inner.
  In every sub-case, hitting a boundary mid-gesture does NOT
  transfer ownership.
- T_wheel_over_scrollbar. Wheel events whose cursor is over the
  scrollbar track go through the gesture state machine and are
  NOT classified as scrollbar clicks (per A8 / B5a).

## Out of scope

- Animated "rubber-band" overshoot indicator. The fix is silent
  absorption.
- Configurable threshold (200ms is a hard-coded constant; can be
  tuned per platform if profiling shows misfires).
- Non-blocklist-embedded scroll containers (e.g., top-level app
  panels, modal scroll regions, the standalone diff viewer in
  `app/src/code/diff_viewer.rs`) — those follow native browser
  / OS scroll-chaining semantics and are unaffected by V1.
