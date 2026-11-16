# /continue-locally slash command — Product Spec

Linear: REMOTE-1515. Figma: none provided.

## Summary
Add a `/continue-locally` slash command that forks the active cloud Oz agent conversation into a local Warp conversation from the input. It is a parity entrypoint for the existing "Continue locally" affordances on the conversation-ended tombstone (`app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs:485-491`) and on the conversation details panel (`app/src/ai/conversation_details_panel.rs:512-520`).

## Behavior

1. The command is registered with name `/continue-locally`, description "Continue this cloud conversation locally", and an optional argument with hint text `<optional prompt to send in forked conversation>`. It uses the same `bundled/svg/arrow-split.svg` icon as the rest of the fork family.

2. The command appears in the slash-command menu (and is reachable via keybinding) only when the active conversation is associated with a cloud Oz `AmbientAgentTask`. For local conversations and for non-Oz cloud runs (Claude, Gemini), the command is hidden, matching the harness gate already used by the existing button entrypoints.

3. Availability is independent of the cloud run's status: the command shows for in-progress, succeeded, blocked, errored, and cancelled cloud Oz runs. Hard prerequisites: AI is globally enabled and the input is in agent view (`AGENT_VIEW`). The command does not require `NO_LRC_CONTROL`, since long-running-command state is a property of the local terminal and is not relevant to a cloud run.

4. Selecting the command from the slash menu inserts `/continue-locally ` into the input. The user may optionally type a prompt; that prompt is sent as the first user query in the forked local conversation after the fork completes. An empty / whitespace-only argument is treated as no argument (no follow-up prompt is sent).

5. Pressing Enter forks into a split pane to the right of the current pane. Pressing Cmd+Enter (Ctrl+Enter on Linux/Windows) forks into a new tab. The agent input footer mirrors `/fork`'s tip: `Enter new pane` / `Cmd-Enter new tab`.

6. The forked conversation is a local Warp Oz conversation seeded from the cloud run's transcript via the same pipeline as the existing button — same fork prefix, same destination handling. The source cloud run is never modified; the cloud agent continues running (or stays as it was) and the user's local Warp client gets a new local conversation forked from it.

7. On success, the command shows a dismissible toast `Forked "<source conversation title>"` (titles longer than `MAX_FORK_TOAST_TITLE_LENGTH` are truncated with an ellipsis). This is the same toast as `/fork` — produced by `Workspace::show_fork_toast` — so the user gets a consistent acknowledgement that the cloud-to-local handoff completed regardless of which entrypoint they used.

8. If the source conversation can't be loaded or the fork itself fails, the command surfaces the same dismissible error toasts as the existing button entrypoints ("Failed to load conversation for forking." or "Conversation forking failed."). If the active conversation is somehow not a cloud Oz run at execute time (e.g. via stale keybinding), the command shows an error toast and does not dispatch a fork.

9. Telemetry: a successful invocation emits `AgentManagementTelemetryEvent::ConversationForked` (matching `/fork`) plus a new `AgentManagementTelemetryEvent::SlashCommandContinueLocally` event so cloud-to-local handoffs from the slash command can be measured separately from the tombstone and details-panel buttons. Existing `TombstoneContinueLocally` and `DetailsPanelContinueLocally` events are unchanged.

## Non-goals
- Surfacing this command in completed conversation transcript viewers, where input is hidden by design. The existing tombstone button remains the only entrypoint there.
- Adding a `/continue-locally` button to wasm. Like `ContinueConversationLocally` itself, the command is `cfg(not(target_family = "wasm"))`.
