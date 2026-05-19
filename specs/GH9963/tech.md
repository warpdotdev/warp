# Tech Spec: Shift+Click extends terminal text selection

**Issue:** [warpdotdev/warp#9963](https://github.com/warpdotdev/warp/issues/9963)

## Context

Terminal output selection state lives in [`app/src/terminal/model/blocks/selection.rs`](https://github.com/warpdotdev/warp/blob/master/app/src/terminal/model/blocks/selection.rs), which already models selections as a pair of `BlockAnchor`s — `head` and `tail` — with helper methods like `start_anchor()` and `end_anchor()` that resolve them regardless of which way the selection is oriented. The orientation flips automatically when `head` ends up before `tail` in document order.

The mouse-down handler that initiates a selection lives in the terminal view (find via `grep -rn "fn.*mouse_down\|on_mouse_down" app/src/terminal/view/`). Today it always *replaces* the selection on click. The fix is a small branch in that handler: if `Shift` is held and a selection exists, mutate the existing selection's `head` to the click point rather than constructing a fresh one.

### Relevant code

| Path | Role |
|---|---|
| `app/src/terminal/model/blocks/selection.rs` | The `Selection` struct with `head`/`tail` anchors. Already exposes mutation helpers; the new behavior consumes them. |
| `app/src/terminal/view/init.rs` (or wherever the terminal view's mouse handlers are wired — verify at implementation time) | The mouse-down dispatch site. New branch checks for `Shift` modifier + existing selection. |
| `app/src/terminal/event.rs` | Mouse event types. The `Shift` modifier is already plumbed through; no new event surface. |
| `crates/warpui/.../keystroke.rs` | The Modifiers struct. `shift: bool` is the existing field used by other Shift-aware handlers (e.g. Shift+Arrow in the input). |
| `app/src/terminal/model/blocks/selection_tests.rs` (or equivalent — locate at implementation time) | Existing selection tests. New unit tests mirror their structure. |

### Related closed PRs and issues

- The existing `Selection::start_anchor()` / `end_anchor()` helpers were added precisely to support orientation-agnostic selection mutations. This spec uses them to handle the backward-extension case (invariant 2) without explicit branching on direction.
- No closed PRs interact with this surface that are relevant context.

## Crate boundaries

All new code lives in `app/`. No new crate, no cross-crate boundary changes. The `Selection` struct is `pub` within the terminal model module already.

## Proposed changes

### 1. Mouse-down handler: Shift-extension branch

**File:** the existing terminal view mouse-down dispatch (locate via `grep -rn "fn handle_mouse_down\|MouseEvent::Down" app/src/terminal/view/`).

Pseudocode for the new branch (added before the existing "construct fresh selection" path):

```rust
fn handle_terminal_mouse_down(
    &mut self,
    event: &MouseDownEvent,
    ctx: &mut ViewContext<Self>,
) {
    let click_point = self.point_at(event.position);

    // New branch: Shift held + existing selection → extend.
    if event.modifiers.shift {
        if let Some(existing) = self.current_selection() {
            // Preserve the original anchor (whichever side was the user's
            // original press point). `Selection::set_head_to` is the
            // existing mutation that updates the head anchor and lets the
            // selection orient itself; if it doesn't exist, add a small
            // method that wraps `selection.head = BlockAnchor::new(...)`
            // and notifies subscribers.
            self.update_selection(|sel| {
                sel.set_head_to(click_point);
            });
            return;
        }
        // Shift held but no existing selection: fall through to fresh-selection
        // construction (degenerate case is invariant 3).
    }

    // Existing path: construct a fresh selection at click_point.
    self.start_new_selection(click_point);
}
```

The "original anchor" is the `tail` of the existing selection in today's model — the field that wasn't moved by the most recent mouse drag. If the existing selection's `head` is currently before `tail` in document order (i.e. the user dragged backward last time), `set_head_to(click_point)` may flip the orientation; the existing `Selection`'s `start_anchor()` / `end_anchor()` getters resolve the user-visible "from" and "to" points correctly regardless.

### 2. Selection mutator (if missing)

**File:** `app/src/terminal/model/blocks/selection.rs`.

If a public `set_head_to(point)` (or equivalent) doesn't already exist on `Selection`, add one:

```rust
impl Selection {
    /// Move the head anchor to the given point, preserving tail.
    /// Used by Shift+Click selection extension. The selection's
    /// `start_anchor()` / `end_anchor()` getters automatically resolve
    /// the user-visible direction after this mutation.
    pub fn set_head_to(&mut self, point: GridPoint) {
        self.head = BlockAnchor::new(point, self.head.side);
    }
}
```

The `side` field on `BlockAnchor` (left vs right of the column) is preserved; for click-to-extend, keeping the existing head side is the right default — Shift+Click at a column doesn't change the half-column the selection ends on.

### 3. Anchor persistence across scroll

The `BlockAnchor` already encodes a buffer position (block index + grid point), not a viewport position. Scrolling the buffer between the original click-drag and the Shift+Click does NOT invalidate the anchor — invariant 9 in product.md (anchor persistence across scroll) holds for free.

### 4. Visual feedback

**No changes.** The existing selection-render path observes the model and re-renders on mutation. Updating the selection via `set_head_to` triggers the same observers as a drag-end commit, so the visual state is byte-equivalent to "what would have happened if the user had dragged from the original anchor to the click point in one continuous motion".

### 5. Other surfaces (out of scope)

The editor, Markdown viewer, and settings selection state machines are unrelated to `app/src/terminal/model/blocks/selection.rs`. Their handlers are different sites; this spec does not touch them. Invariant 6 ensures we don't accidentally regress them.

## Testing and validation

Each invariant from `product.md` maps to a test at this layer:

| Invariant | Test layer | File |
|---|---|---|
| 1 (forward extension) | unit | `app/src/terminal/model/blocks/selection_tests.rs` extension — construct a selection at `(5,10)→(8,30)`, call `set_head_to(12,5)`, assert resulting `start_anchor()` == `(5,10)` and `end_anchor()` == `(12,5)`. |
| 2 (backward extension flips orientation) | unit | selection_tests — construct a selection at `(8,20)→(12,5)`, call `set_head_to(5,10)`, assert resulting selection covers `(5,10)→(8,20)` (start_anchor / end_anchor reflect the new bounds). |
| 3 (no prior selection: degenerate point) | unit | view-level test — invoke the mouse-down handler with `modifiers.shift = true` and no existing selection, assert a new one-point selection is constructed. |
| 4 (non-Shift click resets anchor) | unit | view-level — click-drag → release; non-Shift click elsewhere; Shift+Click yet elsewhere; assert the second Shift+Click extends from the *non-Shift click's anchor*, not the original drag's anchor. |
| 5 (smart-selection sets anchor for subsequent extension) | unit | view-level — double-click on a word; Shift+Click later in the buffer; assert the resulting selection runs from the word's start to the click point. |
| 6 (no regression in editor / Markdown / settings) | integration | existing selection tests for those surfaces — ensure they still pass. The terminal change shouldn't compile-touch them, but a smoke run is cheap. |
| 7 (rectangular-selection mode unaffected) | unit | view-level — enter rectangular mode, attempt Shift+Click, assert behavior matches today's rectangular mode (no extension via this path). |
| 8 (downstream consumers see updated selection) | integration | after a Shift+Click extension, "Copy on Select" (if enabled) copies the new selection bounds; find-in-block highlighting reflects the new selection. The model emits the same observer event as a drag-end, so this is mostly a smoke test. |

### Cross-platform constraints

- Mouse modifier handling already cross-platform via the `Modifiers` struct.
- `BlockAnchor` is platform-agnostic.
- No platform-specific cursor or visual tweaks in V1.

## End-to-end flow

```
User has an active selection from (5, 10) to (8, 30)
  └─> Selection { head: (8,30), tail: (5,10) } in the model

User holds Shift and clicks at (12, 5)
  └─> Terminal view receives MouseDown { position, modifiers: { shift: true } }
        └─> handle_terminal_mouse_down()
              ├─> compute click_point = self.point_at(position) → (12, 5)
              ├─> modifiers.shift && current_selection().is_some() → branch
              │     └─> selection.set_head_to((12, 5))
              │           └─> head = BlockAnchor::new((12,5), self.head.side)
              │           └─> emit ModelChanged event
              └─> render observer fires → highlight redraws with new bounds

User holds Shift and clicks at (3, 2) (now backward of the original anchor)
  └─> handle_terminal_mouse_down → set_head_to((3, 2))
        └─> head is now before tail; start_anchor() returns (3, 2),
            end_anchor() returns (5, 10) (now the "tail" appears as end)
        └─> render reflects (3, 2) → (5, 10) selection
```

## Risks

- **`set_head_to` may not exist as a public mutation.** If the existing `Selection` API only exposes `start_anchor()` / `end_anchor()` getters and constructs new selections rather than mutating, the implementation grows by a small `pub fn`. Already noted in Section 2.
- **Interaction with shared-session selection broadcasting.** If selections are broadcast to other viewers (shared sessions), the broadcast event needs to fire on Shift+Click extension just as it does on drag. **Mitigation:** the broadcast subscribes to the same `ModelChanged` event the render observer uses, so this rides for free as long as `set_head_to` triggers the same notification. Verify at implementation time.
- **Touchpad sensitivity producing accidental Shift+Clicks.** A user who's holding Shift for a keyboard shortcut and incidentally clicks would now extend a selection where today they'd have started a new one. **Mitigation:** matching mainstream terminal behavior is the bigger consideration; this potential surprise is well-understood by users coming from any other terminal. Document the change clearly in release notes.
- **Modifier-key plumbing on non-mac platforms.** Linux/Windows mouse events plumb `Shift` through the same `Modifiers` struct; verify there's no platform-specific gap in the event path. The existing Shift+Arrow input handler is the proof-of-existence — if Shift modifiers reach that code, they reach the mouse handler too.

## Follow-ups (out of this spec)

- Shift+Drag to extend an existing selection while dragging (V1 only handles the discrete click case; a drag started with Shift held continues today's drag behavior unchanged).
- Shift+Click parity in the editor / Markdown viewer / settings selection surfaces.
- Shift+Click semantics specific to rectangular-selection mode.
- Keyboard-only selection extension in terminal output (Shift+Arrow).
- A "Copy on Select" follow-up that explicitly tests interaction with extension.
