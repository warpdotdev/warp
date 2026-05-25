# Inline conversation and history navigation via agent entries
## Context
APP-4313 makes inline conversation surfaces use `AgentConversationsModel` entries instead of local-only `ConversationNavigationData`. 
The old source, `ConversationNavigationData`, is local-conversation-centric. It is built from open terminal views, cleared/live local conversations, and local metadata in `app/src/ai/conversation_navigation/mod.rs (18-348)`. It has pane/window navigation fields, but no ambient task identity.
The new source, `AgentConversationsModel`, exposes one normalized projection for local conversations and ambient runs:
- `get_entries` merges task rows, loaded conversation rows, and historical metadata rows in `app/src/ai/agent_conversations_model.rs (1032-1081)`.
- When a task shadows a local conversation, the task keeps row ownership.
- `AgentConversationEntryId` identifies either an `AmbientRun` or `Conversation` in `app/src/ai/agent_conversations_model/entry.rs (29-38)`.
The same model owns open-action policy. `resolve_open_action` re-reads current state at accept time, then prefers:
- already-open ambient sessions
- already-open local conversations
- joinable ambient sessions
- restorable local conversations
- transcript viewer fallback
See `app/src/ai/agent_conversations_model.rs (1117-1239)`.
`ActiveAgentViewsModel` tracks open/focused local conversations and ambient sessions through `ConversationOrTaskId` in `app/src/ai/active_agent_views_model.rs (25-66)`. Its focused/open terminal view lookup APIs feed inline menu labels and suffixes in `app/src/ai/active_agent_views_model.rs (297-515)`.
Before this change, inline conversation menu rows carried `ConversationNavigationData`, and inline history rows carried `AIConversationId`. That excluded task-backed cloud agent rows and duplicated navigation rules in input handlers.

## Proposed changes
### Shared row identity
Use `AgentConversationEntryId` as the row action payload.
`AcceptConversation` now stores `item_id`. The message bar maps that id to `ConversationOrTaskId` to choose “go to conversation” vs “continue in this pane” in `app/src/terminal/input/conversations/mod.rs (29-72)`.
Inline history mirrors that shape. `AcceptHistoryItem`, `MenuItem`, row identity restoration, and accepted events now preserve `AgentConversationEntryId` in `app/src/terminal/input/inline_history/data_source.rs (33-219)` and `app/src/terminal/input/inline_history/view.rs (34-87)`.
### Inline conversation menu
`ConversationMenuDataSource` now starts from `AgentConversationsModel::get_entries(...)`, then keeps only rows where `resolve_open_action(..., ActivePane, ...)` returns an action. This hides unopenable rows and centralizes navigation policy in `app/src/terminal/input/conversations/data_source.rs (32-50)`.
Search behavior stays the same:
- empty query shows up to 50 recent entries
- non-empty query fuzzy-matches title
- Current Directory compares `entry.display.working_directory`
- the currently active local conversation is hidden
See `app/src/terminal/input/conversations/data_source.rs (57-156)`.
Rendering uses entry display data:
- status icon from `entry.display.status`
- timestamp from `entry.display.last_updated`
- title/accessibility text from `entry.display.title`
See `app/src/terminal/input/conversations/search_item.rs (24-196)`.
Open-state rendering asks `ActiveAgentViewsModel::get_terminal_view_id_for_entry(...)` for the open terminal view, using task id first and local conversation id as fallback. It compares that terminal view with `get_focused_terminal_view_id(...)` to decide whether to show “open in different pane” in `app/src/terminal/input/conversations/search_item.rs (99-113)` and `app/src/ai/active_agent_views_model.rs (297-515)`.

## Testing and validation
Preserve:
- inline conversation menu hides the currently active local conversation
- Current Directory filter still works
Verify:
- task-backed `AgentConversationEntryId::AmbientRun` survives inline-history interleaving
- accepting a task-backed row opens or focuses the ambient session when possible
- accepting a local conversation row still restores/navigates into the active pane
- labels and suffixes are correct for open local conversations, open ambient sessions, and task-backed rows with local ids
Manual checks:
- Open inline conversation menu, search a local conversation, accept it, confirm it opens in active pane.
- Open inline history, search a cloud/ambient run, accept it, confirm it focuses or opens through the shared resolver.
- Toggle Current Directory filter and confirm only matching `entry.display.working_directory` rows appear.

## Risks and mitigations
### Stale action payloads
Risk: rows carry a stale `WorkspaceAction` or old `ConversationNavigationData`.
Mitigation: rows carry only `AgentConversationEntryId`; actions resolve at accept time.
### Hidden task-backed rows
Risk: menus only show local conversations.
Mitigation: data sources start from `AgentConversationsModel::get_entries(...)` and only drop rows without an open action.
### Local/task alias mismatch
Risk: a task-backed row with a local conversation id renders wrong open/focused state.
Mitigation: rendering uses the same task-first/local-fallback terminal view lookup shape as open-action resolution.
### Over-broad history rows
Risk: display-only metadata creates unopenable history rows.
Mitigation: both menu data sources filter through `resolve_open_action`.
