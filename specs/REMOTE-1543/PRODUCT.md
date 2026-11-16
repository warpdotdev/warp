# Queued Prompts UI

## Summary
Introduce a collapsible "queued prompts" panel for regular Agent Mode queued prompts that supports multiple prompts, in-place editing, drag-to-reorder, and per-row delete.
Queued prompts run sequentially as the agent finishes each preceding exchange.
## Problem
Today regular Agent Mode queueing only supports a single follow-up prompt at a time.
Re-queueing replaces the previous prompt, the user can't reorder or edit what's pending, and there's no way to deal with multiple in-flight ideas without losing work.
## Goals
- Let users queue any number of follow-up prompts while the agent is responding, and have them auto-fire in order.
- Make the queued prompts visible, reorderable, editable, and individually removable from a single panel.
- Preserve the existing regular queue trigger surfaces: the auto-queue toggle and `/queue` slash command.
- Keep compatibility placeholder flows (`/compact-and`, `/fork-and-compact`, Cloud Mode prompts) on their existing pending-user-query UI instead of broadening the new panel. Expanding those surfaces to also use the queued prompt panel will come in a separate, follow-up PR.
- Reuse `QueueSlashCommand` as the regular queue rollout gate for both the trigger surfaces and the visual panel — no new flags introduced.
## Non-goals
- Persisting the queue across app restarts.
- Cross-conversation queueing (the queue belongs to the conversation it was filed against).
- Reordering or editing the prompt that's currently executing (only items still pending in the queue are editable).
- A "Send now" affordance to interrupt the in-flight exchange (explicitly removed; users cancel via the existing stop button if they want to fire a queued prompt earlier).
## Behavior
### Queue panel placement and visibility
1. The queue panel renders between the warping indicator (status bar) and the agent input box, anchored to the bottom of the conversation area, in the same vertical slot the inline menu uses when it's open.
2. The panel is visible whenever the active conversation has at least one queued prompt; otherwise the panel is not rendered (no empty state).
3. The panel has a header `"<N> queued"` with a chevron icon. The body of the panel (everything below the header) is what collapses, not the header itself. Clicking the chevron (or anywhere on the header) toggles the panel between expanded (header + rows visible) and collapsed (only the header is visible). Default state is expanded. The collapsed state persists for the lifetime of the queue (across re-orderings, edits, deletions, additions). Adding a new prompt while collapsed does not auto-expand.
4. `/queue`, the auto-queue toggle, and the visual queue panel continue to be gated by `QueueSlashCommand`. `/compact-and` continues to be gated by `SummarizationConversationCommand`, and `/fork-and-compact` follows the existing fork-command availability. `PendingUserQueryIndicator` remains compatibility infrastructure for legacy pending-user-query placeholders, not a rollout switch for the regular queue panel.
### What gets queued
5. The auto-queue toggle in the warping indicator is per-conversation. When on for the active conversation, any prompt the user submits while that conversation is in progress (`InProgress` or `Blocked`) is appended to that conversation's queue rather than sent. When off, regular submits still cancel-and-resend (existing behavior). Toggling the button toggles the state for the active conversation only; switching to a different conversation shows that conversation's own toggle state. New conversations default the toggle to off.
6. `/queue <prompt>` appends `<prompt>` to the queue when the active conversation is in progress, and behaves like a normal send when the conversation is idle (existing semantics).
7. `/compact-and <prompt>` and `/fork-and-compact <prompt>` do not create queued-prompts panel rows. Their follow-up prompts stay on the legacy pending-user-query UI while summarization or fork-then-summarization runs.
8. Cloud Mode prompts (both Oz and third-party harness flows) do not create queued-prompts panel rows. Their placeholders stay on the legacy pending-user-query UI and remain lifecycle-owned by the cloud setup / shared-session flow.
9. Submitting in shell mode (input type is Shell, not AI) is never queued — it runs in the terminal as today, regardless of toggle state or in-progress AI status.
10. `/queue` with an empty argument shows an error toast and does not modify the queue (existing behavior).
11. Queues and the auto-queue toggle state are owned per-conversation app-wide. Each conversation has its own queue and its own toggle state, both of which persist across agent-view exit, switching between conversations, and re-entry into the agent view. Switching to a different conversation shows that conversation's queue and toggle state; switching back restores the prior conversation's state. Queue and toggle state are dropped only when the conversation itself is deleted, or when its owning terminal view's conversations are cleared.
### Queue rows
12. Each queue row shows, left to right:
   - A drag handle icon (six-dot grid).
   - A compact multiline prompt preview, capped by both displayed height and character count so long prompts stay scannable in the queue.
   - On hover: a pencil (edit) and a trash (delete) icon-button, right-aligned.
