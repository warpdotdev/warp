# Renaming Conversations — Tech Spec
Product spec: `specs/GH8642/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/8642
Prior art: https://github.com/warpdotdev/warp/pull/9646
## Context
The product spec defines a local, per-conversation custom title that should behave like tab renaming rather than like a modal dialog.
Current title and navigation behavior is spread across these areas:
- `app/src/ai/agent/conversation.rs:1133` — `AIConversation::title()` derives the display title from the root task description, initial query, and `fallback_display_title`.
- `app/src/ai/agent/conversation.rs:2770` — `write_updated_conversation_state()` persists conversation-level JSON through `AgentConversationData`.
- `crates/persistence/src/model.rs:1010` — `AgentConversationData` is serialized into the `agent_conversations.conversation_data` JSON blob. Existing optional fields use `#[serde(default, skip_serializing_if = "Option::is_none")]`, so adding another optional field does not require a SQL migration.
- `app/src/ai/blocklist/history_model/conversation_loader.rs (436-595)` — startup historical metadata derives titles from persisted tasks without constructing full `AIConversation` instances for every historical row.
- `app/src/ai/conversation_navigation/mod.rs (72-111)` and `app/src/ai/conversation_navigation/mod.rs (308-348)` — conversation navigation data is built from live `AIConversation`s and from historical metadata.
- `app/src/ai/agent_conversations_model.rs:406` — conversation-list rows read live titles from `BlocklistAIHistoryModel` when the full conversation is loaded, otherwise they fall back to cached navigation metadata.
- `app/src/workspace/view/conversation_list/view.rs:111` and `app/src/workspace/view/conversation_list/item.rs:156` — the conversation list owns its actions, overflow menu, and row rendering.
- `app/src/workspace/view.rs (4999-5248)` — existing tab and pane rename flows use inline `EditorView`s, save on Enter/blur, cancel on Escape, and expose explicit reset actions.
- `app/src/tab.rs (272-315)` — tab context menus show `Rename tab` and conditionally show `Reset tab name` only when a custom tab title exists.
- `app/src/workspace/view/vertical_tabs.rs:3022` and `app/src/workspace/view/vertical_tabs.rs:3420` — vertical tabs choose agent conversation text and wrap pane titles with a double-click handler for existing pane rename.
- `app/src/workspace/view.rs (6510-6556)` — the vertical-tabs pane context menu currently injects pane rename/reset items based on the clicked pane.
The prior PR correctly identified the data model direction: a nullable user-owned title at the top of `AIConversation::title()`, persisted in `AgentConversationData` without a DB migration, with forked conversations starting unset. The new spec should keep that foundation but replace the modal-only UX with inline tab-like rename and an explicit reset action.
## Proposed changes
### Data model and persistence
1. Add `user_set_title: Option<String>` to `AIConversation`.
   - Initialize it to `None` in `AIConversation::new`.
   - Read it from restored `AgentConversationData` in `AIConversation::new_restored`.
   - Add accessors like `user_set_title(&self) -> Option<&str>` and `has_user_set_title(&self) -> bool`.
   - Add a setter that takes `Option<String>`, trims leading/trailing whitespace, enforces the 200-character product limit, stores `None` for empty/whitespace-only input, and preserves internal whitespace and emoji.
2. Split the existing derived-title logic out of `AIConversation::title()` into a helper such as `derived_title()` or `auto_title()`. Then make `title()` return `user_set_title.clone().or_else(|| self.derived_title())`.
3. Add `user_set_title: Option<String>` to `AgentConversationData` with the same serde pattern as `last_event_sequence`.
   - Include it in every `AgentConversationData` literal in production code and tests.
   - Persist it from `write_updated_conversation_state()`.
   - Preserve legacy compatibility: old JSON without the field deserializes as `None`; `None` is skipped on serialize.
4. Update historical metadata construction in `conversation_loader.rs` to parse `user_set_title` from `conversation_data` and use it as the effective `AIConversationMetadata.title` when present. This is required for past conversations that are listed before their full `AIConversation` is loaded.
5. Add `user_set_title: Option<String>` or an equivalent custom-title flag to `AIConversationMetadata` and `ConversationNavigationData` so list rows and menus can know whether `Reset conversation name` should be shown for loaded and historical conversations.
6. Ensure fork paths explicitly do not copy `user_set_title`.
   - `BlocklistAIHistoryModel::fork_conversation()` and `fork_conversation_at_exchange()` should set `AgentConversationData.user_set_title` to `None`.
   - The forked in-memory conversation therefore starts unset after `insert_forked_conversation_from_tasks()`.
