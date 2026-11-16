# Persist TUI agent prompts for history
## Context
The shared [`BlocklistAIHistoryModel`](https://github.com/warpdotdev/warp/blob/41b922e0/app/src/ai/blocklist/history_model.rs) owns in-memory agent conversations for both the GUI and TUI. Prompt submission updates that model and emits `BlocklistAIHistoryEvent` values, but persistence is a downstream projection rather than part of submission.

The GUI projection lives in [`app/src/pane_group/pane/terminal_pane.rs`](https://github.com/warpdotdev/warp/blob/41b922e0/app/src/pane_group/pane/terminal_pane.rs): its per-pane history subscription filters events for the pane's terminal surface, serializes query-bearing exchanges as `PersistedAIInput`, and sends `ModelEvent::UpsertAIQuery` to the persistence writer. The TUI subscribes to the same history model in [`crates/warp_tui/src/terminal_session_view.rs`](https://github.com/warpdotdev/warp/blob/41b922e0/crates/warp_tui/src/terminal_session_view.rs), but does not currently project those events into its persistence writer. TUI prompts therefore remain available only while their conversations are in memory.

The GUI and TUI use separate SQLite scopes. [`PersistenceScope::Tui`](https://github.com/warpdotdev/warp/blob/41b922e0/app/src/persistence/mod.rs#L63-L72) prevents schema/version skew between the two frontends, so the TUI must write its own prompt rows for later TUI launches to restore them.
## Proposed changes
### Share the query persistence projection
Move the event filtering and `PersistedAIInput` construction from the GUI pane handler into `maybe_build_ai_query_upsert_event` in `app/src/ai/blocklist/persistence.rs`.

The helper:
- accepts `AppendedExchange` and `UpdatedStreamingExchange` events for the requested terminal surface;
- reads the corresponding conversation and exchange from `BlocklistAIHistoryModel`;
- excludes hidden exchanges, passive-only conversations, shared ambient-agent sessions, and exchanges without persistable inputs;
- returns `ModelEvent::UpsertAIQuery` with the exchange's current output status and metadata.

Keep writer ownership outside the history model. Persistence availability, execution policy, and surface lifecycle remain frontend concerns, while serialization and exclusion rules have one implementation.
### Preserve GUI behavior
Update `handle_ai_history_event` in `app/src/pane_group/pane/terminal_pane.rs` to call the shared helper. Retain the existing GUI checks for session restoration and execution modes that can save sessions, the pane-scoped terminal-surface filter, and asynchronous delivery through the pane group's writer sender.
### Add the TUI writer path
Expose `maybe_build_ai_query_upsert_event` and `PersistenceWriter` through `app/src/tui_export.rs`.

In `TuiTerminalSessionView::handle_history_event`, build an upsert for matching history events and send it asynchronously through the TUI process's `PersistenceWriter`. TUI terminal sessions are not shared ambient-agent viewers, so the helper receives `false` for that exclusion input.

The TUI does not apply the GUI's `GeneralSettings::restore_session` gate. That setting belongs to the GUI settings surface and controls restoration of GUI windows, tabs, panes, and blocks; TUI prompt history uses an independent settings and persistence scope.
## Testing and validation
- Unit-test `maybe_build_ai_query_upsert_event` with a restored conversation containing a user query. Verify the resulting `UpsertAIQuery` preserves the conversation ID, exchange ID, and serialized query input.
- Run the blocklist persistence unit tests covering the shared projection.
- Run the `warp_tui` test suite to ensure the new writer lookup and history-event subscription compile with the TUI feature.
- Run `./script/format` and the repository clippy command before submitting.
- Manually submit a prompt in the TUI, restart the TUI, and verify the query is present in the TUI-scoped `ai_queries` table.
## Parallelization
Parallel agents are not useful for this change. The shared projection, GUI call site, TUI call site, and focused tests form one small dependency chain and touch overlapping APIs.
