# Code review find navigation scrolls selected branch-diff matches into view — Tech Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/10396
Product spec: `specs/GH10396/product.md`

## Problem
Code review Find already computes matches across expanded diff editors and selects the next or previous match. The broken path is the scroll from the selected match to the list viewport when the selected editor has not produced character bounds yet. This is most visible in long branch comparisons, where a target editor can be off-screen and lazily laid out.

`CodeReviewView::scroll_to_position` has a pending precise-scroll path for this case, but it can lose the pending target if the target editor emits a viewport/layout event before `character_vertical_bounds` is available. After that, the user must invoke Find navigation again; by then the editor has often rendered enough for the second attempt to succeed. The implementation should make pending Find scrolls durable across lazy-layout events and clear them only after the precise scroll actually succeeds or the target becomes stale.

## Relevant code
- `app/src/code_review/find_model.rs:145` — `CodeReviewFindModel::focus_next_find_match` advances selection and emits `FindEvent::UpdatedFocusedMatch`.
- `app/src/code_review/find_model.rs:180` — `selected_match_info` exposes the selected editor id and character offsets used by the view to scroll.
- `app/src/code_review/find_model.rs:245` — `run_search` searches each expanded editor and preserves result ordering across editor handles.
- `app/src/code_review/code_review_view.rs:1595` — `handle_find_event` receives Find UI actions and calls `focus_next_find_match`.
- `app/src/code_review/code_review_view.rs:1630` — `handle_find_model_event` updates highlights and calls `scroll_to_selected_match`.
- `app/src/code_review/code_review_view.rs:1935` — `scroll_to_selected_match` maps the selected editor id back to a file index and calls `scroll_to_position`.
- `app/src/code_review/code_review_view.rs:1991` — `scroll_to_position` either scrolls immediately using available bounds or records `pending_precise_scroll` after a conservative scroll to trigger lazy layout.
- `app/src/code_review/code_review_view.rs:2075` — `get_match_character_bounds` reads character vertical bounds from the editor render state; this returns `None` before lazy layout has measured the target offsets.
- `app/src/code_review/code_review_view.rs:2132` — `vertically_scroll_to_match` applies final list scrolling once bounds are known.
- `app/src/code_review/code_review_view.rs:2176` — `horizontally_scroll_to_match` aligns long-line matches after vertical scroll.
- `app/src/code_review/code_review_view.rs:991` — `create_list_state` wires the viewported list and scroll-preservation adjustment hook.
- `app/src/code_review/scroll_preservation.rs` — list scroll-preservation context can adjust offsets as item heights change.
- `app/src/code_review/code_review_view.rs:2493` — `build_view_state_for_file_diffs` creates editor state for each diff file and adds list items.
- `app/src/code_review/code_review_view.rs:2909` — desktop code review editors use global buffers and lazy editor layout for file-backed entries.
- `app/src/code_review/code_review_view.rs:3136` — `handle_local_code_editor_events` observes `ViewportUpdated`, `LayoutInvalidated`, and delayed-loading events.
- `app/src/code_review/code_review_view.rs:6906` — `editor_handles` defines the current expanded-editor search scope.

## Current state
The Find flow is split between the model and view:

1. `CodeReviewFindModel::run_search` gathers matches from the editor models and stores `SearchMatch` entries by editor id and offsets.
2. `focus_next_find_match` selects the next global result and emits `FindEvent::UpdatedFocusedMatch`.
3. `CodeReviewView::handle_find_model_event` updates editor highlights and calls `scroll_to_selected_match`.
4. `scroll_to_selected_match` finds the file index for the selected editor and calls `scroll_to_position`.
5. `scroll_to_position` calls `get_match_character_bounds`. If bounds are present, it scrolls vertically and horizontally immediately. If bounds are missing, it scrolls conservatively to the target file, stores `pending_precise_scroll`, and subscribes to the target editor's `ViewportUpdated` event.

