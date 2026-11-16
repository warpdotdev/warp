# REMOTE-1590: Hide sharer's input cursor in cloud agent sessions

## Context
When viewing a cloud agent session running a third-party CLI agent (e.g. Claude Code), the viewer sees a phantom remote cursor in the rich input — rendered with the sharer's avatar — even though the sharer is a headless agent worker that never interactively types in the input.

### How remote input cursors work
The input editor is a CRDT-backed collaborative buffer. For a remote cursor to render, two conditions must be met:
1. The participant is **registered as a remote peer** in the editor's buffer (`view_impl.rs:1150-1157` for the sharer, `view_impl.rs:1122-1129` for viewers).
2. The buffer has **remote selection data** for that peer's replica ID, populated by `UpdateSelections` CRDT operations received from the sharer.

Condition 1 is always met: `handle_presence_manager_event` unconditionally registers the sharer as a remote peer when a viewer joins any shared session (`view_impl.rs:1132-1158`).

Condition 2 was historically never met for cloud agent sessions because the worker's input buffer was inert — no code path on the worker ever called `edit()` on the rich input buffer.

### What changed
The CLI agent rich input feature introduced lifecycle events (`InputSessionChanged::Open` / `Closed`) that call `clear_buffer_and_reset_undo_stack` on the worker's input (`input.rs:2477`, `2492`). Unlike `reinitialize_buffer` (which creates a fresh buffer without emitting CRDT operations), `clear_buffer_and_reset_undo_stack` goes through the full `edit()` → `end_batch()` → `UpdatePeers` path, emitting CRDT selection operations. These operations are transmitted to the viewer via session sharing (`local_tty/terminal_manager.rs:2312-2326`), satisfying condition 2.

This does not affect the Oz harness because it does not use the CLI agent rich input — its buffer clears go through `reinitialize_buffer` during block completion (`input.rs:13448-13449`), which creates a fresh buffer without emitting CRDT operations.

### Why the cursor draws
`input_data_for_participant` (`presence_manager.rs:726`) sets `should_draw_cursors: true` when the participant has `Selection::None` in its presence data. The cloud agent worker — running a CLI agent in the alt screen with no block or text selected — satisfies this.

## Proposed changes
Filter **selection-only** CRDT operations (`UpdateSelections`) from the sharer's input broadcasts in ambient agent sessions. The sharer is a headless worker that never types in the rich input; its selection ops would render a phantom cursor on the viewer side. Content operations (`Edit` / `Undo`) are still forwarded so the viewer's buffer stays in sync if the sharer ever writes to the input. Filtering at the source (rather than on the receiver) avoids needing to guard every downstream call site that touches registered peers.

A predicate helper `is_sharer_selection_op` (`local_tty/terminal_manager.rs`) encapsulates the check for `CrdtOperation::UpdateSelections`.

### `local_tty/terminal_manager.rs` — `InputEditorUpdated` handler
For ambient agent sessions, filter the operations through `is_sharer_selection_op` so only content ops are sent. Non-ambient sessions pass operations through unchanged with no allocation.

### `local_tty/terminal_manager.rs` — initial input flush
Filter the initial `latest_buffer_operations()` iterator through the same predicate before cloning, so selection ops from the pre-share buffer are excluded while content ops are still flushed.

### What stays the same
- The sharer is still registered as a remote peer on the viewer side — presence avatars, `set_remote_peer_selection_data`, and `refresh_input_data_for_participants` all continue to work without warnings.
- The viewer's input CRDT operations still flow to the sharer (gated by executor role on the viewer side, unchanged).
- Normal (human-to-human) shared sessions are unaffected: the check is specific to ambient agent sessions.

## Testing and validation
- Manual: start a cloud agent run with a third-party harness (e.g. Claude Code), open the session as a viewer, and confirm no phantom cursor appears in the rich input.
- Manual: start a normal (non-cloud) shared session between two users and confirm remote cursors still appear in the input.
- Existing `presence_manager_tests` continue to pass unchanged.

## Parallelization
Not beneficial — this is a single-file, two-line change.
