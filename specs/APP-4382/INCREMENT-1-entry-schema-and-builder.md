# Increment 1: entry schema and merge builder
## Context
This increment adds the normalized projection API behind existing UI code. `AgentConversationsModel` already stores tasks and conversations separately in `app/src/ai/agent_conversations_model.rs (844-1901)`, and `ConversationOrTask` currently provides display helpers over borrowed raw data in `app/src/ai/agent_conversations_model.rs (399-813)`. The migration should add the new API without removing or migrating those call sites yet.
The builder must understand metadata-only conversations. `AIConversationMetadata` can represent local persisted conversations or cloud-only metadata in `app/src/ai/blocklist/history_model.rs (49-157)`. `merge_cloud_conversation_metadata` creates a local `AIConversationId` for new cloud-only metadata and records the server token in `server_token_to_conversation_id` in `app/src/ai/blocklist/history_model/conversation_loader.rs (334-453)`.
## Proposed changes
Add a module near `agent_conversations_model.rs`, for example `app/src/ai/agent_conversations_model/entry.rs` if splitting the file is practical, or keep the first pass in `agent_conversations_model.rs` to reduce churn. Define:
- `AgentConversationEntry`
- `AgentConversationEntryId`
- `AgentConversationIdentity`
- `AgentConversationDisplayData`
- `AgentConversationProvenance`
- `AgentConversationBackingData`
- `AgentConversationCapabilities`
Add an internal builder that consumes current model state and `AppContext`. The builder should:
1. build a map from server token to local conversation id from history metadata and loaded conversations;
2. build a map from run id to local conversation id using `conversation_id_for_agent_id`;
3. create one entry for each `AmbientAgentTask`, keyed by `AgentConversationEntryId::AmbientRun(task.run_id())`;
4. attach a matching local conversation id by run id first, then server token;
5. attach server token from the task, matching conversation metadata, or loaded conversation;
6. create `Conversation` entries for metadata/local conversations not already attached to an ambient run.
The display-data derivation should reuse existing `ConversationOrTask` behavior where possible during this increment, but the new type should not expose `ConversationOrTask` publicly. It is acceptable for the builder to use temporary private helpers to avoid duplicating all display logic in the first PR.
Capabilities should be conservative in this increment. `can_open` should mean the entry has at least one of: open ambient run id, local conversation id, or server token. `can_share`, `can_delete`, and `can_fork_locally` can mirror current behavior but should remain data booleans, not actions.
Add a new read API:
```rust
pub fn get_entries(
    &self,
    filters: &AgentManagementFilters,
    app: &AppContext,
) -> Vec<AgentConversationEntry>
```
Returning a `Vec` is acceptable for the first pass because entries are owned snapshots and list sizes are small. Keep the existing `get_tasks_and_conversations` API for current UI.
## Testing and validation
Add unit tests in `app/src/ai/agent_conversations_model_tests.rs` for builder identity and dedupe behavior:
- task-only entry uses `AmbientRun` id and `AmbientRun` provenance;
- local-only entry uses `Conversation` id and `LocalInteractive` provenance;
- cloud metadata-only entry uses `Conversation` id and `CloudSyncedConversation` provenance with `has_cloud_data = true` and `has_loaded_conversation = false`;
- task plus local conversation matched by run id produces one `AmbientRun` entry with both ids attached;
- task plus local conversation matched by server token produces one `AmbientRun` entry with both ids attached;
- unrelated task and conversation produce two entries;
- child-agent cloud metadata skipped today by metadata merge remains skipped from entries.
Run the focused test module after implementation. This increment should not change rendered UI, so existing conversation list and management view behavior should remain unchanged.
## Risks and mitigations
### Duplicates from incomplete indexing
Build token and run-id indices before constructing entries. Keep the “ambient run owns row identity” rule centralized in one builder function.
### Too much display duplication
It is fine to temporarily delegate to private helpers based on `ConversationOrTask` as long as the public API is normalized. Increment 4 should remove the old helper path.
