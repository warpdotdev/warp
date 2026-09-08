# TECH: Frontend-neutral conversation-list policy

## Context

`AgentConversationsModel` is the process-wide source for normalized local conversations and ambient runs. Its entries combine stable identity, display metadata, backing sources, and capabilities, but navigation availability is currently resolved through GUI-only workspace actions.

The relevant pre-migration paths are:

- [`app/src/ai/agent_conversations_model.rs (1207-1424) @ 99261042`](https://github.com/warpdotdev/warp/blob/992610422e4e441ae1e4d29de17aebbc5e0a1b23/app/src/ai/agent_conversations_model.rs#L1207-L1424) aggregates normalized entries and resolves them into GUI `WorkspaceAction` values.
- [`app/src/ai/agent_conversations_model/entry.rs (30-166) @ 99261042`](https://github.com/warpdotdev/warp/blob/992610422e4e441ae1e4d29de17aebbc5e0a1b23/app/src/ai/agent_conversations_model/entry.rs#L30-L166) defines stable entry identity, normalized display data, backing data, and capabilities.
- [`app/src/terminal/input/conversations/data_source.rs (19-150) @ 99261042`](https://github.com/warpdotdev/warp/blob/992610422e4e441ae1e4d29de17aebbc5e0a1b23/app/src/terminal/input/conversations/data_source.rs#L19-L150) independently excludes the selected conversation and determines which normalized entries the GUI inline menu can open.
- [`app/src/terminal/input/conversations/search_item.rs (20-165) @ 99261042`](https://github.com/warpdotdev/warp/blob/992610422e4e441ae1e4d29de17aebbc5e0a1b23/app/src/terminal/input/conversations/search_item.rs#L20-L165) independently compares open and focused terminal views to render “open in different pane.”
- [`app/src/ai/blocklist/conversation_selection.rs (12-112) @ 99261042`](https://github.com/warpdotdev/warp/blob/992610422e4e441ae1e4d29de17aebbc5e0a1b23/app/src/ai/blocklist/conversation_selection.rs#L12-L112) provides the per-terminal-surface abstraction for the conversation targeted by the next query.

Selection and navigation state are relative to the frontend surface presenting a list. A normalized entry can be selected in one surface, open elsewhere in another, available through one frontend’s navigation model, or unsupported by another frontend. Encoding those states in `AgentConversationEntry` would make process-wide data depend on one presentation context, while exposing `WorkspaceAction` requires every frontend to understand GUI workspace semantics.

## Proposed changes

### Add frontend-relative list policy

Define these types next to `AgentConversationEntry`:

- `AgentConversationListEntryState` with `Selected`, `OpenElsewhere`, `Available`, and `Unavailable`.
- `AgentConversationListPolicy`, which classifies one normalized entry using the current `AppContext`.

Keep `AgentConversationsModel::get_entries` as the normalized management API. List consumers classify each returned entry through their policy while applying presentation-specific filtering. Aggregation, deduplication, ownership filtering, and normalized identity remain centralized in the model; transient frontend state is not stored in a second entry type.

### Make conversation selection own list policy

Extend `ConversationSelection` with `AgentConversationListPolicy`. Both abstractions answer questions relative to the same terminal surface:

- Which conversation receives the next query.
- Whether a normalized entry is that selected conversation.
- Whether the surface can navigate to the entry.
- Whether navigation should focus another surface instead of restoring locally.

Keeping the policy on the existing selection handle avoids a second per-surface model and lets list consumers use the same dynamic frontend implementation that controls query selection.

Test selection implementations classify entries as unavailable unless the test supplies a policy explicitly. This keeps mocks conservative and prevents a test-only selection from accidentally advertising unsupported navigation.

### Preserve GUI behavior through the policy

`AgentViewConversationSelection` implements the GUI policy in this order:

1. Return `Selected` when the entry’s local conversation identity matches the controller’s selected conversation.
2. Return `OpenElsewhere` when `ActiveAgentViewsModel` maps the entry to a terminal surface other than the selection’s own surface.
3. Return `Available` when the existing active-pane open-action resolution succeeds.
4. Otherwise return `Unavailable`.

`ConversationMenuDataSource` stores `ConversationSelectionHandle`, requests normalized entries, classifies each entry, and retains only `Available` and `OpenElsewhere`. It preserves the existing current-directory filter, recency ordering, default result cap, fuzzy matching, and search result cap.

`ConversationSearchItem` receives `is_open_elsewhere` from the classified result rather than consulting global focus state while rendering. The accepted action carries the same presentation state for its footer copy, while actual navigation continues to re-resolve the stable entry ID through `AgentConversationsModel`.

The menu constructor receives the terminal surface’s existing selection handle. The now-redundant focused-terminal getter is removed from `ActiveAgentViewsModel`.

### Implement the policy for the headless frontend

The headless conversation selection implements the same trait without importing GUI workspace actions. It classifies its selected local conversation as `Selected`, safely restorable terminal-state Oz entries with local or server identity as `Available`, and unsupported entries as `Unavailable`. It never returns `OpenElsewhere` because the frontend currently owns one conversation surface.

Only the entry, state, policy, display status, and harness types needed for that implementation are exported through `app/src/tui_export.rs`.

## Testing and validation

- Add focused GUI policy coverage for selected, open-elsewhere, available, and unavailable entries.
- Retain the GUI inline menu’s filtering, ordering, fuzzy search, open-elsewhere suffix, and acceptance behavior.
- Add headless policy tests covering selected entries, supported terminal-state entries, active entries, unsupported harnesses, and entries without restoration identity.
- Run formatting and linting for both the main application and `warp_tui`.

## Risks and mitigations

- **GUI behavior drift:** The GUI policy delegates availability to the existing active-pane open-action resolution and migrates only the inline conversation menu.
- **Stale presentation state:** Accepted actions retain stable entry identity, and navigation re-resolves that identity instead of trusting the classification captured by the search row.
- **Frontend coupling:** Shared entry aggregation does not import frontend actions; policy implementations remain alongside each frontend’s conversation selection.

## Parallelization

Parallel implementation is not useful because the trait definition, every `ConversationSelection` implementation, the GUI data source, and exported types must compile together. The migration is one dependency chain and should land as one branch.