### History-model API and events
1. Add a history-model mutation API such as `set_conversation_user_title(conversation_id, title: Option<String>, ctx) -> Result<(), RenameConversationError>`.
2. The mutation should handle both loaded and historical-local conversations:
   - If the conversation exists in `conversations_by_id`, mutate it, persist it with `write_updated_conversation_state()`, and update `all_conversations_metadata` when metadata exists.
   - If the conversation is not loaded but `AIConversationMetadata.has_local_data` is true, load it from SQLite using the existing `load_conversation_from_db()` path, mutate it, persist it through the same `ModelEvent::UpdateMultiAgentConversation` path, and update metadata. Inserting the loaded conversation into `conversations_by_id` is acceptable if it simplifies consistency, but the operation should not open the conversation in a pane.
   - If the row is cloud-only or task-only with no local `AIConversation`, return a non-renamable error so the UI can disable or ignore the action.
   - If `AIConversation::is_viewing_shared_session()` is true, return a non-renamable error and do not persist.
3. Add `BlocklistAIHistoryEvent::UpdatedConversationTitle { conversation_id, terminal_view_id: Option<EntityId> }`.
   - Include the new event in `BlocklistAIHistoryEvent::terminal_view_id()`.
   - Have `AgentConversationsModel::handle_history_event()` emit `ConversationUpdated` or resync conversations on this event. Since row titles read live data when available, `ConversationUpdated` is enough for loaded conversations, but historical metadata title changes may need `sync_conversations()`.
   - Any workspace/window-title listeners that currently refresh on status or active-conversation changes should also refresh when the active conversation title changes.
4. Keep the server metadata title untouched. This feature is local-only, so `set_server_metadata_for_conversation()` and cloud metadata merge should not upload or overwrite `user_set_title`.
### Workspace vertical-tabs UX
1. Add conversation rename state alongside existing tab and pane rename state.
   - Suggested fields: `current_workspace_state.conversation_being_renamed: Option<AIConversationId>` and `Workspace::conversation_rename_editor: ViewHandle<EditorView>`.
   - Subscribe the editor to Enter/blur/Escape the same way `tab_rename_editor` and `pane_rename_editor` are handled.
2. Add actions to `WorkspaceAction`:
   - `RenameConversation(AIConversationId)`.
   - `ResetConversationName(AIConversationId)`.
   - Optionally `SetConversationUserTitle { conversation_id, title }` if programmatic dispatch is useful.
3. Implement rename handlers by mirroring `rename_tab_internal()`, `finish_tab_rename()`, and `cancel_tab_rename()`:
   - Seed the editor with the current effective conversation title and select all text.
   - Enter or blur calls the history-model setter with `Some(editor_text)`; the setter normalizes empty/whitespace-only to reset.
   - Escape exits rename mode without calling the setter.
   - After save/reset, call `update_window_title(ctx)`, notify, and save app state if necessary for existing title chrome.
4. Teach vertical tabs to render the conversation inline editor when the title slot corresponds to the selected local agent conversation.
   - Extend the terminal-agent text data returned from `terminal_agent_text()` with `conversation_id` and `has_user_set_title`.
   - Extend `PaneProps` or the terminal row props with enough state to know whether the title slot is a conversation title currently being renamed.
   - In `render_pane_title_slot()`, when the slot is a conversation title and `conversation_being_renamed == conversation_id`, render `TextInput` backed by `conversation_rename_editor` instead of static text.
   - Double-clicking that title dispatches `WorkspaceAction::RenameConversation(conversation_id)` instead of `RenamePane`.
5. Extend the vertical-tabs context menu.
   - When the clicked or active pane is a terminal with a selected local agent conversation, add `Rename conversation`.
   - If that conversation has `user_set_title`, add `Reset conversation name`.
   - Preserve existing tab and pane menu items. Labels must distinguish conversation actions from `Rename tab` and `Rename pane`.
   - Hide or disable conversation rename items for shared-session viewer conversations and cloud-only/task-only rows that cannot be renamed locally.
### Conversation-list UX
1. Extend `ConversationListViewAction` with rename/reset actions for `ConversationOrTaskId`.
2. Add a single-line `rename_editor` and `renaming_conversation_id` to `ConversationListView`, similar to the existing search editor but scoped to the active row.
3. In the overflow menu:
   - Add `Rename conversation` for local `ConversationId` rows.
   - Add `Reset conversation name` when `ConversationOrTask::has_user_set_title(app)` is true.
   - Disable `Rename conversation` with tooltip text for `TaskId` rows that do not resolve to local conversation data.
   - Keep existing Share, Fork, and Delete ordering stable except for inserting rename/reset near the other conversation-management actions.
4. Update `conversation_list/item.rs` so a row whose ID matches `renaming_conversation_id` renders the inline editor in place of the title text.
   - Suppress row open on click while the editor is active.
   - Preserve hover, selection, status icon, timestamp, sharing dialog, and overflow button behavior for non-renaming rows.
5. Finish/cancel behavior mirrors tab rename:
   - Enter or blur saves through `BlocklistAIHistoryModel::set_conversation_user_title`.
   - Escape cancels without mutation.
   - A successful save/reset updates `ConversationListViewModel` via the new history event and refocuses sensibly, either back to the list row or to the search field depending on prior focus.
