# Increment 3: Agent Management and details migration
## Context
Agent Management currently calls `get_tasks_and_conversations` and then maps each `ConversationOrTask` into card state, artifacts, action buttons, copy links, and details-panel data in `app/src/ai/agent_management/view.rs (938-1357)`. Cards use `get_open_action(Some(NewTab), app)` for clickability in `app/src/ai/agent_management/view.rs (1710-1755)` and dispatch it from `AgentManagementViewAction::OpenSession` in `app/src/ai/agent_management/view.rs (2324-2379)`.
This surface also owns filtering by status, source, environment, harness, creator, created date, and artifacts. Those filters should move to normalized entries so they are based on the same merged identity/display data that drives navigation.
## Proposed changes
Replace the management list’s `ManagementCardItemId` task/conversation split with `AgentConversationEntryId`. Card construction should use `AgentConversationEntry` fields directly:
- title from `display.title`;
- status from `display.status`;
- source from `display.source`;
- environment from `display.environment_id`;
- harness from `display.harness`;
- artifacts from `display.artifacts`;
- creator from `display.creator`;
- request usage and runtime from `display`.
Move filtering to the entry builder or add an entry-specific filter function. Prefer one filtering path that can be shared by the conversation list and Agent Management, with owner-specific behavior parameterized by `AgentManagementFilters`.
Replace card click and details-panel open actions with `resolve_open_action(Entry(entry.id), Some(RestoreConversationLayout::NewTab), ctx)`.
Replace copy-link derivation with a sibling resolver:
```rust
pub fn resolve_copy_link(
    subject: AgentConversationNavigationSubject,
    app: &AppContext,
) -> Option<String>
```
This should share identity resolution with `resolve_open_action` and avoid a separate `link_preference()` policy. Link policy can prefer a live session URL for active executions and otherwise use the server conversation token when available.
Update details panel construction so both task-backed and conversation-backed entries go through one normalized path. Keep raw task-only fields where they are truly run-specific, but they should come from attached run identity rather than from a separate card variant.
## Testing and validation
Add unit tests for entry filtering:
- stale raw `InProgress` task with terminal matching conversation filters into Done/Failed according to `AgentRunDisplayStatus`;
- environment filter includes local/cloud conversation entries as “None” and filters task environments correctly;
- harness filter handles task harness, local Oz conversation, and metadata-only cloud conversation;
- artifact filter works for artifacts sourced from task, loaded conversation, and metadata.
Add tests for copy-link resolver:
- active joinable session returns session link;
- non-active cloud-backed entry returns conversation link;
- local-only unsynced conversation returns no link;
- task with no token but attached local synced conversation returns conversation link.
Manual validation:
- Agent Management card click and details “Open” use the same destination as the conversation list for the same logical run;
- copy-link button and card click no longer disagree for completed cloud runs;
- details panel displays one coherent entry when task and local conversation both exist.
## Risks and mitigations
### Details panel field regressions
Some fields are genuinely task/run-specific. Keep `AgentConversationIdentity.ambient_run_id` available and fetch raw task data only for fields not represented in `display`.
### Filter behavior changes
Normalized filtering may intentionally move stale task rows between status buckets. Preserve current behavior only where it matches the derived display status policy.
