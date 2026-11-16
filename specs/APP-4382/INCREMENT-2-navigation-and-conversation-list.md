# Increment 2: navigation resolver and conversation list migration
## Context
The conversation list currently caches `ConversationOrTaskId`s in `ConversationListViewModel` and filters out task rows whose `get_session_status()` is not available in `app/src/workspace/view/conversation_list/view_model.rs (31-183)`. Rendering checks `conversation.get_open_action(None, app)` for cursor/clickability in `app/src/workspace/view/conversation_list/item.rs (377-407)`, and activation recomputes the action in `app/src/workspace/view/conversation_list/view.rs (535-548)`.
This can fail when a visible task row shadows a local conversation row but lacks fresh session/conversation fields. Increment 2 replaces that path with normalized entries and a dynamic navigation resolver.
## Proposed changes
Add an entry navigation resolver near `AgentConversationsModel`, for example:
```rust
pub enum AgentConversationNavigationSubject {
    Entry(AgentConversationEntryId),
    ServerToken(ServerConversationToken),
}

pub fn resolve_open_action(
    subject: AgentConversationNavigationSubject,
    restore_layout: Option<RestoreConversationLayout>,
    app: &AppContext,
) -> Option<WorkspaceAction>
```
The resolver should re-read current state at click time. For `Entry`, resolve the latest `AgentConversationEntry` or equivalent identity refs from `AgentConversationsModel`. For `ServerToken`, resolve through `BlocklistAIHistoryModel::find_conversation_id_by_server_token` and fall back to transcript loading where the caller supports it.
Default resolver order for listed entries:
1. if an ambient run id is attached and `ActiveAgentViewsModel` has an open ambient session, focus/open that tab;
2. if a local conversation id is attached and currently open, focus it;
3. if the ambient run has an active execution with parseable `session_id`, open ambient shared session;
4. if a local conversation id is attached, restore/navigate to the local conversation with the requested layout;
5. if a server token is attached, open the cloud transcript viewer;
6. otherwise return `None`.
The resolver may initially return existing `WorkspaceAction` variants. If focusing an already-open ambient session still relies on `WorkspaceAction::OpenAmbientAgentSession` plus workspace fallback, keep that behavior but ensure the resolver prefers the open ambient identity before transcript fallback.
Migrate `ConversationListViewModel` to cache `AgentConversationEntryId` and `ConversationEntry { id, highlight_indices }`. It should source entries from `AgentConversationsModel::get_entries` with the same personal/all status defaults currently used by the conversation list. Do not filter out completed cloud entries simply because `get_session_status()` is unavailable; filter based on normalized `capabilities.can_open`.
Migrate `render_item` props to take `AgentConversationEntry` or a lightweight view data struct instead of `ConversationOrTask`. The leading icon can continue to use existing helper behavior if Increment 1 exposes enough display data; otherwise add a normalized icon helper that consumes `AgentConversationEntry`.
Migrate click/Enter to call `resolve_open_action(Entry(entry.id), None, ctx)`.
## Testing and validation
Add unit tests for the resolver:
- task row with matching local conversation but missing task `conversation_id` restores local conversation;
- task row with `session_link` but no parseable `session_id` does not claim session-open capability and falls back to local/server token when available;
- active ambient session is preferred over transcript opening;
- active local conversation is preferred over restoring into a new tab;
- server-token-only navigation can open transcript when no local id is known.
Add conversation-list view-model tests if existing harness support is sufficient:
- cloud metadata-only entries appear when openable by token;
- stale/unavailable session status does not hide a restorable local conversation attached to a task;
- search still matches titles from normalized display data.
Manual validation:
- open a local cloud-mode conversation that also has a task row and verify clicking the list item focuses/restores the local conversation even if the task has no active session;
- open a live cloud task and verify clicking the list item focuses/joins the live session;
- open a completed cloud run and verify clicking opens/restores transcript/local conversation consistently.
## Risks and mitigations
### Workspace action gaps
The existing workspace actions may not express “focus open ambient session by task id” directly. If needed, add a small focused action or keep using `OpenAmbientAgentSession` with the workspace’s `find_tab_with_ambient_agent_conversation` fallback.
### UI state churn
Changing list item IDs can reset hover/selection state. Use `AgentConversationEntryId` as a stable key and preserve row state maps by that key.