### Search, window title, and display surfaces
1. Command palette conversation search should not need a separate index if `ConversationNavigationData.title` and `ConversationOrTask::title()` return the effective title. Verify:
   - Loaded conversations use `AIConversation::title()`.
   - Historical conversations use metadata initialized with `user_set_title`.
   - Resets update metadata back to the derived title.
2. `TerminalView::selected_conversation_display_title()` and `preferred_agent_tab_titles()` should automatically pick up custom titles through `AIConversation::title()`. The latest-prompt setting must only override the derived fallback when `user_set_title` is absent.
   - If needed, change the title preference helper to accept a `has_user_set_title` flag so the latest-prompt setting cannot override a custom title.
3. `Workspace::update_window_title()` already reads the active pane group's display title. Ensure title-change events for the active conversation cause it to run, and ensure a manually renamed tab still wins where `PaneGroup::display_title()` is the source.
## Risks and mitigations
- Historical rows can be renamed before the full conversation is loaded. Mitigate by updating `conversation_loader.rs` metadata derivation and by making the rename API load/persist local historical conversations through the existing SQLite path.
- The latest-prompt setting could accidentally override a custom conversation title. Mitigate by carrying an explicit `has_user_set_title` flag through vertical-tabs title selection and adding tests for both setting states.
- Inline rename can conflict with click-to-open in the conversation list. Mitigate by suppressing row open while the editor is active and by reusing the tab rename event pattern.
- Adding another optional field to many `AgentConversationData` literals can be easy to miss. Mitigate with compiler errors and targeted serialization tests.
- Cloud and shared-session title behavior can appear inconsistent because local custom titles do not sync. Mitigate with explicit disabling/hiding of rename actions on non-local rows and by documenting local-only behavior in the product spec.
## Testing and validation
1. Unit tests for product Behavior 1, 2, 13, 14, 16, and 21:
   - `AIConversation::title()` returns the derived title when no custom title exists.
   - A custom title wins over task description, initial query, fallback title, and latest-prompt preference inputs.
   - Setting whitespace clears the custom title.
   - Long input is capped at 200 Unicode scalar values.
   - Task-description updates do not overwrite an existing custom title.
2. Persistence tests for Behavior 18 and legacy compatibility:
   - `AgentConversationData` roundtrips `user_set_title`.
   - Legacy JSON without `user_set_title` deserializes as `None`.
   - `None` is skipped when serialized.
   - Historical metadata initialization uses `user_set_title` as the effective title and keeps the derived title available after reset.
3. History-model tests for Behavior 17, 19, 22, 23, and 24:
   - Renaming a loaded local conversation updates memory, metadata, and emits `UpdatedConversationTitle`.
   - Renaming an unloaded historical local conversation loads/persists metadata without opening a pane.
   - Reset restores the derived title in metadata and live views.
   - Forked conversations start with no custom title.
   - Shared-session viewer and cloud-only/task-only rows are not mutated.
4. Workspace/vertical-tabs tests for Behavior 4, 5, 6, 7, 8, 12, 17, and 26:
   - Double-clicking an agent conversation title starts inline conversation rename.
   - Enter and blur save; Escape cancels.
   - `Rename conversation` starts the same editor.
   - `Reset conversation name` only appears when a custom title exists and clears it.
   - Existing tab and pane rename tests still pass.
   - With `use_latest_prompt_as_title` enabled, custom conversation titles still win.
5. Conversation-list tests for Behavior 9, 10, 11, 12, 19, 20, and 24:
   - Overflow `Rename conversation` renders an inline row editor.
   - Reset appears only for custom-titled local conversations.
   - Row click does not open the conversation while editing.
   - Search matches the custom title and stops matching it after reset.
   - Task-only rows show disabled rename affordance or omit reset.
6. Manual validation:
   - Rename an active agent conversation from vertical tabs by double-clicking the title.
   - Rename the same conversation from the conversation list overflow menu.
   - Close the tab, restart Warp, search for the custom title, and reopen the conversation.
   - Reset the conversation name from both entry points and verify the derived title returns.
   - Rename the tab separately and verify it does not change the conversation list title.
   - Fork the renamed conversation and verify the fork does not inherit the custom title.
## Parallelization
The implementation can be split, but the data-model/API work should land before UI work. A practical split is: one agent implements model, persistence, metadata, and tests; a second agent implements vertical-tabs and conversation-list inline rename after the model API is available. The final integration pass should be single-owner because title propagation, focus behavior, and reset visibility cross both UI surfaces.
## Follow-ups
- Cloud sync for custom conversation titles if product decides names should follow users across machines.
- A command-palette action or keybinding for renaming the active conversation.
- Telemetry for rename/reset if the product team wants adoption data.
