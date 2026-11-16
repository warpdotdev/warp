# REMOTE-1573 — Settings to gate local-to-cloud handoff and snapshot upload

## Summary

Add user-facing settings to control local-to-cloud handoff, gate the `&` prefix entrypoint independently, and ensure snapshot uploads are disabled when cloud conversations are off.

Figma: none provided

## Behavior

### Setting: handoff enabled

1. A new boolean setting "Cloud handoff" appears in the AI settings page under the Cloud Agents section. It defaults to **enabled**.

2. When the setting is enabled, all local-to-cloud handoff surfaces are available: the `&` compose prefix, the `/handoff` slash command, and the "Handoff to cloud" footer chip in agent view.

3. When the setting is disabled, all three surfaces are hidden or inert:
   - Typing `&` as the first character in agent input does **not** enter cloud handoff compose mode.
   - The `/handoff` slash command is not shown in the slash command menu and returns `false` (no-op) if invoked programmatically.
   - The "Handoff to cloud" footer chip is not rendered.
   - `WorkspaceAction::OpenLocalToCloudHandoffPane` dispatches are no-ops.

4. The setting is persisted locally and synced to cloud via the existing `define_settings_group!` / Warp Drive settings infrastructure, following the pattern of other AI settings (e.g. `orchestration_enabled`). TOML path: `agents.warp_agent.other.cloud_handoff_enabled`.

### Setting: `&` prefix entrypoint enabled

5. A second boolean setting "Use & to trigger handoff" appears as a sub-setting beneath the handoff toggle. It defaults to **enabled**.

6. When this sub-setting is enabled and the parent handoff setting is also enabled, typing `&` as the first character in a local agent conversation's input activates cloud handoff compose mode (existing behavior).

7. When the sub-setting is disabled (but the parent is enabled), the `&` prefix no longer triggers handoff compose mode. The `/handoff` slash command and footer chip remain available — only the keyboard shortcut is suppressed.

8. When the parent handoff setting is disabled, the `&` sub-setting row is hidden entirely. Its stored value is preserved so re-enabling the parent restores the user's previous choice.

### Force-disabling handoff when prerequisites are missing

9. The handoff setting is force-disabled (toggle rendered checked-off and non-interactive) when **either** of these conditions is true:
   - The user's cloud conversation storage setting is off (user-level `is_cloud_conversation_storage_enabled == false`, or org-level `cloud_conversation_storage_settings == Disable`).
   - The user or org has AI disabled.

10. When force-disabled, the toggle shows a tooltip explaining why: "Cloud handoff requires cloud conversations to be enabled."

11. When force-disabled, the effective value of the setting is `false` regardless of the stored value. All handoff surfaces (invariants 3a–3d) are suppressed.

12. If the prerequisites become satisfied again (user re-enables cloud conversations), the toggle becomes interactive and the stored value takes effect again.

### Snapshot gating: local-to-cloud handoff

13. When the effective handoff setting is disabled (either by user choice or force-disabled), no local-to-cloud handoff flow runs, so `fork_conversation` and `upload_snapshot_for_handoff` are never called. This is a natural consequence of invariant 3, not a separate gate.

14. Independently of the handoff setting, if cloud conversation storage is disabled at the time a cloud agent is spawned (from any surface — cloud mode compose, handoff, etc.), the client sets `snapshot_disabled: true` on the `SpawnAgentRequest` so the cloud agent's end-of-run snapshot upload is also skipped.

15. The `snapshot_disabled` field is added to `SpawnAgentRequest` as an optional boolean. When `None` or absent, the server/agent uses its default behavior (snapshot enabled). When `Some(true)`, the cloud agent skips the end-of-run snapshot upload pipeline.

### Snapshot gating: all cloud agent spawns

16. The `snapshot_disabled` flag is set on **every** cloud agent spawn from the client (not just handoff spawns) when cloud conversation storage is disabled. This includes spawns from cloud mode compose, the agent management view, and any other client-initiated spawn path that goes through `AmbientAgentViewModel::spawn_agent` or `spawn_agent_with_request`.

17. Cloud-to-cloud follow-up submissions (`submit_cloud_followup`) are **not** gated — follow-ups are allowed regardless of cloud conversation storage, since the parent cloud run already has a conversation. However, the `snapshot_disabled` flag propagates to follow-up runs the same way it does for initial spawns.

### Interaction with cloud-to-cloud handoff

18. The handoff setting gates local-to-cloud handoff only. Cloud-to-cloud follow-ups (the "Continue" tombstone flow gated by `HandoffCloudCloud`) are unaffected by this setting.

### Edge cases

19. If the user disables cloud conversation storage while a local-to-cloud handoff is in progress (fork + snapshot upload already in flight), the in-flight operation completes. The setting change takes effect on the next handoff attempt.

20. The setting does not affect the SDK/CLI `oz` agent path.

21. Anonymous and logged-out users never see the handoff setting (AI settings are hidden when AI is disabled, and AI is disabled for anonymous/logged-out users).
