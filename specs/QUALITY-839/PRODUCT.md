# Auto-queue prompts during agent-controlled long-running commands

Linear: [QUALITY-839](https://linear.app/warpdotdev/issue/QUALITY-839/auto-enable-prompt-queueing-during-lrc)

## Summary

While an agent is in control of a long-running command (LRC), submitting a prompt auto-queues it instead of immediately sending it to the agent driving the command. The user can then press Enter on an empty input to fire the queued prompt into the running exchange when they actually want it delivered. A new cloud-synced setting controls whether this auto-queue behavior is on.

Figma: none provided.

## Problem

Today, a prompt submitted while an agent controls an LRC is delivered to that agent immediately, steering it mid-command. Users often type thoughts ahead of time and don't want them injected into the running command the instant they hit Enter; they want them held until they deliberately release them or until the exchange finishes.

## Behavior

### Trigger and scope

1. Auto-queue activates for a conversation exactly when the agent holds control of an active long-running command in that conversation. This includes the state where the agent is blocked on user approval to interact with the command.
2. Auto-queue does not activate when the user is in control of the LRC — e.g. before the agent has taken control, or after a manual takeover, stop, or agent-initiated transfer of control back to the user.
3. Auto-queue activation is per-conversation: it affects only the conversation whose agent controls the LRC. Other conversations' queue toggle states are untouched.
4. The behavior is gated on the same feature availability as the existing prompt-queue feature (the queue chip / `/queue` surface). Where the queue feature is unavailable, behavior is unchanged from today.

### Queuing while the LRC runs

5. While auto-queue is active, submitting a non-empty prompt appends it to the conversation's queued prompts (the same queue used by the auto-queue chip and `/queue` today) instead of sending it to the agent. The input clears, and the queued prompts panel shows the row.
6. Pressing Enter on an empty input sends the top queued row immediately — delivered to the same target an immediate submission would have used (the agent controlling the LRC) — per the existing empty-input-Enter send-now behavior. Each press sends exactly one row.
7. All existing queue interactions (panel rows, edit, delete, reorder, send-now buttons, drain-on-completion semantics, pause on error/cancel) behave exactly as they do for manually-enabled queue mode. This feature changes only when queue mode turns on, not how the queue works.
8. Prompts queued during the LRC that are still queued when the exchange finishes normally drain per existing sequential-firing rules.

### Status chip and ghost text

9. While auto-queue is active, the prompt-queue chip in the warping indicator renders in its active (accent-colored) state, identical to when the user enables queue mode manually.
10. While auto-queue is active and the input is in AI mode with an empty buffer, the ghost text shows the existing queue hint copy ("Queue a follow up for the running agent", with the classic-input "or backspace to exit" variant), replacing the steer hint shown today during an LRC.

### Reverting and manual override

11. Auto-queue is a derived state, not a sticky toggle: when the LRC ends (command finishes, or control transfers to the user for any reason), the conversation's queue mode reverts to whatever it was before the LRC — the user's per-conversation toggle state, or the default from the queue-vs-interrupt setting. Already-queued rows remain queued.
12. If the user manually toggles queue mode off (chip click or its keybinding) while the agent still controls the LRC, the override is respected for the remainder of that LRC: prompts submit immediately to the agent, as today. The override is scoped to that LRC only — it does not change the conversation's persistent toggle state, and the next agent-controlled LRC in the conversation auto-enables again.
13. Toggling queue mode back on after such an override re-enables queuing for the remainder of the LRC; reverting at LRC end still applies per (11).
14. If the conversation was already in queue mode before the LRC (via the default-mode setting or a per-conversation toggle), entering and exiting the LRC produces no visible change: queue mode stays on throughout and after.

### Setting

15. A new setting, "Auto-queue prompts during long-running commands", controls invariants (1)–(14). It is a boolean toggle, default ON, cloud-synced, and visible on the AI settings page directly below the "Default prompt submission mode" (queue vs. interrupt) dropdown. The settings row includes an info tooltip explaining the behavior in more depth — e.g. "While an agent is driving a long-running command, submitted prompts are queued instead of sent to the agent immediately. Press Enter on an empty input to send the next queued prompt."
16. When the setting is OFF, behavior during agent-controlled LRCs is unchanged from today: prompts submit immediately to the agent, and the chip/ghost text reflect only the user's own queue toggle state.
17. The setting is also toggleable from the Command Palette via enable/disable entries.
18. Changing the setting takes effect immediately, including mid-LRC: turning it off while auto-queue is active reverts the conversation to its non-LRC queue state; turning it on while an agent controls an LRC activates auto-queue (subject to any manual override per (12)).

### Edge cases

19. If multiple exchanges occur within one conversation, each agent-controlled LRC independently triggers auto-queue on entry and reverts on exit; manual overrides per (12) never outlive the LRC they were made in.
20. Read-only shared-session viewers and other states where prompt sending is unavailable keep their existing restrictions; auto-queue does not create new send affordances there.
21. Auto-queue never queues an empty submission; Enter on an empty input follows (6) when rows are queued, and otherwise keeps its existing behavior.
