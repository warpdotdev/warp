# Cloud agent tombstone and followup input behavior — Product Spec
Linear: APP-4483. Figma: none.
## Summary
When `FeatureFlag::HandoffCloudCloud` is enabled, make the idle-state UI for cloud agent conversations consistent and permission-aware. The decision to show the conversation-ended tombstone, the inline followup input, and any continue CTA should depend on two product concepts:
- Which harness produced the conversation: Oz vs. a third-party harness such as Claude Code.
- Whether the current user has edit access to the underlying AI conversation object.
Oz conversations should feel directly resumable when the user can edit the conversation. Third-party harness conversations should always preserve the terminal transcript boundary with a tombstone when no execution is active, because their continuation flow is different and they cannot be forked into a local Oz conversation.
## Problem
Today the tombstone/input behavior is scattered across session-sharing end handling, ambient task end handling, tombstone CTA rendering, and followup input state. The primary gate is currently whether the current user appears to own the ambient task. That is not the correct product model for shared cloud conversations: access should follow the underlying conversation permissions, not just task creator identity.
This leads to inconsistent outcomes when `FeatureFlag::HandoffCloudCloud` is enabled:
- A user with edit access to a shared Oz conversation may not get the inline followup input if they are not the task creator.
- A user who can only view an Oz conversation can see continuation affordances that imply they can mutate the original conversation.
- Third-party harness conversations can be treated like Oz conversations even though local forking is unsupported for those harnesses.
## Goals
- Use edit access to the underlying AI conversation as the product source of truth for whether the current user may continue the original cloud conversation.
- For Oz conversations:
  - Hide the tombstone and show the followup input when the current user can edit the underlying conversation.
  - Show the tombstone and the existing local continuation CTA when the current user can only view the underlying conversation.
- For third-party harness conversations:
  - Always show the tombstone when there is no active execution.
  - Show a cloud continuation CTA only when the current user can edit the underlying conversation.
  - Never show a fork-local CTA.
- Keep active executions unchanged: while the cloud execution is live, the user should see the active shared/cloud session rather than an ended-state tombstone.
## Non-goals
- Implement fork-and-continue for third-party harness conversations.
- Change tombstone metadata, credits, artifacts, runtime, or error presentation.
- Change server-side authorization rules. The client should reflect permissions, but the server remains authoritative for followup submission.
- Redesign the tombstone UI layout beyond which CTA is present.
- Change behavior when `FeatureFlag::HandoffCloudCloud` is disabled.
## Product concepts
### Conversation edit access
“Edit access” means the current user has at least edit-level access to the underlying AI conversation object. This should be derived from the conversation’s server permissions, not from ambient task ownership.
If the client has not yet loaded permissions, it should avoid presenting mutation affordances that might be wrong. The safe default is to treat access as view-only until edit access is known.
### Harness
Harness should come from the conversation/task metadata already used to identify Oz vs. non-Oz runs. `Oz` is the only harness eligible for local fork continuation. Any other known harness, including Claude Code, Codex, Gemini, or future third-party harnesses, should follow the third-party behavior.
If the harness is unknown while metadata is still loading, the safe default is to show the tombstone and hide mutation CTAs until the harness and permissions are known.
### Active execution
This spec applies when the cloud agent conversation has no active execution. If an execution is currently active, the live session UI remains the source of truth and the ended-state tombstone should not be inserted.
## Behavior
### Oz harness
If the conversation was produced by Oz and there is no active execution:
- User has edit access:
  - Tombstone: hidden.
  - Followup input: shown and editable.
  - CTA: none, because the input is already available.
  - Submitting the input continues the same cloud conversation.
- User has view access only:
  - Tombstone: shown.
  - Followup input: hidden or non-editable.
  - CTA: show `Continue locally`.
  - Clicking the CTA forks the cloud conversation into a local Warp conversation and continues locally, without mutating the original shared conversation.
### Third-party harness
If the conversation was produced by a third-party harness and there is no active execution:
- User has edit access:
  - Tombstone: shown.
  - Followup input: hidden until the user explicitly chooses to continue.
  - CTA: show `Continue`.
  - Clicking `Continue` starts the third-party cloud followup flow for the same conversation/run.
