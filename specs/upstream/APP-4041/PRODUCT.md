# /queue Slash Command & Auto-Queue Toggle

Linear: [APP-4041](https://linear.app/warpdotdev/issue/APP-4041/add-queue-command)
Figma: none provided

## Summary

Two complementary features for queuing follow-up prompts while the agent is responding:
1. `/queue <prompt>` — a slash command that queues a specific prompt.
2. **Auto-queue toggle** — a status bar button (and keyboard shortcut) that causes the next regular input submission to be queued instead of sent immediately.

When the agent finishes, the queued prompt is automatically sent to the same conversation.

## Problem

When the agent is mid-response the user often already knows what they want to ask next, but they have to wait for the agent to finish before they can type and send it. This breaks flow and wastes time.

## Goals

- Let users queue exactly one follow-up prompt while the agent is busy.
- Provide visual feedback that a prompt is queued and will be sent automatically.
- Let users cancel the queued prompt before it fires.
- Automatically send the queued prompt when the agent finishes successfully; gracefully handle failures.

## Non-goals

- Queuing multiple prompts in sequence (only one at a time).
- Providing a persistent prompt queue that survives app restart.

## User Experience

### Invoking `/queue`

1. The command is `/queue <prompt>` and appears in the slash command menu when the `QueueSlashCommand` feature flag is enabled (dogfood).
2. The prompt argument is required; `/queue` with no argument shows an error toast.
3. The command is only available when the user is in an agent view with an active conversation.
4. If the agent is currently responding (in-progress or blocked), the prompt is queued and the input buffer is cleared.
5. If the agent is idle (conversation finished), the prompt is sent immediately — same as typing it and pressing enter.

### Auto-queue toggle

6. A toggle button appears in the warping indicator (status bar) when the agent is responding, gated behind `QueueSlashCommand`.
7. The button uses a `ClockPlus` icon; when active the icon is accent-colored, otherwise disabled-colored.
8. Keyboard shortcut: `Cmd+Shift+J` (Mac) / `Ctrl+Shift+J` (Linux/Windows).
9. When the toggle is on and the user submits any input (regular prompt, slash command, or skill command) while the conversation is in progress, the input is queued instead of sent immediately.
10. The toggle persists across exchanges in the same conversation (same semantics as fast-forward / auto-approve).

### Pending query indicator

11. A "pending user query" block appears at the bottom of the blocklist showing:
    - The user's avatar and display name.
    - The queued prompt text (dimmed, bold).
    - A "Queued" badge (italic, smaller font).
    - A dismiss button (X icon) in the top-right corner with a "Remove queued prompt" label.
    - A "Send now" button (Play icon) when the queued prompt is interruptible (i.e. from `/queue` or auto-queue, but not from summarization-triggered queuing like `/compact-and`).
12. The pending block is gated behind the `PendingUserQueryIndicator` feature flag (dogfood).

### Dismissal

13. Clicking the dismiss button (X) removes the pending query block and cancels the queued prompt — it will not be sent.
14. Clicking the "Send now" button removes the pending query block and immediately submits the queued prompt, interrupting the in-progress conversation.
15. Exiting the agent view while a prompt is queued also cancels it.

### Auto-send behavior

16. When the agent finishes successfully (`FinishReason::Complete`), the queued prompt is re-submitted through the normal input flow (so slash commands, skill commands, and session sharing are all handled correctly).
17. When the agent finishes with an error, cancellation, or cancelled-during-command-execution, the queued prompt is placed back into the input buffer (so the user doesn't lose it) instead of being sent.

### Interaction with other commands

18. Both `/queue` and the auto-queue toggle reuse the same `send_user_query_after_next_conversation_finished` mechanism as `/compact-and` and `/fork-and-compact`, meaning only one queued prompt can exist at a time (the latest one wins).

## Success Criteria

- `/queue fix the tests` queues the prompt, shows the pending indicator, and auto-sends "fix the tests" once the agent finishes.
- `/queue` with no argument shows an error toast and does not clear the input.
- `/queue` when the agent is idle sends the prompt immediately.
- `/queue` when there is no active conversation shows an error toast.
- Clicking the dismiss X removes the pending block and prevents the prompt from sending.
- If the agent errors or is cancelled, the prompt text is restored to the input buffer.
- The `active_ai_block` and `last_ai_block` helpers correctly skip over the pending query block (same as they skip usage footers).
- Auto-queue toggle: toggling on and submitting during an in-progress conversation queues the prompt.
- Auto-queue toggle: toggle state persists across exchanges until the user explicitly toggles it off.

## Validation

- Manual: invoke `/queue` during an in-progress conversation, verify indicator appears and prompt auto-sends on completion.
- Manual: invoke `/queue` then dismiss, verify prompt does not send.
- Manual: invoke `/queue` then cancel the agent, verify prompt is placed in input buffer.
- Manual: invoke `/queue` when agent is idle, verify prompt is sent immediately.
- Manual: invoke `/queue` with no argument, verify error toast.
- Manual: toggle auto-queue on, type a prompt and press enter during in-progress conversation, verify it queues.
- Manual: verify auto-queue toggle persists after a queued prompt is sent.
- Code review: verify `active_ai_block` and `last_ai_block` skip `is_pending_user_query()` entries.
