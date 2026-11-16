# Queued Prompts in Cloud Mode Setup

Linear: APP-4562. Builds on the regular Agent Mode queued-prompts panel from [`specs/REMOTE-1543/PRODUCT.md`](../REMOTE-1543/PRODUCT.md), extending it to Cloud Mode runs.

Figma: none provided.

## Summary
Extend the multi-prompt queued-prompts panel to Cloud Mode runs so the initial cloud prompt and any follow-ups queued during environment setup render in the same panel as regular Agent Mode queued prompts, with the initial prompt rendered as a locked first row. Subsequent queued rows fire automatically as the cloud agent finishes each exchange.

## Problem
Today in Cloud Mode setup, the user's submitted prompt appears as a separate "pending user query" indicator block, and pressing Enter while the cloud environment is setting up does nothing — the prompt is dropped. Queued prompts also do not drain for cloud runs, because the local "response finished" signal that drives draining does not fire when the response is being streamed by a remote cloud agent. As a result, users cannot queue follow-up work on a cloud run.

## Goals
- Render the initial Cloud Mode prompt as a row in the regular queued-prompts panel instead of as the legacy pending-user-query block.
- Let users queue any number of follow-up prompts while the cloud environment is setting up, and have them auto-fire in order once the cloud agent is live.
- Keep the panel's row interactions (drag-to-reorder, edit, delete) for follow-up rows, while preventing them on the initial row that the cloud agent has already accepted.
- Gate everything behind a new `QueuedPromptsV2` feature flag, dogfood-only.

## Non-goals
- Changing the regular Agent Mode queued-prompts panel behavior described in `specs/REMOTE-1543/PRODUCT.md`.
- Persisting the cloud-mode queue across app restarts.
- Letting users edit, delete, or reorder the initial prompt after it has been dispatched to the cloud.
- Exposing this behavior to non-dogfood builds.

## Behavior

### Feature gating
1. All behavior described below is gated on the `QueuedPromptsV2` feature flag. When the flag is off, Cloud Mode setup behaves exactly as it does today: initial prompts render as the legacy pending-user-query block, submitting a prompt while the environment is setting up is a no-op, and queued prompts do not drain for cloud runs.
2. The `QueuedPromptsV2` flag transitively enables the regular `QueueSlashCommand` feature. When V2 is on, every existing regular Agent Mode queued-prompts surface (the auto-queue toggle, `/queue` slash command, queue panel) is also available.

### Initial cloud-mode prompt
3. When the user submits the initial prompt in a Cloud Mode pane, the prompt appears as the first row of the queued-prompts panel for that conversation. The panel renders in the same position relative to the V2 cloud-mode composing input that it renders in for regular Agent Mode (above the input editor, inside the centered V2 layout).
4. The initial cloud-mode prompt row is *locked*:
   - Its drag handle is rendered in a visually disabled state and does not respond to drag gestures.
   - Its edit (pencil) and delete (trash) icon-buttons remain rendered on hover and render with the same naked styling as their interactive counterparts on other rows — no greyed-out background or text. They are not clickable.
   - Hovering the drag handle or either icon-button shows the same short tooltip — "The first cloud-mode prompt cannot be changed." — explaining why no interaction is possible.
   - The static preview text renders identically to other rows.
5. The locked row's preview text is the prompt as the user typed it, including any `/plan`, `/orchestrate`, or other prefix the user included (matching today's pending-user-query block treatment).
6. When the cloud agent picks up the prompt — i.e. the first real exchange shows up in the conversation transcript, or the harness reports that the command has started (Oz harnesses use the harness-command-started signal; oz local-to-cloud handoff uses the first appended exchange) — the locked row is removed from the panel. After removal, the second row (if any) becomes the next row to fire, but is still considered a follow-up, not the initial prompt.
7. If the cloud run fails before the prompt is picked up (failure, cancellation, GitHub-auth required, snapshot upload failure), the locked row is removed from the panel at the same moment the legacy pending-user-query block would be removed today. Any follow-up rows queued behind it remain in the panel, available for review, edit, deletion, or reordering, exactly like regular queued rows.

### Submission during environment setup
8. Whenever the cloud pane is an ambient-agent pane that is not currently composing and not currently running, submitting the input editor queues the prompt instead of doing nothing. The queued prompt appears as a new row in the panel, after the locked initial row. In practice this covers every pre-run cloud state where the user can reach the editor — `WaitingForSession`, `Failed`, `Cancelled`, `NeedsGithubAuth`, and `Setup` (the last is normally unreachable because the first-time-setup modal owns the focus, but the predicate matches it for completeness).
9. Follow-up rows queued during setup are *not* locked. They support the same interactions as regular Agent Mode queued rows: drag-to-reorder among themselves, hover-revealed edit and delete buttons, and so on.
10. The locked initial row stays pinned at index 0 regardless of how follow-up rows are reordered. Dragging another row above the locked row is not possible — the panel keeps the locked row at the top.
11. Submitting an empty prompt does not append a new row (existing trim-and-skip behavior).
12. Submitting in shell mode is unaffected — the shell command runs in the terminal as today, regardless of whether the cloud agent is setting up.

### Drain behavior (after the initial prompt is picked up)
13. Once the locked initial row has been removed (per §6), the panel behaves as the regular Agent Mode queued-prompts panel: every time the active cloud conversation finishes an exchange cleanly, the first remaining row is removed from the panel and submitted as a follow-up prompt to the same cloud conversation.
14. The queued prompt is submitted through the same path that user-initiated cloud follow-ups use — it reaches the cloud agent, not the local agent controller. From the user's perspective, an auto-fired queued prompt is indistinguishable from a prompt the user typed and submitted manually after the agent finished.
15. When the active cloud conversation finishes for a non-clean reason (error, cancellation, cancellation during requested command execution), auto-fire pauses immediately. The queue is not flushed:
    - If the input editor is currently empty, the first remaining queued row is removed from the panel and its text is placed in the input editor. The user can edit and re-submit it manually.
    - If the input editor is non-empty, no rows are removed and the input is not modified.
    - In both cases, remaining queued rows beyond the first stay intact.
16. Auto-fire resumes naturally the next time the active cloud conversation completes an exchange cleanly — from that completion onward, the queue resumes draining from the top.

### Conversation lifecycle interactions
17. The queued-prompts panel is owned by the terminal view and implicitly scoped to whichever conversation is currently active in that view. Switching to a different conversation goes through agent-view exit (which clears the queue) before re-entering for the new conversation, so the panel always reflects the active conversation and never carries follow-up rows across conversation switches.
18. Exiting the cloud pane, closing the tab, or removing the conversation discards the queue (including any locked initial row).
19. The collapsed/expanded state of the panel, the row-level edit state, and reorder behavior all match the regular Agent Mode queued-prompts panel for follow-up rows.

### Telemetry
20. Existing queued-prompts panel telemetry (edit committed, row deleted, row reordered, panel collapse toggled) continues to fire for follow-up rows. The locked initial row does not emit edit/delete/reorder events because those interactions are disabled.
