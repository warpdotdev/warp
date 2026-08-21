# Shift+click extends block-list text selection

## Context
The product behavior is defined in [PRODUCT.md](PRODUCT.md). The design is based on `master` commit [`36dd2cc`](https://github.com/warpdotdev/warp/commit/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1).

- [`app/src/terminal/block_list_element.rs:812`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/terminal/block_list_element.rs#L812) aliases `BlockTextSelectAction` to the shared terminal `SelectAction<BlockListPoint>`.
- [`app/src/terminal/block_list_element.rs:1508-1675`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/terminal/block_list_element.rs#L1508-L1675) always dispatches `Begin` for command and rich-content rows. It does not inspect Shift for text extension.
- [`app/src/terminal/block_list_element.rs:4609-4641`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/terminal/block_list_element.rs#L4609-L4641) routes mouse events and currently treats every Shift+drag as whole-block selection.
- [`app/src/terminal/model/selection.rs:240-254`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/terminal/model/selection.rs#L240-L254) defines `Begin`, `Update`, and `End`, but no explicit `Extend`.
- [`app/src/terminal/model/blocks/selection.rs:394-444`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/terminal/model/blocks/selection.rs#L394-L444) resets both endpoints in `start_selection` and already moves only the tail in `update_selection`.
- [`app/src/terminal/model/blocks/selection.rs:826-875`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/terminal/model/blocks/selection.rs#L826-L875) normalizes a completed semantic or line selection to visible simple endpoints. Keyboard extension already uses this path.
- [`app/src/terminal/view.rs:18780-18932`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/terminal/view.rs#L18780-L18932) starts and updates point-based block-list selection. Starting a terminal selection primes agent rich-content blocks at their minimum or maximum point so drag can cross them.
- [`app/src/terminal/model/blocks/selection.rs:873-897`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/terminal/model/blocks/selection.rs#L873-L897) tracks rich-content-only selection separately when no point-based block-list selection owns the range.
- [`crates/warpui_core/src/elements/gui/selectable_area.rs:283-378`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/crates/warpui_core/src/elements/gui/selectable_area.rs#L283-L378) clears every prior rich-content selection on mouse-down.
- [`crates/warpui_core/src/elements/gui/selectable_area.rs:397-503`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/crates/warpui_core/src/elements/gui/selectable_area.rs#L397-L503) already updates only the rich-content tail and supports reversing around its head.
- [`app/src/editor/view/element.rs:281-341`](https://github.com/warpdotdev/warp/blob/36dd2cc2ecc0fbb3e9221b2aebb2e500f2405df1/app/src/editor/view/element.rs#L281-L341) is the in-repo interaction pattern: Shift dispatches `Extend`, while a plain click dispatches `Begin`.

The open earlier spec PR [#10026](https://github.com/warpdotdev/warp/pull/10026) covers terminal text only and predates the current rich-content selection coordination. CORE-762 supersedes it by defining the block-selection fallback, the current multi-block behavior, rich-content scope, and the fixed-endpoint rule against the current model.

The external implementation PR [#11933](https://github.com/warpdotdev/warp/pull/11933) is useful evidence that an explicit extend action and an “extending” drag state fit the event path. Do not adopt its nearest-boundary behavior, alt-screen scope, or lack of rich-content support.

## Proposed changes
### 1. Add an explicit terminal extend action
- Add `SelectAction::Extend` with the clicked point, side, and mouse position.
- Keep `Begin` for new selection, `Update` for an active drag, and `End` for mouse-up.
- Update all exhaustive matches. Alt-screen handlers must ignore `Extend`; v1 must not enable alt-screen Shift+click extension.

An explicit variant is preferred over dispatching `Update` directly because mouse-down must also:
- Check applicability.
- Normalize a completed selection before moving its active endpoint.
- Enter active text-drag state.
- Preserve the fixed endpoint across later Shift+clicks.
- Suppress whole-block Shift+drag routing for this gesture.

### 2. Detect applicability before changing block state
In `BlockListElement::mouse_down`, determine whether Shift+click can extend the current text selection before dispatching `BlockSelectAction`.

For a point-based block-list selection, extension applies when:
- The current selection expands to non-empty selected text.
- The click maps to a block-list text position supported by the existing drag path.
- A child interactive element has not consumed the event.

For a rich-content-only selection, the owning `SelectableArea` decides applicability. The block list must not treat the presence of a tracked rich-content view ID as a point-based anchor. An externally primed rich-content selection bound is different: it identifies a `SelectableArea` that participates in an existing point-based block-list selection.

When point-based extension applies:
- Dispatch `Extend`, not `Begin`.
- Do not initiate or range-extend whole-block selection.
- Record that the current mouse gesture extends text, so `LeftMouseDragged` sends `Update` even while Shift remains held.

When extension does not apply, leave the current event path unchanged. This preserves whole-block range selection and normal clicks.

### 3. Extend the block-list tail
Handle `Extend` in `TerminalView` next to `begin_block_text_selection`.

- Call `standardize_text_selection` before the first tail move. This converts completed semantic and line selections to their visible simple endpoints.
- Call the existing `BlockList::update_selection(point, side)` primitive. Do not replace the selection or move its fixed endpoint.
- Set `is_selecting` and the gesture state needed for drag and mouse-up.
- Clear whole-block selections only after a non-empty text extension is established, matching existing text-selection mutual exclusion.
- Reuse `end_text_selection` for copy-on-select, empty-selection cleanup, agent-context invalidation, and final notification.

Do not add “move nearest boundary” logic. Repeated Shift+click must always move the same active endpoint.

### 4. Extend rich content through `SelectableArea`
Add a Shift-aware branch to `SelectableArea::on_mouse_down`.

When the click is inside the area, extend instead of clearing if either condition holds:
- The area owns a non-empty selection whose fixed endpoint is relative to that area.
- The area has an external minimum or maximum bound installed by the block-list selection path.

For either condition:
- Do not call `clear()`.
- Normalize the completed selection to simple character endpoints.
- Keep the fixed endpoint.
- Move the tail using the existing `update_selection` logic.
- Set `is_selecting` so a following drag and mouse-up use the existing callbacks.

Otherwise, use the current clear-and-begin path.

For point-based terminal selections that cross agent rich content, preserve the current coordination through `start_selection_at_min_point` and `start_selection_at_max_point`. When the fixed bound is external, the rich-content handler must allow the mouse-down to continue to the block list, or explicitly dispatch the corresponding block-list `Extend`. Both the local rich-content tail and the point-based block-list tail must update in the same gesture so rendering and copied text agree at command/rich-content boundaries.

Do not introduce a shared coordinate type between `BlockListPoint` and `SelectionBound`. A rich-content-only selection is relative to one `SelectableArea`; translating it into a terminal anchor would require new global ownership and layout state. Both implementations instead share the product invariant: keep one fixed endpoint and move one active endpoint.

### 5. Keep selection precedence explicit
- Applicable text extension takes precedence over `SelectedBlocks::range_select`.
- Whole-block Shift+click remains unchanged when no applicable text selection exists.
- Plain click remains `Begin`.
- Interactive children keep event precedence. A Shift+click consumed by a button or nested editor must not also extend the parent selection. Links and selectable secret text remain eligible when no child consumes the event.

## Decisions and trade-offs
- **Explicit `Extend` versus reusing `Update`:** Use `Extend`. The action carries different mouse-down lifecycle and precedence semantics even though both eventually move the tail.
- **Fixed active endpoint versus nearest boundary:** Keep the original fixed endpoint. Nearest-boundary extension makes repeated Shift+click unstable and conflicts with the requester-approved editor-like rule.
- **Two implementations versus one abstraction:** Use the existing block-list and `SelectableArea` primitives. A common abstraction would need a global coordinate and ownership model for little v1 benefit.
- **Normalize semantic selections:** Convert completed word and line selections to visible simple endpoints before extension. This gives simple-cell Shift+click behavior without losing the visible anchor.
- **No feature flag:** The change adds a standard selection interaction and retains a precise fallback. A runtime flag would duplicate event branches and is not required by the request.

## Testing and validation
### Unit tests
- In `app/src/terminal/model/blocks/selection_tests.rs`, verify:
  - A forward simple selection extends forward and keeps its fixed endpoint. Covers PRODUCT 1-2.
  - A later extension reverses past the fixed endpoint without changing it. Covers PRODUCT 3.
  - A selection spans multiple command blocks and includes intermediate block text. Covers PRODUCT 4.
  - Semantic and line selections normalize to visible simple endpoints before extension. Covers PRODUCT 11.
- In `crates/warpui_core/src/elements/gui/selectable_area.rs` tests, verify:
  - Shift+click in the same area keeps the head and moves the tail.
  - Repeated extension can reverse.
  - An empty selection uses clear-and-begin behavior.
  - A click outside the owning area does not extend it. Covers PRODUCT 1-7.

### View and integration tests
- Add or extend a block-list integration test that:
  - Drags a non-empty terminal selection, releases, Shift+clicks in the same block, and verifies the copied text.
  - Repeats Shift+click across another command block and after scrolling.
  - Reverses past the fixed endpoint.
  - Shift+drags after the extending mouse-down and verifies text, not whole blocks, is selected.
  - Plain-clicks afterward and verifies the old selection is replaced.
- Add an agent rich-content integration case that:
  - Extends within one rich-content area.
  - Extends a terminal-anchored selection into or through agent rich content where drag supports the same range.
  - Verifies the rendered highlight and copied text contain the same content.
- Add a regression case with selected whole blocks and no text selection. Verify Shift+click still calls range selection.
- Run the focused Rust tests for the changed test targets.
- Run `cargo fmt --check`.
- Run the repository-prescribed presubmit checks for the changed Rust crates.
- Capture a computer-use video in the running Warp desktop client showing terminal same-block extension, cross-command extension through rich content, rich-content-only extension, reversal, plain-click reset, and whole-block fallback. Attach the video to the implementation PR.

## Risks and mitigations
- **Text and block selection both react to one gesture:** Decide applicability before dispatching either action and keep an explicit extending-gesture state.
- **Highlight and copied rich text diverge:** Test cross-boundary rendering and copied text together; retain the existing rich-content priming path.
- **Word or line anchor changes unexpectedly:** Normalize once before moving the tail and test both forward and reversed selections.
- **Nested rich-content controls receive duplicate events:** Preserve child-first event dispatch and extend only when the owning `SelectableArea` handles the click.

## Parallelization
Parallel implementation is not recommended. The terminal action, block-selection precedence, drag gesture state, and rich-content coordination share one mouse-event lifecycle and should land in one branch and one PR. Unit-test additions can be delegated only after the event contract is implemented, but the expected time savings do not justify merge risk for this medium, tightly coupled change.
