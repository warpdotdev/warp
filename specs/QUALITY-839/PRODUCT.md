# Auto-queue prompts during agent-requested long-running commands

Linear: [QUALITY-839](https://linear.app/warpdotdev/issue/QUALITY-839/auto-enable-prompt-queueing-during-lrc)

## Summary

While an agent is in control of a long-running command (LRC) that the agent requested as part of a conversation, submitting a prompt auto-queues it instead of immediately sending it to the agent driving the command when regular queue mode is otherwise off. LRC-auto-queued prompts are delivered to the agent when the command finishes only when doing so preserves existing queue order; prompts queued by regular queue mode keep normal end-of-response semantics. The user can also press Enter on an empty input to fire the next queued prompt earlier. A new cloud-synced dropdown setting, "Default long-running command submission mode", controls whether prompts are queued or sent immediately during eligible LRCs. It only applies — and is only shown — when the default prompt submission mode is Interrupt.

Figma: none provided.

## Problem

Today, a prompt submitted while an agent controls an agent-requested LRC is delivered to that agent immediately, steering it mid-command. Users often type thoughts ahead of time and don't want them injected into the running command the instant they hit Enter; they want them held until they deliberately release them or until the exchange finishes.

## Behavior

### Trigger and scope

1. Auto-queue activates for a conversation exactly when the agent holds control of an active long-running command that the agent requested in that conversation and the settings call for it: the default prompt submission mode is Interrupt and the LRC submission mode is "Queue until command finishes" (see 18). This includes the state where the agent is blocked on user approval to interact with the command.
2. Auto-queue does not activate for user-started LRCs where the user explicitly tagged in the agent, or when the user is in control of the LRC — e.g. before the agent has taken control, or after a manual takeover, stop, or agent-initiated transfer of control back to the user.
3. Auto-queue activation is per-conversation: it affects only the conversation whose agent controls the LRC. Other conversations' queue toggle states are untouched.
4. The behavior is gated on the same feature availability as the existing prompt-queue feature (the queue chip / `/queue` surface). Where the queue feature is unavailable, behavior is unchanged from today.
5. When the default prompt submission mode is Queue, the LRC machinery is entirely inert: prompts queue until the end of the full response per existing queue-mode behavior, the chip toggle behaves persistently, and the LRC setting is hidden (see 19).

### Queuing while the LRC runs

6. While auto-queue is active and regular queue mode is otherwise off, submitting a non-empty prompt appends it to the conversation's queued prompts (the same queue used by the auto-queue chip and `/queue` today) instead of sending it to the agent, and the input clears. If the current queue head is absent or is itself queued until command finish, the queued prompts panel shows the new row with an italic, secondary-colored "(queued until the command finishes)" suffix after its preview text — the same treatment as the model picker's "(selected)" label. If the current queue head is not queued until command finish, the prompt appends as a regular queued row with no command-finish suffix.
7. If regular queue mode is already enabled for the conversation (via the queue chip/keybinding or the default prompt submission mode), submissions during the LRC use regular queue semantics: they append as normal queued rows with no command-finish suffix and drain at the end of the response unless the user sends them manually.
8. Pressing Enter on an empty input sends the top queued row immediately — delivered to the same target an immediate submission would have used (the agent controlling the LRC) — per the existing empty-input-Enter send-now behavior. Each press sends exactly one row.
9. All existing queue interactions (panel rows, edit, delete, reorder, send-now buttons, pause on error/cancel) behave exactly as they do for manually-enabled queue mode.
10. When the command finishes, leading prompts that were auto-queued during it (the suffixed rows at the head of the queue) are sent to the agent immediately, in queue order — including when the user manually took over the command before it finished. Rows queued by other means (`/queue`, an explicit queue-mode toggle, queue default mode) are untouched and drain per the existing end-of-response rules; command-finish delivery never skips over them.
11. Shell-command rows queued while the agent controls the LRC are regular queued commands (no suffix): they cannot be delivered to the agent, do not fire at command end, and keep the existing queued-command drain semantics.

### Status chip and ghost text

12. While auto-queue is active, the prompt-queue chip in the warping indicator renders in its active (accent-colored) state, identical to when the user enables queue mode manually.
13. While auto-queue is active and the input is in AI mode with an empty buffer, the ghost text shows the existing queue hint copy ("Queue a follow up for the running agent", with the classic-input "or backspace to exit" variant), replacing the steer hint shown today during an LRC.

### Reverting and manual override

14. Auto-queue is a derived state, not a sticky toggle: when the LRC ends (command finishes, or control transfers to the user for any reason), the conversation's queue mode reverts to whatever it was before the LRC — the user's per-conversation toggle state, or the default from the queue-vs-interrupt setting. Rows that did not fire per (10) remain queued.
15. If the user manually toggles queue mode off (chip click or its keybinding) while the agent still controls the LRC, the override is respected for the remainder of that LRC: prompts submit immediately to the agent, as today. The override is scoped to that LRC only — it does not change the conversation's persistent toggle state, and the next eligible agent-requested LRC in the conversation auto-enables again.
16. Toggling queue mode back on after such an override re-enables regular queue mode for the conversation; prompts submitted after that toggle use normal queued-row semantics rather than command-finish LRC semantics. Reverting at LRC end still applies per (14).
17. If the conversation was already in queue mode before the LRC (via a per-conversation toggle), entering and exiting the LRC produces no visible change: queue mode stays on throughout and after, and its rows drain at end of response per (10).

### Setting

18. A new setting, "Default long-running command submission mode", controls invariants (1)–(17). It is a dropdown with two options — "Send immediately" and "Queue until command finishes" (the default) — cloud-synced, and visible on the AI settings page directly below the "Default prompt submission mode" (queue vs. interrupt) dropdown. Its description reads: "What happens when you submit a prompt while an agent is driving a long-running command. LRC-queued prompts are sent to the agent when the command finishes."
19. The dropdown is only rendered while "Default prompt submission mode" is Interrupt. With Queue selected it is hidden (and ignored), since prompts already queue until the end of the full response.
20. When set to "Send immediately", behavior during eligible agent-requested LRCs is unchanged from today: prompts submit immediately to the agent, and the chip/ghost text reflect only the user's own queue toggle state.
21. The setting is also settable from the Command Palette via "Set long-running command submission: …" entries, shown only while the default prompt submission mode is Interrupt.
22. Changing the setting takes effect immediately, including mid-LRC: switching to "Send immediately" while auto-queue is active reverts the conversation to its non-LRC queue state; switching to "Queue until command finishes" while an agent controls an eligible agent-requested LRC activates auto-queue (subject to any manual override per (15)).

### Edge cases

23. If multiple exchanges occur within one conversation, each eligible agent-requested LRC independently triggers auto-queue on entry and reverts on exit; manual overrides per (15) never outlive the LRC they were made in.
24. Read-only shared-session viewers and other states where prompt sending is unavailable keep their existing restrictions; auto-queue does not create new send affordances there.
25. Auto-queue never queues an empty submission; Enter on an empty input follows (8) when rows are queued, and otherwise keeps its existing behavior.
