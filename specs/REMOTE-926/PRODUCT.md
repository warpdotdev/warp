# REMOTE-926: Reopening a closed cloud-mode tab should rejoin the session

## 1. Summary
When a user closes a tab that is viewing a cloud conversation (ambient agent / shared-session viewer) and then undoes the close via `Cmd-Shift-T` within the grace period, the restored tab should be live again: the viewer reconnects to the ongoing conversation, new blocks continue to stream in, the input is interactive (if the viewer had an editor role), and the "session ended" affordances do not appear. If the grace period expires without a restore, behavior is unchanged from today.

## 2. Problem
Closing a cloud-mode tab today tears down the shared-session viewer's WebSocket, marks the viewer's `SharedSessionStatus` as `FinishedViewer`, clears remote peers, inserts a "session ended" banner, and flips the input to `Selectable` (read-only). The tab is still held on the undo-close stack and can be restored with `Cmd-Shift-T`, but the restore path does not reverse any of that teardown. The user is left looking at a frozen snapshot of the conversation at the moment of close, even though the cloud agent keeps producing output server-side. This is a lossy experience that effectively discards in-flight agent work from the user's perspective.

## 3. Goals
- Restoring a closed cloud-mode tab within the undo-close grace period produces a tab indistinguishable from the one before close: connected, streaming, and interactive.
- Post-close, pre-restore: the tab's WebSocket to the session-sharing server remains healthy so the restore is instant and does not rely on a new rejoin.
- If the user does not restore the tab within the grace period, cleanup and teardown behavior match today exactly (network closes, conversations marked historical, blocks deleted, view dropped).
- No regressions for non-cloud tabs (local terminals, code panes, etc.) — close/restore must behave identically for those.

## 4. Non-goals
- Cross-window tab restore (undo-close across windows).
- Protocol-level "suspended viewer" state — we accept that during the grace period, the sharer may briefly see the closed-tab user in the participant list.
- Restoring a cloud conversation after the tab has been fully discarded from the undo stack — the existing "OpenAmbientAgentSession" / transcript-viewer paths already cover that.
- Changing the default 60s grace period (`UndoCloseGracePeriod`).
- Unifying the TerminalManager close/restore paths across local/remote/shared-session viewers.
- Supporting this for ambient-agent transcript viewers (the read-only "Open Conversation Transcript" flow) — that path does not hold a live WebSocket.

## 5. Figma / design references
Figma: none provided. This change is behavioral and is expected to have no new visual surface area.

## 6. User experience
All behavior applies to a tab whose active pane is a shared-session viewer for a cloud conversation (ambient agent session). Behavior for local-terminal, code, notebook, workflow, etc. panes is unchanged.

### Baseline state
- Tab `T` is open and actively viewing a cloud conversation. The cloud agent may be mid-response, idle, or finished.
- The viewer holds a live WebSocket to the session-sharing server via `shared_session::viewer::Network`, and its `SharedSessionStatus` reflects its role (Viewer, Editor, etc.).
- Input may be interactive or not depending on role; `self.shared_session` on the `TerminalView` is `Some(...)`; no "session ended" banner is present.

### Closing the tab (`Cmd-W`)
- The tab is removed from the tab bar.
- The tab is pushed onto the `UndoCloseStack` with its full `TabData` intact.
- The viewer's network connection and `SharedSessionStatus` are preserved (see invariants below). No "session ended" banner is inserted and the input interaction state is not changed.
- Agent events (new blocks, status updates, etc.) received from the server are still processed by the view model, but since the view is not rendered, nothing is drawn.

### Restoring the tab (`Cmd-Shift-T`) within the grace period
- The tab reappears in the tab bar at its original index and is activated.
- The tab's content matches the cloud conversation's current server-side state at the moment of restore — not the snapshot at close time. Any blocks, status transitions, or selected-conversation changes that occurred while the tab was hidden are visible.
- The input's editability matches the viewer's role, identical to pre-close behavior.
- No "session ended" banner is visible.
- The `shareable_object` for the pane (the link that drives copy-link / share dialog) is whatever it was before close.
- Further agent activity streams in normally.

### Discarding the tab (grace period expires without restore, or stack is bumped past limit)
- Identical to today. Conversations are marked historical, blocks are deleted from SQLite, `ActiveAgentViewsModel` unregisters the view, CLI agent sessions are cleaned up, the WebSocket is closed, `SharedSessionStatus` is set to `FinishedViewer`, and the `TerminalView` is dropped. The conversation remains searchable as historical but is no longer live.