- User has view access only:
  - Tombstone: shown.
  - Followup input: hidden or non-editable.
  - CTA: none.
  - The user can inspect the transcript but cannot continue or fork it from this UI.
### Behavior invariants
- B1: Oz + edit access + no active execution: no tombstone, show followup input.
- B2: Oz + view-only access + no active execution: show tombstone with `Continue locally`.
- B3: Third-party + edit access + no active execution: show tombstone with `Continue`.
- B4: Third-party + view-only access + no active execution: show tombstone with no continue CTA.
- B5: Any harness + active execution: no ended-state tombstone; keep live execution UI.
- B6: Unknown harness or unknown access + no active execution: show tombstone with no mutation CTA.
### UI state table
| Harness | Conversation access | Execution state | Tombstone | Followup input | CTA | Result |
| --- | --- | --- | --- | --- | --- | --- |
| Oz | Edit | Active execution | Hidden | Hidden while execution is active | None | User watches or interacts with the live cloud execution UI. |
| Oz | View only | Active execution | Hidden | Hidden while execution is active | None | User watches the live cloud execution UI without ended-state affordances. |
| Third-party | Edit | Active execution | Hidden | Hidden while execution is active | None | User watches or interacts with the live third-party cloud execution UI. |
| Third-party | View only | Active execution | Hidden | Hidden while execution is active | None | User watches the live third-party cloud execution UI without ended-state affordances. |
| Oz | Edit | No active execution | Hidden | Shown and editable | None | User submits a followup that continues the same cloud conversation. |
| Oz | View only | No active execution | Shown | Hidden or non-editable | `Continue locally` | User forks into a local Warp conversation before continuing. |
| Third-party | Edit | No active execution | Shown | Hidden until continuation starts | `Continue` | User continues via the existing third-party cloud followup execution flow. |
| Third-party | View only | No active execution | Shown | Hidden or non-editable | None | User can inspect the transcript but cannot continue from this UI. |
| Unknown | Any or unknown | No active execution | Shown | Hidden or non-editable | None | Client waits for metadata before showing mutation affordances. |
## Copy
Preferred CTA copy:
- Oz, view-only: `Continue locally`
- Third-party, edit access: `Continue`
Tooltips can clarify the distinction:
- Oz local continuation CTA: “Fork this conversation into a local Warp session.”
- Third-party continue CTA: “Continue this cloud conversation.”
Avoid showing “Continue locally” for third-party harnesses, because local continuation is unsupported and misleading.
## Edge cases
- Metadata not loaded: show the tombstone and hide continue CTAs until both harness and editability are known.
- Permissions change while viewing: recompute the tombstone/input state from the latest conversation permissions. If edit access is revoked, hide the followup input and remove any continue CTA that would mutate the original conversation.
- Conversation has task metadata but no conversation metadata: show the tombstone and hide mutation CTAs until conversation metadata is available, unless there is an existing server-confirmed editability signal.
- Followup submission fails due to server-side permission denial: keep the user in the ended-state UI, restore their draft if possible, and show a concise error toast.
- Third-party harness later gains local fork support: this spec should be revisited; until then, third-party harnesses must not show fork-local affordances.
## Success criteria
- An Oz cloud conversation where the current user has edit access ends with an editable followup input and no tombstone.
- An Oz cloud conversation where the current user only has view access ends with a tombstone containing `Continue locally`, and no inline followup input.
- A Claude Code cloud conversation where the current user has edit access ends with a tombstone containing `Continue`, and no fork-local CTA.
- A Claude Code cloud conversation where the current user only has view access ends with a tombstone and no continue CTA.
- Ambient task creator identity is no longer the product source of truth for showing the followup input; conversation edit access is.
- Active executions do not show the ended-state tombstone regardless of harness or permissions.
- Existing behavior remains unchanged when `FeatureFlag::HandoffCloudCloud` is disabled.
## Resolved decisions
- Keep the existing `Continue locally` copy for the Oz view-only local continuation CTA.
- Third-party `Continue` uses the existing third-party followup execution flow. This spec should not introduce a new modal, inline input mode, or alternate continuation concept.
- For view-only third-party conversations, the absence of a CTA is sufficient; no explanatory subtitle is required.