The fragile part is the pending path. The subscription takes `pending_precise_scroll` before attempting the precise scroll. If the event belongs to the target editor but bounds are still unavailable, the pending target is dropped. That matches the issue comment's observed behavior: the first navigation to the end of a long diffset fails, while a later attempt succeeds after layout has warmed up.

Scroll preservation is adjacent but not the primary cause. It can change offsets when list item heights are invalidated, so the final implementation must ensure a user-initiated Find scroll remains the active target while lazy editor/list layout settles.

## Proposed changes
### 1. Centralize pending precise-scroll application
Extract the duplicated pending-scroll logic into a helper on `CodeReviewView`, for example:

- `try_apply_pending_precise_scroll(ctx) -> PendingPreciseScrollResult`
- or `apply_pending_precise_scroll_if_ready(ctx) -> bool`

The helper should:

- read the current `pending_precise_scroll`
- validate that the view is still loaded and that the pending file index still exists
- call `get_match_character_bounds`
- when bounds are available, call `vertically_scroll_to_match` and `horizontally_scroll_to_match`, then clear the pending scroll
- when bounds are not available, leave the pending scroll stored and return "not ready"
- when the pending target is stale, clear it and return "stale"

The important invariant is that pending state is cleared only after success or confirmed staleness. A target editor event with missing bounds must not drop the pending scroll.

### 2. Make `scroll_to_position` create durable pending scrolls
Keep the existing immediate path unchanged when bounds are available.

For the missing-bounds path:

- set `pending_precise_scroll` before or immediately after the conservative `scroll_to_with_offset`
- keep the conservative down/up scroll behavior because it is the mechanism that brings the target editor into the viewported list
- subscribe to the target editor only once per pending target when practical, or make repeated subscriptions harmless by routing all events through the centralized helper
- do not overwrite a newer pending target with an older event callback; callbacks should always consult the current `pending_precise_scroll`

If a new Find navigation happens while an older pending scroll exists, replacing the pending target with the new selected match is correct. The current selection is the source of truth.

### 3. Retry after target editor layout events without losing state
Update the `LocalCodeEditorEvent::ViewportUpdated` handling used by the pending-scroll subscription so that:

- events from non-target editors are ignored without mutating the pending target
- events from the target editor call the centralized helper
- if the helper reports "not ready", the pending target remains stored
- if the event fires before the render state has character bounds, the next relevant layout/viewport event can retry the same target

Also call the helper from the existing `LocalCodeEditorEvent::LayoutInvalidated` path in `handle_local_code_editor_events` after invalidating the list height for the file. This gives the pending scroll another chance to complete as item heights settle.

If WarpUI has a standard next-frame or post-layout deferral helper available at implementation time, use it to schedule one retry after a target `ViewportUpdated` that is still not ready. This avoids depending on another user input if the first viewport event is emitted slightly before character bounds become queryable. Keep this retry bounded by the current pending target so it cannot loop forever.

### 4. Protect against stale pending scrolls
Clear or invalidate `pending_precise_scroll` when the underlying result set can no longer correspond to the stored offsets:

- Find query changes or results are cleared
- case-sensitive or regex mode changes trigger a new search
- diff mode changes and `invalidate_all` rebuilds file states/list state
- the Find bar closes
- a target file is collapsed or removed from the searchable expanded-editor set

Some of these paths already replace the loaded state or rerun search. The implementation should make this explicit enough that a pending scroll from an old result cannot jump after the user has moved to a different query or diff mode.

### 5. Keep scroll preservation compatible with Find navigation
Find navigation is a user-directed scroll. Once a pending Find scroll succeeds, the list's scroll context should represent the new visible match area rather than a pre-navigation anchor.

Implementation options:

- after applying the precise scroll, compute and store a scroll context for the target editor using the existing `compute_scroll_context_for_index`
- or clear the old scroll context before the conservative/precise Find scroll so subsequent item height adjustments do not restore the previous position

