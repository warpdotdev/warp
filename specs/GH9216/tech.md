# TECH.md — Configurable `clear` command behavior

**GitHub Issue:** [warpdotdev/warp#9216](https://github.com/warpdotdev/warp/issues/9216)
**Product Spec:** `specs/GH9216/product.md`

## Context

Warp already has two distinct clear concepts:

- Shell-originated `clear` currently preserves history by creating a viewport gap. The shell hook path is `TerminalModel::clear(...)`, which calls `clear_visible_screen()` (`app/src/terminal/model/terminal_model.rs:2947`, `app/src/terminal/model/terminal_model.rs:1870`). `BlockList::clear_visible_screen()` finishes background blocks, removes existing gap items, inserts a new gap before or after the active block, and emits `TerminalClear` (`app/src/terminal/model/blocks.rs:809`).
- Explicit "Clear Buffer" deletes session blocks. `TerminalView::clear_buffer()` clears view state, calls `TerminalModel::clear_screen(ClearMode::ResetAndClear)`, clears find/bookmark/rich-content state, and emits `Event::BlockListCleared` (`app/src/terminal/view.rs:17362`). At the block-list level, `ClearMode::ResetAndClear` drains all blocks except the active block and resets block indices and selection (`app/src/terminal/model/blocks.rs:3492`).

Clear-screen escape sequences also flow through the ANSI parser. `CSI J` maps `0`, `1`, `2`, and `3` to `ClearMode::Below`, `Above`, `All`, and `Saved` respectively (`app/src/terminal/model/ansi/mod.rs:1325`). On the primary screen, `ClearMode::All` delegates to `clear_viewport()`/`clear_visible_screen()` today, while `ClearMode::Saved` already clears stored history (`app/src/terminal/model/grid/ansi_handler.rs:737`).

Relevant settings infrastructure:

- `TerminalSettings` is defined in `app/src/terminal/settings.rs:87` via `define_settings_group!`.
- `TerminalView::new` subscribes to `TerminalSettingsChangedEvent` for settings that must update live terminal models (`app/src/terminal/view.rs:3363`).
- The Features settings page renders Terminal widgets in its Terminal category (`app/src/settings_view/features_page.rs:2702`) and already contains terminal-setting examples such as `AudibleBellWidget` (`app/src/settings_view/features_page.rs:6597`).
- Terminal models are created through `create_terminal_model(...)`, which reads `TerminalSettings` while building initial model state (`app/src/terminal/terminal_manager.rs:74`).

Product behavior is defined in `specs/GH9216/product.md`.

## Proposed changes

### 1. Add a typed terminal setting

In `app/src/terminal/settings.rs`, add:

```rust
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "How shell-originated clear-screen requests behave.", rename_all = "snake_case")]
pub enum ClearCommandBehavior {
    #[default]
    #[schemars(description = "Clear the viewport while preserving session history.")]
    PreserveSessionHistory,
    #[schemars(description = "Delete prior session history from the current terminal view.")]
    DeleteSessionHistory,
}
```

Add a `TerminalSettings` entry:

- type: `ClearCommandBehavior`
- default: `ClearCommandBehavior::PreserveSessionHistory`
- supported platforms: `SupportedPlatforms::ALL`
- sync: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`
- private: `false`
- TOML path: `terminal.clear_command_behavior`
- description: "Controls whether the clear command preserves or deletes session history."

The enum gives settings schema users stable `snake_case` values:

- `preserve_session_history`
- `delete_session_history`

### 2. Store live clear behavior on `TerminalModel`

Add a `clear_command_behavior: ClearCommandBehavior` field to `TerminalModel`.

Thread the initial value from `TerminalSettings` through `create_terminal_model(...)` into `TerminalModel::new(...)` so restored and newly created panes start with the current setting.

Add:

```rust
pub fn set_clear_command_behavior(&mut self, behavior: ClearCommandBehavior)
```

In `TerminalView::new`, extend the existing `TerminalSettingsChangedEvent` subscription so `TerminalSettingsChangedEvent::ClearCommandBehavior { .. }` calls `model.set_clear_command_behavior(*terminal_settings.as_ref(ctx).clear_command_behavior.value())`. This makes behavior changes apply to open panes without restart.

### 3. Centralize block-history deletion cleanup for shell clear

`TerminalView::clear_buffer()` contains both product-specific Clear Buffer behavior and generic cleanup required after deleting blocks. Split the generic cleanup into a helper that can be reused by shell clear without inheriting agent-view-specific semantics:

```rust
fn cleanup_after_block_history_deleted(&mut self, ctx: &mut ViewContext<Self>)
```

This helper owns the state cleanup required by product invariant 13:

- clear selected blocks and block-list text selection
- reset or prune AI context references to deleted blocks
- clear find matches and update find selection
- clear label/bookmark/filter mouse state maps
- clear `bookmarked_blocks`
- clean up active AI/rich-content blocks that were removed
- clear `rich_content_views`
- update prompt suggestion banner state
- reset restored/remote-block flags that are no longer valid
- update focused terminal info
- notify the view

Keep the Clear Buffer-specific behavior in `clear_buffer()`:

- fullscreen agent-view special case
- agent-monitoring early return
- `blocklist_has_been_cleared = true`
- emitting view-level `Event::BlockListCleared`
- subshell separator reinsertion
- clearing blocklist AI history conversations
- clearing autosuggestions and prompt suggestions
- onboarding reset
- error-state reset

For shell-originated delete-history clear, use only the generic cleanup plus any minimal state needed to keep block indices and focus valid. Do not start a new agent conversation and do not apply the fullscreen-agent Clear Buffer shortcut.

### 4. Apply the setting to shell clear hooks

Change `TerminalModel::clear(&mut self, _data: ClearValue)`:

- `PreserveSessionHistory`: keep the current call to `self.clear_visible_screen()`.
- `DeleteSessionHistory`: delete history using the same model primitive as Clear Buffer, then emit a model event so `TerminalView` can run the generic cleanup helper.

Add a terminal model event such as:

```rust
Event::TerminalHistoryDeletedByClearCommand
ModelEvent::TerminalHistoryDeletedByClearCommand
```

Flow:

1. `TerminalModel::clear(...)` receives the `ClearValue` shell hook.
2. If behavior is `DeleteSessionHistory`, call `self.clear_screen(ClearMode::ResetAndClear)`.
3. Send `Event::TerminalHistoryDeletedByClearCommand`.
4. Convert it in `model_events.rs`.
5. In `TerminalView::handle_terminal_event(...)`, run `cleanup_after_block_history_deleted(ctx)`, update scroll position to the post-clear active block, and notify.

This keeps model mutation synchronous with PTY parsing while ensuring view-owned caches cannot reference deleted blocks.

### 5. Apply the setting to primary-screen full-screen erase sequences

`clear` often emits terminal sequences in addition to or instead of Warp shell hooks. To make product invariant 9 reliable, route primary-screen `ClearMode::All` through the same behavior.

Recommended implementation:

- Add a `clear_primary_screen_all()` helper on `TerminalModel` or `BlockList` that is used when the active surface is the primary block list and `ClearMode::All` is received.
- In `TerminalModel::clear_screen(ClearMode::All)`, preserve current alternate-screen behavior. For the primary block list:
  - `PreserveSessionHistory`: keep `delegate!(self.clear_screen(ClearMode::All))`, preserving today's viewport-gap behavior.
  - `DeleteSessionHistory`: use `ClearMode::ResetAndClear` and emit `TerminalHistoryDeletedByClearCommand`.

Do not alter `ClearMode::Saved`; it already represents an explicit saved-history deletion request and should remain independent of the new setting.

If implementation finds that shell-integrated `clear` always emits `ClearValue` before the ANSI sequence, guard against duplicate handling by ensuring a single clear request produces one history deletion. A short-lived parser/model flag for "handled clear hook during this byte-processing batch" is preferable to allowing two consecutive `ResetAndClear` passes.

### 6. Add settings UI

In `app/src/settings_view/features_page.rs`:

1. Add `FeaturesPageAction::SetClearCommandBehavior(ClearCommandBehavior)`.
2. Add telemetry mapping for the action using a non-sensitive enum/debug string.
3. Handle the action by setting `TerminalSettings::clear_command_behavior`.
4. Add `ClearCommandBehaviorWidget` to the Terminal category near other terminal behavior settings.
5. Render it as a dropdown, not a switch, because the labels communicate two explicit modes:
   - "Preserve session history"
   - "Delete session history"

The widget should:

- include search terms such as `clear command clear screen scrollback session history`
- use `LocalOnlyIconState::for_setting(...)` with the new setting's storage key/sync mode
- update the dropdown selected item when `TerminalSettingsChangedEvent::ClearCommandBehavior` fires

### 7. Shared sessions and restored sessions

For shared sessions, the sharer's model mutation should determine what viewers see. Do not re-interpret the clear behavior on a viewer using the viewer's local setting; viewers should receive the resulting ordered terminal/model events and render the already-preserved or already-deleted history.

For restored sessions, no migration is required. The setting only changes future clear requests. If a restored terminal receives a delete-history clear request, the same `ResetAndClear` path removes restored blocks from the current model.

## End-to-end flow

### Default behavior

1. User runs `clear`.
2. Shell integration emits `ClearValue` or a primary-screen full erase sequence.
3. Terminal model sees `ClearCommandBehavior::PreserveSessionHistory`.
4. Model calls `clear_visible_screen()`.
5. `BlockList::clear_visible_screen()` inserts a viewport gap and emits `TerminalClear`.
6. `TerminalView` handles `ModelEvent::TerminalClear` and scrolls after clear.
7. Previous blocks remain available above the gap.

### Delete-history behavior

1. User chooses **Clear command behavior → Delete session history**.
2. `TerminalSettingsChangedEvent::ClearCommandBehavior` updates every open terminal model.
3. User runs `clear`.
4. Terminal model sees `ClearCommandBehavior::DeleteSessionHistory`.
5. Model calls `clear_screen(ClearMode::ResetAndClear)` and emits `TerminalHistoryDeletedByClearCommand`.
6. `TerminalView` runs `cleanup_after_block_history_deleted(...)`, refreshes focus/scroll state, and notifies.
7. Previous blocks are gone; scrolling to the top reaches the current prompt or post-clear output.

## Risks and mitigations

1. **Accidental data loss if the default changes.** The default remains `PreserveSessionHistory`; delete-history behavior is opt-in.
2. **View caches referencing removed blocks.** Factor and reuse cleanup from `clear_buffer()` so find results, selections, bookmarks, rich content, and AI context references are cleared when shell clear deletes blocks.
3. **Double clear from shell hook plus ANSI sequence.** Add a parser/model guard if needed so one logical clear command causes one model mutation.
4. **Deleting too much for TUI redraws.** Keep alternate-screen `ClearMode::All` behavior unchanged and apply the setting only to the primary block-list surface.
5. **Shared-session divergence.** Treat clear behavior as part of the sharer's session stream rather than applying viewer-local settings.
6. **Regression in long-running commands.** Unit tests should cover clear requests emitted while the active block is running so post-clear output remains in the active block.

## Testing and validation

Tests map to `specs/GH9216/product.md` behavior invariants.

1. **Settings tests**
   - Schema/default test verifies `terminal.clear_command_behavior` defaults to `preserve_session_history`.
   - Serialization/deserialization test verifies both TOML values map to the enum.
   - Settings live-update test verifies changing the setting calls `TerminalModel::set_clear_command_behavior(...)` for an existing terminal.

2. **Model tests in `app/src/terminal/model/terminal_model_test.rs` or `blocks_test.rs`**
   - Preserve mode: simulate multiple blocks, run the clear hook, assert block count remains and an active gap exists.
   - Delete mode: simulate multiple blocks, set delete behavior, run the clear hook, assert only the active/current block remains and block indices are reset.
   - ANSI primary-screen `CSI 2J`: assert it follows the setting.
   - ANSI `CSI 3J`: assert saved-history clearing behavior remains independent of the setting.
   - Alternate-screen `CSI 2J`: assert block-list history is not deleted.
   - Long-running active block: emit clear while active, then append output, and assert post-clear output remains visible in the active block.

3. **View tests in `app/src/terminal/view_test.rs`**
   - With delete mode enabled, simulate blocks with bookmarks, selected blocks, find matches, and rich content where practical; run shell clear and assert removed-block UI state is gone.
   - Verify Clear Buffer still deletes blocks regardless of the new setting.
   - Verify shell clear does not run fullscreen-agent Clear Buffer behavior or start a new agent conversation.

4. **Manual verification**
   - Linux: run `printf 'old\n'; clear; git diff` in both modes and verify scroll-to-top behavior.
   - macOS and Windows/Linux: use the Clear Buffer shortcut and verify unchanged behavior.
   - `less`/`vim`: clear/redraw inside alternate screen and verify block history is preserved according to block-list behavior only.
   - Restart Warp and verify the selected clear behavior persists.

## Follow-ups

1. Consider adding a short docs entry for users migrating from terminals where `clear` deletes history by default.
2. If users ask for more granularity, evaluate per-profile or per-shell overrides later; they are intentionally out of scope for this setting.
