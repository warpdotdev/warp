# Renaming Conversations — Tech Spec
Product spec: `specs/GH8642/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/8642
## Context
The "primary line" on every agent card — the largest, most-readable text on the conversation card both in vertical tabs and in the conversation list panel — is the conversation's display title. It is fully derived today, with no user-editable input.
Display title resolution lives in `AIConversation::title()`:
```rust path=/Users/joey/dev/warp/app/src/ai/agent/conversation.rs start=1129
/// Get the auto-generated title of the given conversation
/// (falling back to the first query if the title is empty).
/// Get the title of the given conversation.
/// Priority: auto-generated task description > initial query > fallback_display_title.
pub fn title(&self) -> Option<String> {
    self.task_store
        .root_task()
        .and_then(|task| {
            if task.description().is_empty() {
                self.initial_query()
            } else {
                Some(task.description().to_owned())
            }
        })
        .or_else(|| self.fallback_display_title.clone())
}
```
The `fallback_display_title` field at `app/src/ai/agent/conversation.rs:208` is the only existing override mechanism, but it is set in a couple of internal paths only and is never user-editable; it is also a *fallback* (lowest priority), not an *override*. To support user rename we need a *highest-priority* per-conversation override.
The vertical-tabs primary line picks up `AIConversation::title()` through `selected_conversation_display_title` in the renderer at `app/src/workspace/view/vertical_tabs.rs:3074`, and `terminal_primary_line_data` (`vertical_tabs.rs:2951-2998`) chooses `conversation_display_title` over `terminal_title` and `last_completed_command` for any card backed by an agent conversation. The conversation list panel reads the same accessor through `ConversationEntry::title(ctx)` (called in `view.rs:1063-1066` for Delete; the analogous Rename code path will reuse this). The command palette conversation search reads it via `app/src/search/command_palette/conversations/data_source.rs:28`. So once `title()` returns the user-set value, every display surface picks it up automatically — no per-surface plumbing required.
Persistence for a conversation row already uses a JSON blob column (`agent_conversations.conversation_data`) carrying `AgentConversationData`, with every newer field added under `#[serde(default, skip_serializing_if = "Option::is_none")]` so legacy rows continue to deserialize and `None` values do not bloat the on-disk payload. See `crates/persistence/src/model.rs:1009-1044` and the `last_event_sequence` precedent at `:1042` plus its three roundtrip / legacy / skip-on-none unit tests at `:1340-1389`. We will follow the same pattern for the new field — no schema migration is needed.
The conversation list overflow menu currently exposes Share / Fork / Delete only (`app/src/workspace/view/conversation_list/view.rs:902-984`). It already gates each item on `is_ambient_agent_conversation` and supplies a tooltip when an item is disabled. We will mirror that pattern for Rename + Reset.
The vertical-tabs cards do not have a context menu today on the agent-card body — there is `ToggleVerticalTabsPaneContextMenu` in `WorkspaceAction` at `app/src/workspace/action.rs:126-130`, but it targets a pane locator and is wired to a different surface (the kebab on the card), not a right-click on the card body. The simplest option is to plumb a separate `ToggleAgentCardContextMenu` action and dispatch it from a `MouseListener` on the card's primary-line container in `vertical_tabs.rs`.
The **existing tab and pane rename UX** is the model we are mirroring exactly. The relevant precedent in code:
- `WorkspaceAction::RenameTab(usize)` / `ResetTabName(usize)` / `RenamePane(PaneViewLocator)` / `ResetPaneName(PaneViewLocator)` / `RenameActiveTab` / `SetActiveTabName(String)` (`app/src/workspace/action.rs:110-115`).
- The corresponding handlers `Workspace::rename_tab` / `clear_tab_name` / `rename_pane` / `clear_pane_name` / `set_active_tab_name`, dispatched in `Workspace::handle_action` (`app/src/workspace/view.rs:19762-19767`).
- The double-click → rename wiring on the tab body in `tab.rs:1614-1629` (suppressed while `is_tab_being_renamed`).
- The vertical-tabs row equivalent at `vertical_tabs.rs:431-444` and the pane title double-click at `vertical_tabs.rs:3460-3463`.
- `"Rename tab"` and conditional `"Reset tab name"` menu items in `tab.rs:301-313` (Reset shown only when `pane_group.custom_title(ctx).is_some()`); the equivalent pane-name menu items at `tab.rs:345-374`.
- The inline editor renderer `render_inline_tab_rename_editor` at `vertical_tabs.rs:3379-3397`, and the `render_title_override` switch at `:3399-3430` that picks between the inline editor (when `is_tab_being_renamed` or `is_pane_being_renamed`) and the static title text.
- The group header inline editor in `render_group_header` at `vertical_tabs.rs:2157-2212`.
A related but separate setting already exists: `tab_settings::TabSettings::use_latest_user_prompt_as_conversation_title_in_tab_names` (`app/src/workspace/view/vertical_tabs.rs:3026-3032`, defined under `specs/APP-4080/`). It picks between two derived sources — auto title vs. latest user prompt. It is not user-editable per conversation. Our user-set title sits **above** that setting in the priority chain, so the setting continues to apply only when no user-set title exists.
The relevant shared event/action types we will extend:
- `BlocklistAIHistoryEvent` enum at `app/src/ai/blocklist/history_model.rs:2059-2185`, currently has variants such as `UpdatedConversationMetadata`, `UpdatedAutoexecuteOverride`, `UpdatedConversationArtifacts`. We will add `UpdatedConversationTitle`.
- `WorkspaceAction` enum at `app/src/workspace/action.rs:99-229+`. We will add `RenameConversation { conversation_id }`, `ResetConversationName { conversation_id }`, `SetConversationUserTitle { conversation_id, title }`, and `ToggleAgentCardContextMenu { conversation_id, position }`.
- `ConversationListViewAction` enum at `app/src/workspace/view/conversation_list/view.rs:111-143`. We will add `RenameConversation { conversation_id }` and `ResetConversationName { conversation_id }`. There is no `Event::ShowRenameDialog` — the inline editor lives directly inside the conversation list view (and inside the agent-card render path), so we don't need a sibling-dialog event hop.
No modal dialog file is added. Rename UI is implemented purely as **inline editor + render-state** within the existing conversation-list and vertical-tabs views, exactly mirroring how tab/pane rename works today.
See product spec for user-visible behavior.
## Proposed changes
### 1. New `user_set_title` field on `AIConversation`
In `app/src/ai/agent/conversation.rs`:
- Add `user_set_title: Option<String>` to the `AIConversation` struct (alongside `fallback_display_title` at `:208`). Initialize to `None` in `AIConversation::new` (`:248`) and `AIConversation::new_restored` (`:287`+).
- Add accessor `pub fn user_set_title(&self) -> Option<&str>`.
- Add setter:
```rust path=null start=null
pub fn set_user_title(
    &mut self,
    title: Option<String>,
    terminal_view_id: EntityId,
    ctx: &mut ModelContext<BlocklistAIHistoryModel>,
) {
    let normalized = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().take(200).collect::<String>());
    if normalized == self.user_set_title {
        return;
    }
    self.user_set_title = normalized.clone();
    self.write_updated_conversation_state(ctx);
    ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationTitle {
        terminal_view_id,
        conversation_id: self.id,
        title: normalized,
    });
}
```
- Update `AIConversation::title()` (currently at `:1133`) to make `user_set_title` the highest-priority source:
```rust path=null start=null
pub fn title(&self) -> Option<String> {
    if let Some(t) = self.user_set_title.as_ref() {
        return Some(t.clone());
    }
    self.task_store
        .root_task()
        .and_then(|task| {
            if task.description().is_empty() {
                self.initial_query()
            } else {
                Some(task.description().to_owned())
            }
        })
        .or_else(|| self.fallback_display_title.clone())
}
```
The `fallback_display_title` chain is left byte-for-byte untouched.
### 2. Persistence — extend `AgentConversationData`
In `crates/persistence/src/model.rs:1009-1044`, add:
```rust path=null start=null
#[serde(default, skip_serializing_if = "Option::is_none")]
pub user_set_title: Option<String>,
```
This follows the exact pattern of `last_event_sequence` at `:1042` — backwards-compatible with legacy rows and small on-disk footprint when `None`. No DB schema migration is required because `agent_conversations.conversation_data` is a JSON column.
Add three unit tests in the existing `tests` mod (`:1335-1389`), mirroring the `agent_conversation_data_roundtrips_last_event_sequence` / legacy / skip-on-none trio:
- `agent_conversation_data_roundtrips_user_set_title`
- `agent_conversation_data_deserializes_legacy_payload_without_user_set_title`
- `agent_conversation_data_skips_serializing_none_user_set_title`
### 3. Conversation restore + write paths
In `AIConversation::new_restored` (`app/src/ai/agent/conversation.rs:287`+):
- Extend the destructured tuple at `:349-410` to include `user_set_title` from `AgentConversationData`. Default to `None` in the `else` branch where `conversation_data` is absent.
- Set `self.user_set_title = user_set_title;` alongside the other restored fields when constructing `AIConversation`.
In whatever path materializes `AgentConversationData` from an in-memory `AIConversation` for persistence (the constructor sites for `AgentConversationData { … }` in `app/src/ai/blocklist/history_model.rs`), add `user_set_title: conv.user_set_title().map(str::to_string),`.
### 4. New `BlocklistAIHistoryEvent::UpdatedConversationTitle` variant
In `app/src/ai/blocklist/history_model.rs:2059-2185`, add:
```rust path=null start=null
UpdatedConversationTitle {
    terminal_view_id: EntityId,
    conversation_id: AIConversationId,
    title: Option<String>,
},
```
Subscribers that already react to `UpdatedConversationMetadata` for refresh purposes (e.g. the conversation list view at `:204-211`, vertical-tabs at the call sites in `view/vertical_tabs.rs`) will gain matching handler arms that just call `ctx.notify()` to re-render. We do not need bespoke per-handler logic because every display surface reads through `AIConversation::title()`.
### 5. New `BlocklistAIHistoryModel::set_conversation_user_title` helper
In `app/src/ai/blocklist/history_model.rs`, add a model-level helper colocated with `toggle_autoexecute_override` (`:1818-1831`):
```rust path=null start=null
pub fn set_conversation_user_title(
    &mut self,
    conversation_id: AIConversationId,
    title: Option<String>,
    ctx: &mut ModelContext<Self>,
) {
    let Some(conversation) = self.conversations_by_id.get_mut(&conversation_id) else {
        return;
    };
    let Some(terminal_view_id) =
        self.terminal_view_id_for_conversation(&conversation_id)
    else {
        return;
    };
    conversation.set_user_title(title, terminal_view_id, ctx);
    // Also mirror onto the cached metadata so the conversation list reflects
    // the rename for not-currently-active conversations.
    if let Some(meta) = self.all_conversations_metadata.get_mut(&conversation_id) {
        meta.user_set_title = conversation.user_set_title().map(str::to_string);
    }
}
```
This requires adding a parallel `user_set_title: Option<String>` to `AIConversationMetadata` (already filtered for `is_ambient_agent_conversation()` in `:1887-1893`) so the conversation list panel still shows the override when the conversation is no longer in `conversations_by_id`. Initialize it from `AIConversation::user_set_title()` in `AIConversationMetadata::from(&AIConversation)`.
### 6. New `WorkspaceAction` variants + handlers
Add to `app/src/workspace/action.rs:99-229+` (alongside the existing `RenameTab` / `ResetTabName` / `RenamePane` / `ResetPaneName` family at `:110-115`):
```rust path=null start=null
RenameConversation { conversation_id: AIConversationId },
CancelConversationRename { conversation_id: AIConversationId },
ResetConversationName { conversation_id: AIConversationId },
SetConversationUserTitle { conversation_id: AIConversationId, title: Option<String> },
ToggleAgentCardContextMenu { conversation_id: AIConversationId, position: Vector2F },
```
Wire each in `Workspace::handle_action` in `app/src/workspace/view.rs:19749-19767`, mirroring how `RenameTab` / `ResetTabName` are dispatched there:
- `RenameConversation { conversation_id }` → `self.begin_rename_conversation(conversation_id, ctx)`. Sets the rename-state for that conversation on whichever surface initiated (see change 7 below for surface ownership) and focuses the editor.
- `CancelConversationRename { conversation_id }` → clears the rename-state on the surface owning the in-flight rename. Dispatched on Esc from the inline editor; persists nothing.
- `ResetConversationName { conversation_id }` → calls `BlocklistAIHistoryModel::set_conversation_user_title(conversation_id, None, ctx)`. Same effect as Save-empty.
- `SetConversationUserTitle { conversation_id, title }` → calls `BlocklistAIHistoryModel::set_conversation_user_title(conversation_id, title, ctx)`. Dispatched on Enter from the inline editor.
- `ToggleAgentCardContextMenu { conversation_id, position }` → opens the right-click menu on the agent card with two items (Rename / Reset, conditionally per spec invariants 4, 10, 11).
All four variants are flagged in `should_save_app_state_on_action` (`action.rs:707-761`), grouped with the existing `RenameTab` / `ResetTabName` family at `:723-728`.
### 7. Inline rename plumbing in vertical tabs
The agent-card primary line is rendered via `render_pane_title_slot` / `render_title_override` (`vertical_tabs.rs:3399-3465`), which already handles the inline-editor-vs-text branch for tab and pane rename. We extend the same branch to handle conversation rename:
- Add a `is_conversation_being_renamed` (and matching `conversation_rename_editor: Option<ViewHandle<EditorView>>`) field to whatever struct backs the agent card row — the same shape used today for `is_tab_being_renamed` / `rename_editor` and `is_pane_being_renamed` / `pane_rename_editor` flowing into `PaneProps` (see `render_title_override` at `:3407-3417`).
- Update `render_title_override` to return the conversation rename editor when `is_conversation_being_renamed` is true, mirroring the tab/pane branches.
- Wrap the title in a `Hoverable` similar to `render_pane_title_slot` (`:3452-3464`) and dispatch `WorkspaceAction::RenameConversation { conversation_id }` on `on_double_click`. Suppress this binding while `is_conversation_being_renamed` is true (same `!is_tab_being_renamed` guard pattern as `tab.rs:1614-1629`).
- On the row's outer mouse handler at `vertical_tabs.rs:370-375`, suppress the `FocusPane` activation when `is_conversation_being_renamed` is true so click-into-editor doesn't double-fire the activate.
- Add a right-click handler on the title region that dispatches `WorkspaceAction::ToggleAgentCardContextMenu { conversation_id, position }`. Only attach for cards whose `terminal_agent_text(...).conversation_display_title.is_some()` and whose `AIConversation::is_viewing_shared_session()` is `false`.
- The context menu populates with `MenuItemFields::new("Rename conversation")` always, plus `MenuItemFields::new("Reset conversation name")` only when `conversation.user_set_title().is_some()` — mirroring the `pane_group.custom_title(ctx).is_some()` gate at `tab.rs:307-313`.
The editor's Enter handler dispatches `WorkspaceAction::SetConversationUserTitle { conversation_id, title: trimmed_value_as_option }` and the Esc handler dispatches a new `WorkspaceAction::CancelConversationRename { conversation_id }` that simply clears `is_conversation_being_renamed` without persisting (same shape as the tab-rename cancel path).
### 8. Inline rename plumbing in the conversation list
In `app/src/workspace/view/conversation_list/view.rs`:
- Extend `ConversationListViewAction` (`:111-143`) with `RenameConversation { conversation_id: ConversationOrTaskId }` and `ResetConversationName { conversation_id: ConversationOrTaskId }`.
- The conversation row item rendering (`item.rs`, sibling file under `conversation_list/`) gets the same inline-editor branch as the agent card: store an `Option<ViewHandle<EditorView>> rename_editor_for: Option<AIConversationId>` on `ConversationListView`, and when a row's id matches, render the editor instead of the title text. This mirrors the per-row hover/state pattern the file already uses in `StateHandles::item_states` (`:60-83`).
- Add `on_double_click` on the row title that dispatches `ConversationListViewAction::RenameConversation { conversation_id }`. The view's `handle_action` arm forwards to `WorkspaceAction::RenameConversation { conversation_id }` for non-ambient, non-shared-viewer rows.
- In the overflow-menu construction (`:902-984`), insert two items between Share and Fork:
  - `MenuItemFields::new("Rename conversation")` — always present for non-shared-viewer conversations; `with_disabled(is_ambient_agent_conversation)` and the documented tooltip when ambient.
  - `MenuItemFields::new("Reset conversation name")` — only added to `items` when the resolved `AIConversation::user_set_title().is_some()` (same conditional pattern as Reset tab name at `tab.rs:307-313`).
  Hide both entirely for shared-session viewer mode (same `if !is_viewing_shared_session()` gate).