Prefer the smaller change that prevents scroll-preservation from undoing the final Find scroll. Do not remove the broader scroll-preservation feature.

### 6. Preserve existing Find model behavior
Avoid changing `CodeReviewFindModel` result ordering or search scope unless implementation discovers a direct bug there. The model already selects matches by global result index and exposes offsets through `selected_match_info`. The view-side scroll path should be enough for the reported symptom.

Only add model state if the view needs a stable selection generation or token to reject stale pending scrolls. If added, keep it internal to the Find/code-review flow and update tests accordingly.

## End-to-end flow
1. User searches in the code review panel and presses Enter.
2. `CodeReviewFindModel::focus_next_find_match` selects the next result and emits `FindEvent::UpdatedFocusedMatch`.
3. `CodeReviewView::scroll_to_selected_match` resolves the selected editor id to a file index.
4. If the target match already has render bounds, the view scrolls immediately and updates horizontal scroll.
5. If render bounds are missing, the view stores a pending precise scroll, conservatively scrolls to the target file, and waits for target editor/list layout events.
6. Each target layout/viewport event retries the current pending precise scroll without dropping it if bounds are still missing.
7. Once bounds exist, the view scrolls to the selected match, updates horizontal scroll, updates or clears scroll-preservation context as needed, and clears the pending target.
8. If the query, selection, diff mode, or file state changes before success, stale pending work is cleared or replaced by the new current target.

## Risks and mitigations
- Risk: retrying while bounds remain unavailable can create a scroll or render loop. Mitigation: retry only in response to target editor/list layout events or a bounded next-frame deferral tied to the current pending target.
- Risk: pending scroll callbacks from older targets mutate newer navigation state. Mitigation: callbacks must inspect `pending_precise_scroll` at execution time and verify the target editor/index still matches the pending target.
- Risk: clearing pending scroll too aggressively preserves the bug. Mitigation: unit-test the exact case where a target `ViewportUpdated` fires before `character_vertical_bounds` is available; pending state must remain until a successful retry.
- Risk: scroll preservation restores the pre-Find anchor after item height invalidation. Mitigation: update or clear scroll context when performing Find-driven scrolling.
- Risk: tests depending on actual lazy layout are flaky. Mitigation: prefer focused unit tests around the pending-scroll helper plus one integration-style test that uses the existing code review test utilities to exercise a long diffset.

## Testing and validation
- Add a focused test around the pending precise-scroll helper:
  - configure a pending target
  - make `get_match_character_bounds` return unavailable or simulate a render state without bounds
  - fire the target viewport/layout event
  - assert the pending target remains
  - make bounds available
  - fire the retry path
  - assert vertical/horizontal scroll occurred and pending state cleared
- Add a regression test in `app/src/code_review/code_review_view_tests.rs` or a sibling integration test using `code_review_view_integration.rs` helpers:
  - create multiple file diffs with enough content to place a target match outside the viewport
  - select a branch-style diff mode such as `DiffMode::MainBranch`
  - run Find and navigate to a match in a later file
  - assert `visible_anchor_for_test` or list state ends at the selected file/line after one navigation action and layout settle
- Extend coverage for upward navigation and wraparound if an existing helper can do so without large fixture cost.
- Add stale-state coverage: create a pending scroll, change query or diff mode, then deliver a delayed viewport event and assert the viewport does not jump to the stale target.
- Manual validation on desktop dev build:
  - long branch diff against `master`, next through matches across files
  - previous through matches across files
  - same sequence in "Uncommitted changes"
  - change query/close Find while an off-screen target is pending and verify no delayed jump

## Follow-ups
- If repeated issues appear in other code review jump flows, consider reusing the same durable pending-scroll helper for comment navigation and file-sidebar jumps.
- Longer term, list item height estimation for lazily rendered code review editors could expose a first-class "scroll to editor offset after layout" API. That is larger than this bug fix and is not required for issue #10396.