13. Hovering a row reveals the edit/delete icons. Moving the cursor off the row hides them.
14. Every row in the queued-prompts panel is a regular user-managed queued prompt, so the row interactions in (12)–(13) apply uniformly to every visible panel row.
15. Rows render in queue order from top (next to fire) to bottom (last to fire).
### Edit interaction
16. Clicking the pencil icon on a row replaces the row's static preview with an inline multiline editor pre-filled with the current prompt text and selects the entire prompt.
17. The editor is visually outlined while editing, grows until it reaches the same visual line cap as the static row preview, then scrolls internally with a visible scrollbar. Pressing Enter commits the edit (the row's prompt is replaced with the editor contents) and exits edit mode. An empty edit restores the original prompt text and exits edit mode.
18. Pressing Escape cancels the edit and restores the original prompt text. Clicking outside the row, including focusing the main input, commits the current editor text.
19. While a row is in edit mode, that row's drag handle is inert (the row cannot be reordered until the edit is committed or cancelled). Other rows can still be dragged.
20. Only one row can be in edit mode at a time. Clicking the pencil on a different row exits edit mode on the previous row without changing that row's last committed text, then enters edit mode on the new one.
21. Auto-fire never sends a row that is currently in edit mode. If the active conversation reaches `FinishReason::Complete` while the first queue row is in edit mode, then:
   - If the main input is empty, that row is removed from the queue and — mirroring the delete behavior in (23)–(24) — the row's prompt text (its last committed value, not any uncommitted text still in the inline editor buffer) is placed in the main input box, and the input is focused.
   - If the main input is non-empty, that row stays in the queue and the input is not modified.
   Other queue rows are not affected and resume normal sequential firing on the next completion. If the row being edited is not the first row, auto-fire proceeds normally for the actual first row; the edited row is left in place and can become the next-to-fire after rows ahead of it drain.
### Delete interaction
22. Clicking the trash icon on a row removes that row from the queue.
23. If the input box is empty when a row is deleted, the deleted row's prompt text is placed in the input (replacing the empty buffer); the input gains focus.
24. If the input box is non-empty when a row is deleted, the deleted prompt is discarded — the input is not modified.
25. Deleting the last visible row in the queue removes the panel (since the queue is now empty); the collapsed/expanded state resets for any future queue.
### Drag-to-reorder
26. Dragging a row vertically reorders it within the queue. The dragged row is visually highlighted while the queue live-reorders around it.
27. Dropping the row commits the new order. The first row of the new order is what will fire next.
28. Dragging is constrained to the vertical axis — horizontal motion does not change order.
29. Rows reflow as the dragged item crosses the midpoint of a neighboring row, making the tentative new order visible before drop. Reflow live-mutates the queue, so auto-fire (and any other read of the queue head) always sees the current post-drag order rather than the pre-drag order.
30. If the queue is mutated by something other than the drag itself while a drag is active, behavior depends on which row was mutated. If a different row is removed (e.g. auto-fire pops the current head while the user is dragging a non-head row) or appended, the drag continues against the updated queue and subsequent reflow uses the new neighbor positions. If the dragged row itself is removed (only reachable when the dragged row is currently the head and auto-fire pops it), the row disappears and is submitted as the next user query, and the drag is cancelled silently: no reorder is committed, no reorder telemetry fires, and any further mouse motion before mouseup is inert.
31. Legacy pending-user-query placeholders for Cloud Mode, `/compact-and`, and `/fork-and-compact` are outside this panel, so they do not participate in drag-to-reorder.
### Sequential firing
32. When the active conversation reaches `FinishReason::Complete`, the first prompt in the queue is removed and submitted as the next user query in the same conversation, routed through the normal submission path so slash, skill, and session-sharing paths are handled correctly.
33. While that newly-fired prompt is mid-exchange, the rest of the queue stays intact, the panel updates the count to `<N-1> queued`, and additional prompts can still be queued at the tail.
34. The cycle continues until either the queue is empty or one of the abort conditions in (35) fires.
### Cancellation and error handling
35. When the active conversation finishes for any non-`Complete` reason — `Error`, `Cancelled`, `CancelledDuringRequestedCommandExecution` — auto-fire pauses immediately. The queue is not flushed.
36. When auto-fire pauses for one of those reasons, the head-restore behavior depends on whether the user is currently viewing the cancelled conversation in agent view:
   - If the user is **not** viewing this conversation in agent view (e.g. they triggered the cancel by exiting the agent view, or the conversation was running in the background), the queue is left fully intact — neither the head nor any other row is placed into the input.
   - If the user **is** viewing this conversation in agent view (e.g. stop button or `Ctrl-C` while in agent view):
     - If the input is currently empty, the first queued prompt is removed from the queue and its text is placed in the input box. The row is removed in this case so that re-submitting the input does not also re-fire the same prompt from the queue.
     - If the input is non-empty, the first prompt's text is not placed in the input and the queue is left intact (the first prompt remains in the queue at position 0).
   - In all cases all queue rows beyond the first remain intact in the panel, so the user can review, edit, reorder, delete, or send further prompts.
37. Auto-fire resumes naturally the next time the active conversation reaches `FinishReason::Complete` — i.e. the user re-runs or sends a new prompt that succeeds, and from that completion onward the queue resumes draining from the top.
38. Manually cancelling the in-progress agent (stop button or `Ctrl-C` shortcut) is treated as `Cancelled` for the purposes of (35)–(36).
### Conversation lifecycle interactions
39. Exiting the agent view (Esc to terminal, closing the tab/pane) does not discard the queue for that conversation. Re-entering the agent view for that conversation later restores its queue and auto-queue toggle. This matters especially for cloud agents, whose conversations continue running in the background after the user leaves the agent view; their queues continue to drain when the conversation eventually finishes, even if the user has navigated away.
40. Starting a new conversation begins with an empty queue and the auto-queue toggle defaulted off — each conversation has its own queue and toggle, so prior conversations' state is unaffected.
41. The queue belongs to a conversation; if the agent splits the conversation (`/fork`, `/fork-and-compact`), regular queued-prompts panel rows do not carry into the new conversation. Any summarize/fork follow-up placeholder behavior remains separate legacy pending-user-query UI, not queue-panel state.
### Focus
42. The auto-queue toggle keybinding (`Cmd-Shift-J` / `Ctrl-Shift-J`) is unchanged.
43. Submitting from the main input always returns focus to the main input, even when the submission appended to the queue.
### Telemetry
44. Existing `/queue` and auto-queue telemetry events continue to fire. Queue-panel-specific interactions (edit committed, row deleted, row reordered, panel collapsed/expanded) are tracked as new telemetry events so we can measure usage of the new affordances.
