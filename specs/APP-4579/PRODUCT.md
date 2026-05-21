# APP-4579 — Local-to-cloud handoff for orchestrated agents

Linear: [APP-4579](https://linear.app/warpdotdev/issue/APP-4579/support-handoff-for-orchestrators-and-orchestrated-agents-or-make-it)

## Summary
Today, local-to-cloud handoff is unconditionally disabled for any conversation that is part of an orchestration tree — either a parent that has spawned children, or a child spawned by a parent. Typing `&` does nothing, the `/handoff` slash command is hidden, the footer chip is hidden, and the workspace action shows a toast that reads "Cloud handoff isn't available for orchestrated agent conversations." This is confusing because there is no technical blocker: the existing handoff plumbing already forks the conversation, uploads a workspace snapshot, and spawns a fresh cloud agent that rehydrates from that snapshot. The only thing missing is making the cloud agent aware that its prior orchestration relationships do not survive the handoff.

This spec enables local-to-cloud handoff for orchestrated agents. When the handoff happens, the cloud agent is given a single, hidden first-turn system message that tells it that the parent and/or children it had at handoff time are no longer reachable, so it can stop expecting messages from them and stop trying to message them.

## Behavior
### Surfaces become available for orchestrated conversations
1. In a local conversation that has a parent agent, has at least one child agent, or both, the user can initiate local-to-cloud handoff via every existing entry point:
   - typing `&` as the first character of agent input
   - the `/handoff` slash command in the slash command menu
   - the "Handoff to cloud" footer chip in agent view
   - the workspace action (`WorkspaceAction::OpenLocalToCloudHandoffPane`) dispatched by URI handlers and auto-handoff (macOS sleep, etc.)
2. The "Cloud handoff isn't available for orchestrated agent conversations" toast in `app/src/workspace/view.rs:13830` is removed. Any path that previously short-circuited on orchestration now proceeds through the standard handoff flow.
3. The non-orchestration gating (cloud handoff disabled by user/org setting, AI disabled, cloud conversation storage off, feature flag off, etc.) is unchanged.

### Cloud agent is told it lost its orchestration context, on the first turn only
4. When a handoff happens from a conversation that was part of an orchestration tree, the cloud agent's first LLM turn includes a hidden system message describing the orchestration context that did not transfer. The message is purely generic — it names neither specific run ids nor specific agent names — and is exactly one of the following forms, chosen by which relationships the source conversation had:
   - **Source had a parent (was a child):** "You have been handed off from a local environment to this cloud environment. The parent agent that started you, and any sibling agents under that parent, remain in the local environment and cannot be reached from here. Do not attempt to send them messages, wait for their messages or events, or otherwise coordinate with them. Operate independently from this point forward. Any new agents you start in this environment are reachable normally; this notice refers only to the orchestration relationships that existed at the time of handoff."
   - **Source had children (was a parent):** "You have been handed off from a local environment to this cloud environment. The child agents you previously started remain in the local environment and cannot be reached from here. Do not attempt to send them messages, wait for their messages or events, or otherwise coordinate with them. Any new agents you start in this environment are reachable normally; this notice refers only to the orchestration relationships that existed at the time of handoff."
   - **Source had both (was a parent that also had a parent):** A combined message naming both severed relationships in the same hidden first-turn system message.
5. This hidden message is delivered as part of the cloud agent's first turn after handoff. It is rendered as a `<system-message>`-wrapped system query, the same mechanism the snapshot-rehydration preamble uses, and is invisible in the user-facing transcript UI.
6. The message is included **only** on the cloud agent's first turn. On subsequent user follow-ups within the same cloud run, the message is not re-injected. The agent retains the message in its conversation context for those follow-ups (it is part of the transcript) but is not nagged with it again. This matches the existing snapshot-rehydration preamble behavior.
7. If the same cloud run later spawns new child agents from inside the cloud environment, the hidden first-turn message must not be re-applied to those new orchestration relationships. The wording in invariant 4 ("the orchestration relationships that existed at the time of handoff") covers this case.

### Invariant: orchestration and snapshot prompts compose cleanly
8. When the cloud agent's first turn would normally include the snapshot-rehydration preamble (from the existing local-to-cloud snapshot pipeline) **and** the orchestration handoff applies, both messages are delivered. They are independent injections that do not suppress each other.
9. When only one applies, only that one is delivered. Concretely:
   - orchestration handoff applies, no snapshot files were uploaded → only the orchestration message
   - snapshot files were uploaded, source was not orchestrated → only the snapshot-rehydration preamble (existing behavior)
   - both apply → both messages, snapshot rehydration first (so it is positioned next to the patches it talks about), orchestration message immediately after
10. When neither applies (e.g. a non-orchestration handoff with no touched workspace) → no hidden first-turn messages, same as today.

### Cloud-to-cloud handoff is unaffected
11. The cloud-to-cloud handoff path (e.g. the "Continue" tombstone flow gated by `HandoffCloudCloud`, and any cloud-to-cloud retry) does not inject the orchestration handoff message. A cloud agent that hands off to a fresh cloud sandbox keeps its server-side orchestration relationships intact, so the relationships are not "severed."

### Edge cases
12. A local conversation that is part of orchestration but has no `server_conversation_token` yet (conversation has not synced to the cloud) is still blocked from handoff — same as it is today for non-orchestration conversations — because the server-side fork requires a synced source. The existing inline error toast ("Your conversation hasn't synced to the cloud yet…") covers this case unchanged.
13. A local conversation that has an active long-running command is still blocked from handoff — same as today — and the existing "Can't hand off while a command is running" toast is shown unchanged. Orchestration status does not change this.
14. The local conversation that is handed off is cancelled at the end of the existing handoff flow, regardless of orchestration. Local sibling agents and local parents continue running until they finish or are cancelled by the user. We do not cancel them as part of the handoff — the user remains in control of their local runs.
15. Auto-handoff (macOS sleep, URI-triggered handoff) is now eligible for orchestration conversations too. Today it skips orchestration conversations via the same gate; once the gate is removed it will route orchestration conversations through the same flow as user-initiated handoff. The hidden first-turn orchestration message is delivered identically.
16. The setting "Cloud handoff" (REMOTE-1573) still gates this entire flow. If the user has cloud handoff disabled or it is force-disabled by cloud-conversations-off, orchestrated handoff is not available — same as non-orchestrated handoff.

## Success criteria
- A user in a local orchestrated conversation (parent or child) can hand off to cloud through every existing entry point with no warning toast or silent failure attributable to orchestration.
- The handed-off cloud agent's first LLM turn includes the appropriate hidden orchestration message, and that message is not re-injected on subsequent turns of the same cloud run.
- The cloud agent does not attempt to send messages to, or wait for events from, the parent/children it had at handoff time — verified by manual dogfood with a small orchestration tree and by spot-checking the cloud agent's tool calls.
- A handoff that triggers both snapshot rehydration and orchestration handoff delivers both hidden first-turn messages, in that order.
- Non-orchestration handoff behavior is unchanged.
