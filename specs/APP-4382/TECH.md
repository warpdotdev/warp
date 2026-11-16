# Agent conversation entry normalization migration
## Context
Linear: https://linear.app/warpdotdev/issue/APP-4382/normalize-agent-conversation-entries-for-list-and-navigation-surfaces
`AgentConversationsModel` currently exposes `ConversationOrTask`, a wrapper over either `AmbientAgentTask` or `ConversationMetadata`, to list, details, filtering, and navigation surfaces in `app/src/ai/agent_conversations_model.rs (399-813)`. The wrapper centralizes several display helpers, but still encodes important behavior as “task vs conversation” decisions. The most problematic example is `link_preference()` and `get_open_action()` in `app/src/ai/agent_conversations_model.rs (624-813)`: task rows choose between session, conversation transcript, or no action from cached task fields, while local conversation rows restore/navigate from `ConversationNavigationData`.
The model also intentionally hides local conversation rows when a task appears to represent the same logical run in `app/src/ai/agent_conversations_model.rs (1382-1497)`. That shadowing is useful for preserving cloud-run affordances, but it means the visible task row can discard better local-conversation navigation data. A task row with a stale or incomplete `session_id`, `session_link`, `conversation_id`, or `is_sandbox_running` can therefore be unopenable even when the hidden conversation representation is restorable.
The underlying sources are already separate. `BlocklistAIHistoryModel` owns loaded `AIConversation` contents in `conversations_by_id` and lightweight historical/cloud metadata in `all_conversations_metadata` in `app/src/ai/blocklist/history_model.rs (49-214)`. Cloud metadata can exist without a loaded conversation via `AIConversationMetadata` and `merge_cloud_conversation_metadata` in `app/src/ai/blocklist/history_model/conversation_loader.rs (334-453)`, while `load_conversation_data` loads content only on demand in `app/src/ai/blocklist/history_model/conversation_loader.rs (145-236)`. `ActiveAgentViewsModel` separately tracks which local conversations and ambient sessions are currently open in `app/src/ai/active_agent_views_model.rs (45-531)`.
This migration introduces a projection-layer `AgentConversationEntry` for list/navigation/detail surfaces. It should not replace `AIConversation`, `BlocklistAIHistoryModel`, or transcript content APIs. Its job is to merge provenance and lightweight display data into one logical row so UI surfaces no longer choose between task and conversation representations.
## Proposed changes
### Target model boundary
Add a normalized entry API to `AgentConversationsModel` while keeping raw task and conversation storage as implementation details:
```rust
pub struct AgentConversationEntry {
    pub id: AgentConversationEntryId,
    pub identity: AgentConversationIdentity,
    pub display: AgentConversationDisplayData,
    pub provenance: AgentConversationProvenance,
    pub backing: AgentConversationBackingData,
    pub capabilities: AgentConversationCapabilities,
}
```
`AgentConversationEntry` is a projection. It carries owned IDs and display snapshots, not borrowed references to `AmbientAgentTask` or `AIConversation`. Callers that need transcript contents still go through `BlocklistAIHistoryModel` and loader APIs.
Use local/client identities for listed entries:
```rust
pub enum AgentConversationEntryId {
    AmbientRun(AmbientAgentTaskId),
    Conversation(AIConversationId),
}
```
Do not use `ServerConversationToken` as a normal list identity. Server tokens are global fetch/navigation handles; cloud-only metadata rows already receive a local `AIConversationId` when metadata is merged.
The entry identity should preserve every known reference:
```rust
pub struct AgentConversationIdentity {
    pub local_conversation_id: Option<AIConversationId>,
    pub ambient_run_id: Option<AmbientAgentTaskId>,
    pub server_conversation_token: Option<ServerConversationToken>,
    pub parent_conversation_id: Option<AIConversationId>,
    pub parent_run_id: Option<AmbientAgentTaskId>,
}
```
Keep provenance semantic and move loadability into backing:
```rust
pub enum AgentConversationProvenance {
    LocalInteractive,
    AmbientRun,
    CloudSyncedConversation,
}

pub struct AgentConversationBackingData {
    pub has_loaded_conversation: bool,
    pub has_local_persisted_data: bool,
    pub has_cloud_data: bool,
    pub has_ambient_run: bool,
}
```
`AgentConversationDisplayData` should centralize list/detail fields currently read from `ConversationOrTask`: title, initial query, created/updated timestamps, `AgentRunDisplayStatus`, creator, working directory, source, environment, harness, request usage, run time, and artifacts. The display status should continue to use the derived `AgentRunDisplayStatus` algorithm instead of raw `AmbientAgentTaskState` or raw `ConversationStatus`.
`AgentConversationCapabilities` should expose eligibility booleans for UI affordances such as open, copy link, share, delete, fork locally, continue locally, and cancel. It must not store the final `WorkspaceAction`. Open actions should be resolved dynamically from entry identity at click time.
### Increment sequence
Increment 1, `INCREMENT-1-entry-schema-and-builder.md`, adds the entry schema, merge builder, and tests without migrating UI. It introduces `AgentConversationsModel::get_entries(...)` or equivalent behind existing code paths.
Increment 2, `INCREMENT-2-navigation-and-conversation-list.md`, adds a dynamic navigation resolver and migrates the conversation list to normalized entries. This is the first user-visible behavior fix for task rows that shadow restorable conversations.
Increment 3, `INCREMENT-3-agent-management-and-details.md`, migrates Agent Management cards, filters, action buttons, and details panel data to normalized entries.
Increment 4, `INCREMENT-4-cleanup-and-hardening.md`, removes or narrows `ConversationOrTask`, deletes `link_preference()` as a source of truth, and adds regression coverage for previously inconsistent entrypoints.
## End-to-end flow
1. `AgentConversationsModel` keeps its existing task and conversation caches.
2. The entry builder indexes tasks by stable run id, conversations by local id, and metadata by server token.
3. For each ambient run, the builder creates one `AmbientRun` entry and attaches matching local conversation/server-token data when available.
4. For each local/cloud metadata conversation not already attached to a run, the builder creates one `Conversation` entry.
5. List/detail surfaces render from `AgentConversationEntry`.
6. On click, the navigation resolver re-reads current task, history, active-view, and workspace state before producing a `WorkspaceAction`.
## Testing and validation
The migration should be implemented as stacked PRs. Each increment spec lists its focused test coverage. Across the full migration, add tests for:
- local conversation only
- cloud metadata only with `has_local_data = false`
- task only with active joinable session
- task only with terminal state and server token
- task plus local conversation by run id
- task plus local conversation by server token
- task with stale in-progress state but terminal local conversation status
- task with `session_link` but no parseable `session_id`
- existing open local conversation focus
- existing open ambient session focus
- copied link and open action using the same resolved identity data
Before opening PRs, run the focused `agent_conversations_model` tests for each increment and a targeted app check. Follow repo PR workflow before review.
## Risks and mitigations
### Merge identity regressions
Wrong merge rules can duplicate rows or hide rows. Mitigate by writing the merge tests before migrating UI and keeping merge precedence explicit: ambient run owns the UI identity when present, but local conversation/server token refs remain attached.
### Recreating stale navigation in a new type
If entries cache `WorkspaceAction`, they will reproduce the current bug. Store identities and capabilities only; resolve final actions at click time.
### Overreaching into transcript content
`AgentConversationEntry` should not expose exchanges, block data, or content mutation methods. Keep transcript operations in `BlocklistAIHistoryModel` and loader paths.
### Incomplete event invalidation
Entries depend on task updates, history events, metadata updates, and active view changes. Initially prefer rebuilding entries on any relevant `AgentConversationsModelEvent` instead of maintaining fine-grained derived caches.
## Parallelization
Increment 1 should be implemented first and mostly sequentially because later increments depend on the entry schema. After that, Increment 2 and Increment 3 can be implemented on separate stacked branches if the entry API is stable. Increment 4 should wait until both UI migrations land.