### Restoring a tab after its in-memory WebSocket died
The grace period is bounded, but the WebSocket may still die independently of user action (e.g., network glitch, server restart). If the tab is restored while the network is attempting to reconnect or has entered `Stage::Finished` due to irrecoverable failure:
- If reconnect is in-flight (`Stage::Reconnecting`), the restored tab renders the reconnecting state (existing behavior in `NetworkEvent::Reconnecting`).
- If the network gave up (`Stage::Finished` due to `FailedToReconnect` or `SessionEnded`), the restored tab shows the same "session ended" banner it would have shown had the tab been open when the failure occurred. The restore does not mask genuine session-ended signals from the server.

### Non-cloud tabs
- Close and restore behavior for local-terminal, code, notebook, and other pane types is unchanged.

### Invariants
- While a closed cloud-mode tab sits on the undo-close stack:
  - Its viewer `Network` model is alive and in `Stage::JoinedSuccessfully` (or `Reconnecting`) — it has not been told `close_without_reconnection`.
  - Its `TerminalModel::shared_session_status()` is not `FinishedViewer`.
  - `TerminalView::shared_session` is `Some(...)`.
  - The "shared session ended" banner has not been inserted.
  - The input's `InteractionState` has not been flipped to `Selectable` by `on_session_share_ended`.
  - `ActiveAgentViewsModel` has unregistered the view (conversation moves to "Past" in the conversation list for the duration of the grace period). On restore, `TerminalPane::attach` re-registers with `ActiveAgentViewsModel`, moving the conversation back to "Active".
- On grace-period expiry, all of the above are torn down exactly as they would be if the tab had never been closeable (i.e., matches current "Closed" behavior).
- On restore, no additional work is needed to make the tab live — the view simply becomes visible again and the already-live network drives it.

## 7. Success criteria
- Closing a cloud-mode tab and immediately restoring it (`Cmd-W` followed by `Cmd-Shift-T`) results in a tab that is fully live, identical in state to before close (save for any server-driven updates that happened in between).
- If the user sends a new prompt in the live cloud conversation (e.g., from another client) while the tab is closed, restoring the tab reflects the new agent output.
- After restore, the input editable state matches the viewer's role. If the role was Editor, `Cmd-Enter` / typing works as before.
- After restore, no "session ended" banner is present on the restored tab.
- After grace-period expiry, the conversation transitions from "Active" to "Past" in the conversation list (same as today).
- After grace-period expiry, SQLite no longer contains blocks for that session UUID (same as today).
- Closing and restoring a non-shared-session tab (local terminal, code pane, notebook, workflow) produces identical behavior to today.
- Closing a cloud-mode tab, waiting past the grace period, and then attempting `Cmd-Shift-T` does not reopen the tab (stack is empty) and produces no errors.

## 8. Validation
- Integration test (new): open a cloud conversation viewer, close the tab, simulate the sharer emitting a new terminal event while the tab is closed, restore the tab, assert the new event is visible in the restored view.
- Integration test (new): open a cloud conversation viewer, close the tab, restore the tab, assert no "session ended" banner is present and input is editable (for Editor role).
- Integration test (new): open a cloud conversation viewer, close the tab, wait for grace-period expiry, assert network is closed, `SharedSessionStatus::FinishedViewer` is set, and conversation is marked historical.
- Unit test (new): `TerminalManager::on_view_closed` with `DetachType::HiddenForClose` for a shared-session viewer leaves `network_state` in `Active` and `SharedSessionStatus` unchanged.
- Unit test (new): `TerminalManager::on_view_closed` with `DetachType::Closed` for a shared-session viewer performs today's full teardown.
- Regression test: existing local-terminal close/restore tests continue to pass.
- Manual: run `cargo run` locally, reproduce the original bug (close tab mid-agent-response, `Cmd-Shift-T`), confirm the tab is live and updating.

## 9. Open questions
- During the grace period, the sharer (and any co-viewers) will still see the closing user in the participant list via heartbeats/presence. Is that an acceptable ghost-viewer window, or should we also send a lightweight "presence: suspended" hint to the server so UI elsewhere can dim or hide the ghost? Recommended resolution: accept today and track a protocol-level suspension as a follow-up (Non-goal in §4).
- Should any UI indicate that the tab is "suspended for undo" while it's on the stack (not visible), for debugging or transparency? Recommended resolution: no UI — the tab is not visible to the user during this period.
