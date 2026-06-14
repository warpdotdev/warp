# Cross-Window Tab Drag Redrag — Tech Spec
Product spec: `specs/pei/cross-window-tab-drag/redrag/PRODUCT.md`

## Problem
The cross-window tab drag state machine can put a detached tab back into the source window during the same mouse-down gesture, but it does not correctly support dragging that same inserted tab back out again before mouse-up.

The initial investigation showed that the second drag-out is detected, but the state transition that should restore the preview is incomplete. A follow-up implementation attempted to route the caller-owned reverse handoff through `Workspace`, but that approach caused the drag not to continue and introduced visual bugs. That implementation has been reverted; this document captures the findings so the next fix can be designed more deliberately.

## Relevant Code
- `app/src/workspace/cross_window_tab_drag.rs` — singleton model for active cross-window tab drag state, attach-target hit testing, handoff, reverse-handoff, drop finalization, and preview-window ownership.
- `app/src/workspace/view.rs` — `Workspace::on_tab_drag`, source-side drag initiation, handoff dispatch, tab extraction/insertion helpers, preview workspace setup.
- `app/src/workspace/view.rs` — `Workspace::perform_handoff`, which currently performs put-back into the source for multi-tab source windows.
- `app/src/workspace/view.rs` — `get_tab_transfer_info_for_attach`, `prepare_for_transferred_tab_attach`, `insert_transferred_tab_at_index`, and `remove_tab_without_undo`.
- `WorkspaceRegistry` — lookup path used by the singleton to find source, preview, and target workspaces.
- `DraggableState` — carries the in-progress pointer drag behavior and overlay state for the tab element.
- `WindowManager` and `ctx.transfer_view_tree_to_window` — move the pane-group view tree between windows.

## Current Behavior

### First drag-out
For a multi-tab source window, dragging outside the tab bar starts a cross-window drag:
- source workspace keeps a placeholder or detached state for the dragged tab
- a dedicated preview window is created
- the pane group is transferred from the source window to the preview window
- the preview follows the cursor

Observed logs from the original investigation included:
- `preview_should_create=true active_drag=false`
- `preview_created preview_wid=1 source_wid=0 source_tab_index=1`
- `begin_multi_tab_drag source_wid=0 preview_wid=1 source_tab_index=1`

### Drag back into source
When the cursor re-enters the source tab bar, the existing path treats this as a handoff back to the caller/source:
- `on_drag_while_floating` returns `HandoffNeeded`
- `Workspace::perform_handoff` calls `execute_handoff_back_to_caller`
- the tab is transferred from the preview back into the source tab list
- the preview is hidden and kept alive so it can potentially be reused
- the active drag phase becomes `InsertedInTarget`

Observed logs included:
- `on_drag_while_floating -> HandoffNeeded (back-to-caller) target_wid=0 insertion_index=0 caller_wid=0`
- `perform_handoff branch=target==caller (put-back)`
- `execute_handoff_back_to_caller -> InsertedInTarget target_wid=0 insertion_index=0`
- `mark_source_placeholder_consumed source_wid=0 preview_wid=1 source_tab_index=1 phase=InsertedInTarget`

### Second drag-out failure
When the cursor leaves the source tab bar again without mouse-up, the model detects that a preview should be restored, but the reverse-handoff path cannot complete correctly.

Observed logs included:
- `preview_should_create=true via active reverse_handoff caller_wid=0 target_wid=0 target_insertion_index=0 preview_wid=1 source_placeholder_consumed=true (InsertedInTarget->Transitioning)`
- `reverse_handoff caller_wid=0 target_wid=0 target_insertion_index=0 (phase Transitioning->Floating)`
- `reverse_handoff missing target workspace target_wid=0 caller_wid=0 preview_wid=1 -> Floating`

The important detail is that `target_wid` is the caller/source workspace. During `Workspace::on_tab_drag`, that workspace is already borrowed. Looking it up through `WorkspaceRegistry` from inside the singleton can fail, so the singleton cannot extract the inserted tab and move it back into the preview.

## Findings

### Finding 1: The second drag-out detection fires
The bug is not simply that the drag-out threshold fails after put-back. Logs show the active drag handler reaches the reverse-handoff path when the tab leaves the source tab bar again.

This means any fix should preserve the existing attach-target/redrag detection and focus on ownership transfer and visual state continuity.

### Finding 2: The source/caller workspace is special
The source workspace is directly available as `self` in `Workspace::on_tab_drag`, but it may not be safely retrievable through `WorkspaceRegistry` during the same event callback.

The registry path is acceptable for other windows that are not currently borrowed by the event handler. It is fragile for `target_window_id == caller_window_id`.

### Finding 3: Put-back makes the original source index stale
After the first put-back, the original detached source placeholder has already been consumed. The stored `source_tab_index` no longer necessarily identifies a placeholder for the dragged tab.

Any later drop or reverse-handoff path that uses the original `source_tab_index` risks:
- removing the wrong tab
- hiding an unrelated tab as a zero-width placeholder
- duplicating the same pane group in two windows
- persisting duplicate terminal pane UUIDs

### Finding 4: Preview ownership must be explicit
After put-back, the source owns the dragged tab again and the preview is hidden. After redrag, the preview should own the tab again.

The previous attempted fix added a flag for "source placeholder consumed but tab is back in preview" and adjusted finalization around it. That direction captured a real ownership distinction, but the implementation still failed visually because ownership, `DraggableState`, overlay suppression, preview visibility, and source tab-list mutation were not all updated as one atomic state transition.