- Add `RenameConversation` and `ResetConversationName` arms in `handle_action` (`:847-1156`) that look up the `AIConversationId` (rejecting `ConversationOrTaskId::TaskId(_)` ambient ids) and dispatch the corresponding `WorkspaceAction`.
No new dialog file is added. The conversation list view owns one rename editor handle at a time (per product spec invariant 16), constructed lazily in the `RenameConversation` arm.
### 9. Conversation card right-click menu (vertical tabs side)
The `ToggleAgentCardContextMenu` action (added in change 6) opens a `Menu<WorkspaceAction>` near the click position with the same two items as the conversation list overflow menu:
- `Rename conversation` → dispatches `WorkspaceAction::RenameConversation { conversation_id }`.
- `Reset conversation name` (conditional on `user_set_title().is_some()`) → dispatches `WorkspaceAction::ResetConversationName { conversation_id }`.
Menu state lives next to the existing `ToggleVerticalTabsPaneContextMenu` state in `Workspace`. Dismiss-on-outside-click follows the same pattern (`workspace/action.rs:126-130` handler).
### 10. Fork preserves auto-only behavior
In the existing fork code path (search for `forked_from_server_conversation_token` + the `AgentConversationData` constructor used by forking, both already present in `history_model.rs` per the existing `insert_forked_conversation_from_tasks` at `:2003-2033`), explicitly **do not** copy `user_set_title` into the new conversation. The forked `AgentConversationData` arrives from the server with `user_set_title = None` because we never sync it to the server, but we should also assert in code that the forked-from-server token resets the field to `None`. Add a one-line `conversation.user_set_title = None;` in `insert_forked_conversation_from_tasks` for clarity / regression safety.
### 11. Hooking into `selected_conversation_display_title` (sanity check)
The vertical-tabs primary line and search fragment paths (`vertical_tabs.rs:2916-2949` and `:3059-3086`) read through `selected_conversation_display_title`/`title()`, so changes 1 and 2 are sufficient for those surfaces — no additional plumbing.
The conversation list (`conversation_list/view.rs`) reads via the `ConversationEntry::title(ctx)` accessor from `view_model`. Confirm that `ConversationListViewModel`'s sort key for the Active vs. Past sections is unaffected by a rename — it's keyed on `last_modified_at`, not title — so renames do not reorder rows.
The OS window title and tab title routing through `tab.rs:811-1187` already reads `selected_conversation_display_title`, so no additional changes are needed there.
The command palette conversation search at `app/src/search/command_palette/conversations/search_item.rs:84-180` reads `c.title()` for both display and match; no additional changes are needed.
## Testing and validation
Each numbered invariant in `specs/GH8642/product.md` maps to at least one test or manual step below.
### Unit tests
- `app/src/ai/agent/conversation_tests.rs` (sibling-file pattern per WARP.md) — `title_prefers_user_set_title_over_auto_description`. Builds an `AIConversation` with a non-empty `task.description()`, asserts `title()` returns the description, then calls `set_user_title(Some("My custom"))`, asserts `title()` returns `"My custom"`, then calls `set_user_title(None)`, asserts `title()` returns the description again. Guards invariants 1 and 6.
- `app/src/ai/agent/conversation_tests.rs` — `set_user_title_normalizes_whitespace_and_caps_length`. Saves `"   "` → cleared. Saves `"  hello  "` → `"hello"`. Saves a 250-character string → 200 characters preserved (`hello…<truncated>`). Saves the same value twice → second call is a no-op (no event emitted). Guards invariants 5 and 16.
- `crates/persistence/src/model.rs` `tests` mod — `agent_conversation_data_roundtrips_user_set_title`, `agent_conversation_data_deserializes_legacy_payload_without_user_set_title`, `agent_conversation_data_skips_serializing_none_user_set_title`. Mirror the existing `last_event_sequence` trio. Guards invariant 8.
### Render / model tests
- `app/src/workspace/view/vertical_tabs_tests.rs` — extend the existing test points around `:118-237` with a case where `user_set_title` is set: assert the primary-line text returned by `terminal_primary_line_data` is the user-set title, that the search fragments include the user-set title rather than the description, and that with the existing `use_latest_user_prompt_as_conversation_title_in_tab_names` flag toggled the user-set title still wins. Guards invariants 1, 7, 12.
- `app/src/ai/agent_conversations_model_tests.rs` — `rename_updates_conversation_list_entry_title`. Insert a conversation with a non-empty description, call `set_conversation_user_title`, assert the model emits `BlocklistAIHistoryEvent::UpdatedConversationTitle` and that the conversation list entry's `title(ctx)` returns the new title. Guards invariants 2 and 7.
- `app/src/workspace/view/conversation_list/view_tests.rs` (or extend the closest existing tests file) — assert the overflow menu contains both `Rename conversation` (always for non-shared-viewer) and `Reset conversation name` (only when `user_set_title.is_some()`), in the documented order between Share and Fork. Assert that both items are disabled with the documented tooltip for `ConversationOrTaskId::TaskId(_)` (ambient agents), and hidden when `is_viewing_shared_session()` is `true`. Assert that double-clicking a row dispatches `ConversationListViewAction::RenameConversation`. Guards invariants 2, 4, 10, 11.
- New `app/src/workspace/view/conversation_list/rename_editor_tests.rs` — with the inline editor active for a row: assert Enter on empty input dispatches `WorkspaceAction::SetConversationUserTitle { title: None }`; Enter on whitespace dispatches `None`; Enter on a real string dispatches `Some(trimmed)`; Esc dispatches `WorkspaceAction::CancelConversationRename` and does not call into the model. Guards invariants 3, 5, 6, 16.
- `app/src/workspace/view/vertical_tabs_tests.rs` — add a render test that flips `is_conversation_being_renamed` and asserts `render_title_override` returns the inline editor element (not the static `Text`); plus a test that double-click on the title region dispatches `RenameConversation`, and that double-click while `is_conversation_being_renamed` is true is a no-op (mirroring `tab.rs:1614-1629`). Guards invariants 2 and 16.
### Integration / manual validation
- Start an agent conversation; confirm the auto title appears on the agent card. Open the conversation list overflow menu and confirm `Rename conversation` and (after a rename) `Reset conversation name` appear above Delete. Right-click the agent card body and confirm the same two items in a context menu. (Invariants 2, 4, 7.)
- Double-click the title on either surface; confirm the title text is replaced by an inline editor seeded with the current title, the surrounding card click-to-activate is suppressed, Enter saves, and Esc cancels. (Invariants 2, 3, 16.)
- Save a custom title; confirm it appears on the agent panel card, the conversation list row, the command palette conversation search, and the OS window title. Restart Warp; confirm it persists. (Invariants 5, 7, 8.)
- With a user-set title in place, select `Reset conversation name` from either menu; confirm the auto-generated title returns and the Reset item disappears from the menu. Clear the editor input via Save-empty and confirm the same outcome. (Invariants 4, 6.)
- Fork the renamed conversation; confirm the fork uses the auto-generated title, not the parent's user-set title. Rename the fork independently and confirm both retain their respective titles. (Invariant 9.)
- Open an ambient-agent conversation; confirm `Rename conversation` and `Reset conversation name` are visible but disabled with the documented tooltip in both menus, and that double-click on the title does not enter rename mode. (Invariant 10.)
- Open a shared-session viewer for a conversation owned by another user; confirm both items are hidden in both menus and double-click does nothing. (Invariant 11.)
- Toggle `Use latest user prompt as conversation title in tab names` on and off; confirm the user-set title wins in either state, and the setting still works for unrenamed conversations. (Invariant 12.)
## Risks and mitigations
### Risk: Stale per-card cache shows the old title after rename
The vertical-tabs renderer reads `terminal_agent_text(...)` and computes the primary line on every render. As long as the `UpdatedConversationTitle` event triggers `ctx.notify()` on the relevant view, the next render picks up the new title. The conversation list view already subscribes to model events at `view.rs:204-211` and re-syncs its list items; we will mirror the wiring in `vertical_tabs.rs` if the existing subscription doesn't already cover it (it does, via `BlocklistAIHistoryModel`-handle subscription at the workspace view).
Mitigation: explicit render-test coverage in `vertical_tabs_tests.rs` confirms the new title appears in the primary line on the next render frame.
### Risk: Rename racing with auto-title regeneration
The agent updates `task.description()` over time. Without an override, that update naturally surfaces in the card. With a user-set title, the update would be silently masked. This is the desired behavior (the user explicitly chose their title), but it is also a confusing failure mode if a user forgets they renamed something.
Mitigation: the explicit `Reset conversation name` menu item restores the auto title in one click — same affordance as `Reset tab name`. Users who don't remember renaming will discover Reset by right-clicking the title, mirroring how they discover the equivalent tab affordance today.
### Risk: Cloud-synced viewers see different titles
Two clients (or a shared-session viewer + the owner) will see different titles for the same conversation: the owner's machine shows the user-set title; everyone else sees the auto title.
Mitigation: explicit non-goal in the product spec. Follow-up to add a server field on `ServerAIConversationMetadata` is straightforward and can ship behind a separate feature flag once we have product alignment on the cloud-sync UX.
### Risk: Title length explosion / pathological input
Long titles or escape sequences could break the renderer's truncation/clipping. The vertical-tabs renderer already trims/clips long primary text via `Clipped` and the existing `Text` element handles overflow.
Mitigation: 200-character cap in `set_user_title`. Whitespace trimming on save. The renderer's existing clipping covers anything that gets through.
### Risk: Inline editor state leaks across surfaces
A rename initiated on the conversation list does not bleed into the vertical-tabs card (and vice versa) — each surface owns its own editor handle, mirroring how tab and pane rename editors are independently owned today (`is_tab_being_renamed` vs. `is_pane_being_renamed` in `vertical_tabs.rs:3407-3417`). The underlying `user_set_title` is a single source of truth, so once one surface saves, both render the new title via the existing `UpdatedConversationTitle` notification.
Mitigation: render-test coverage on both surfaces. The shared model state means there is no stale-write window between the two surfaces.
### Risk: Forking + rename order produces unexpected duplicates
A user who renames the parent and then forks: per spec the fork uses the auto title. A user who forks first and then renames the parent: parent change does not propagate to the fork. Both are intentional; both are easy to reason about.
Mitigation: explicit `user_set_title = None;` in `insert_forked_conversation_from_tasks` plus the no-server-sync invariant guarantee this is the only outcome.
## Follow-ups
- Cloud sync of user-set titles via `ServerAIConversationMetadata`. Behind a feature flag, with conflict resolution that prefers the most-recent client write.
- Keyboard shortcut for "rename current conversation" in agent view (would dispatch the same `RenameConversation` action that double-click does, against the active conversation).
- Telemetry event for renames (count, source = list-menu / vertical-tabs-context-menu / double-click, characters changed).
- Optional UX: when an agent generates a much-longer or much-different `task.description()` after a user rename, surface a small "auto title differs" hint near the Reset item so the user can adopt the new auto title with one click.
