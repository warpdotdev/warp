# Increment 4: cleanup and hardening
## Context
After conversation list and Agent Management migrate to `AgentConversationEntry`, `ConversationOrTask` should no longer be the public model consumed by list/navigation/details surfaces. The old `link_preference()` helper in `app/src/ai/agent_conversations_model.rs (624-660)` should stop being a source of truth for open and copy-link behavior.
This increment removes transitional APIs, hardens event invalidation, and adds regression coverage around the originally inconsistent cases.
## Proposed changes
Remove or narrow public access to:
- `ConversationOrTask`
- `ConversationOrTaskId` usage in list/navigation surfaces
- `ConversationOrTask::get_open_action`
- `ConversationOrTask::session_or_conversation_link`
- `ConversationOrTask::link_preference`
If some internal helpers remain useful, make them private to the entry builder and rename them so they cannot be mistaken for the public entry API.
Audit all `get_open_action`, `session_or_conversation_link`, and `get_session_status` call sites. Remaining navigation should go through `resolve_open_action`, and remaining copy-link behavior should go through `resolve_copy_link`.
Review event handling in `AgentConversationsModel`:
- task updates should invalidate/re-emit entry updates;
- conversation status updates should refresh derived status and capabilities;
- server token assignment should update merged identity;
- cloud metadata merge should update entries;
- active view open/close/focus should update active/open capabilities without requiring stale nav data.
If existing `AgentConversationsModelEvent` variants are too ambiguous, add a normalized event such as:
```rust
pub enum AgentConversationsModelEvent {
    EntriesChanged,
    EntryDisplayDataChanged { id: AgentConversationEntryId },
    EntryArtifactsChanged { id: AgentConversationEntryId },
}
```
Only do this if it reduces caller complexity; avoid event churn if all migrated consumers can simply rebuild from `get_entries`.
Update comments and docs in the model to describe the new ownership boundary: raw task/conversation caches are source data, while `AgentConversationEntry` is the UI/navigation projection.
## Testing and validation
Add regression tests named around the fixed behaviors:
- task shadows local conversation but missing task token still opens via local conversation;
- stale active execution no longer forces session open when no parseable session id exists;
- completed cloud run with token remains openable even without session link;
- copy-link and open resolver use consistent source priority;
- metadata-only cloud conversation can be opened by server token without a loaded `AIConversation`;
- server token assignment after an entry is first built updates identity/copy-link behavior.
Run all focused tests touched during increments 1-3 plus a targeted compile/check. Before review, run repository-required formatting and linting commands.
## Risks and mitigations
### Hidden call sites
Use grep for removed method names and old ID types. Keep this increment small and mechanical where possible.
### Event overengineering
Prefer simple rebuild-on-event behavior until performance requires finer invalidation. The entry list is small enough that correctness is more important than micro-optimizing derived state.