### Finding 5: Restoring the preview is more than showing a window
A successful redrag transition needs all of these to agree:
- the pane-group view tree is transferred back to the preview window
- the source removes the inserted tab or otherwise stops rendering it
- the preview tab list contains the transferred tab
- the preview is visible, alpha-restored, focused or focus-suppressed appropriately
- the draggable overlay continues or is rebound without losing pointer capture
- source and preview workspaces notify render state in the right order
- finalization knows whether source cleanup is still needed

The reverted fix made the preview own the tab again in code, but the user-observed result was that dragging the tab out again did not continue the drag and produced visual bugs. This suggests the next fix should treat redrag as a first-class state transition rather than patching only the registry lookup failure.

## Reverted Attempt
The reverted implementation attempted these changes:
- add a `ReverseHandoffNeeded` drag result from `CrossWindowTabDrag`
- return that result when `target_window_id == caller_window_id`
- add `Workspace::perform_reverse_handoff_to_preview` so the already-borrowed caller workspace could extract the tab directly from `self.tabs`
- transfer the pane group back to the preview
- reinsert the tab in the preview workspace
- add a flag tracking whether a consumed source placeholder's tab had moved back into the preview
- adjust finalization to avoid stale source-index cleanup

Why it was insufficient:
- The drag did not continue after the second drag-out.
- Visual state became inconsistent.
- The implementation likely changed ownership without preserving the correct `DraggableState` and overlay behavior needed for the active pointer drag.
- Source/preview rendering and preview-window visibility were not treated as one coordinated transition.
- The fix was too broad in finalization without a proven state-machine model for all redrag cases.

This approach should not be re-applied as-is.

## Technical Requirements for the Next Fix

### State machine
Represent the redrag path explicitly in the drag lifecycle. The model should distinguish at least:
- floating in preview
- ghosted over a target without transfer
- inserted in a target/source
- transitioning between owner workspaces

For multi-tab source redrag, it should also be explicit which workspace currently owns the real dragged tab:
- preview owns tab
- source owns tab
- another target owns tab

Avoid relying on the original source placeholder index after that placeholder is consumed.

### Caller-owned reverse handoff
When the inserted target is the same as the caller/source workspace, the extraction probably needs to be performed by `Workspace`, not by a registry lookup inside `CrossWindowTabDrag`.

However, the handoff API should not just move the pane group. It should return or accept a complete transition payload that includes:
- tab transfer data
- active draggable state strategy
- source cleanup strategy
- preview update strategy
- finalization ownership state

### Draggable continuity
The redrag transition must preserve the in-progress drag. Before implementing another ownership transfer, inspect how `DraggableState` is used for:
- pointer capture
- drag overlay painting
- `cancel_drag`
- `adjust_mouse_position`
- suppressing overlay paint
- drag element identity when a tab moves between workspaces

The second drag-out should not cancel the active drag unless a new draggable state is immediately bound in a way that preserves the current pointer gesture.

### Preview visibility
Preview reactivation should normalize all preview-window visual state:
- show the preview
- restore alpha to `1.0` if a prior ghost path set it to `0.0`
- ensure preview chrome remains in preview mode until final drop
- ensure focus suppression behavior matches the original floating preview path
- notify both source and preview workspaces

### Source cleanup
After put-back, source cleanup cannot be expressed as "remove original source index." Instead, cleanup should be based on current owner and current inserted target.

Potentially useful model:
- store a current owner enum rather than a consumed-placeholder boolean
- store current owner tab index only when owner is a workspace tab list
- clear original source placeholder information once it is no longer valid

### Drop finalization
Finalization should derive its cleanup action from the current owner state:
- preview owns tab and final state is floating: promote preview, no source cleanup if source placeholder was already consumed
- source owns tab and final state is inserted in source: close/discard preview only
- other target owns tab: close/discard preview and only remove source placeholder if it still exists

Avoid stale-index cleanup.

## Open Questions
- Should the source tab be removed immediately on first detach, or should the placeholder remain as a distinct typed placeholder until finalization?
- Should put-back into the source transfer the real pane group back immediately, or should the source render a ghost/proxy until mouse-up?
- Can redrag be implemented by keeping the preview as the real owner throughout the gesture and rendering the source insertion as a proxy instead of transferring ownership back and forth?
- Which `DraggableState` should own the active pointer gesture after the tab moves back into the preview?
- Are the visual bugs from the reverted fix caused by overlay suppression, preview alpha/focus state, duplicate `TabData`, or event ordering?

## Recommended Direction
Prefer a state-machine refactor over another localized registry workaround.

The likely robust direction is to minimize real ownership transfers during the gesture:
- keep one canonical owner for the dragged pane group where possible
- render source/target insertion affordances as ghosts/proxies until drop where possible
- perform real pane-group transfer only when necessary for visible tab content or finalization

If immediate transfer back into the source is required for current UX, then model it as a first-class owner transition with a complete transition payload and tests for each final owner/drop combination.

## Validation Plan
- Add targeted logging for:
  - current drag phase
  - current real owner window
  - current owner tab index
  - preview visibility/alpha
  - source placeholder validity
  - draggable state transition points
- Manual test the primary redrag flow from the product spec.
- Manual test repeated out/in cycles before mouse-up.
- Manual test release during each state:
  - floating preview
  - inserted in source
  - ghosted over another target
  - inserted in another target
- Run `cargo fmt --all`.
- Run `cargo check -p warp`.
