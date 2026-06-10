# PR Cleanup: Navigation Stack

## Current checkpoint
- Closed-target follow-up is now implemented for NavStack restoration/pruning:
  - recently closed panes, tabs, and windows restore when back-navigation reaches them before undo-close cleanup expires
  - permanently cleaned-up closed targets are pruned from history so back/forward does not get stuck on stale entries
  - code diff/code review navigation entries preserve temporary closures and restore editor scroll state on a best-effort basis
- Local validation is complete for the current implementation:
  - `cargo check`
  - `cargo fmt`
  - focused `cargo nextest` coverage for the new `ui/src/navigation.rs` history APIs
  - integration coverage for closed-pane restore, closed-tab restore, closed-window restore, and stale closed-tab pruning
  - `cargo clippy --workspace --exclude warp_completer --all-targets --all-features --tests -- -D warnings`
  - `cargo clippy -p warp_completer --all-targets --tests -- -D warnings`
- Remaining follow-up: run the required cloud UI verification flow after the branch is pushed.

## Follow-up in progress: closed-target restoration
- Extend back/forward restoration so entries targeting recently closed panes, tabs, and windows restore those targets while they are still available in undo-close/session restoration state.
- Prune stale nav-stack entries automatically when those closed targets are permanently cleaned up.
- Preserve temporary code diff navigation entries when the replacement pane is closed, and reopen the diff view on restore.
- Add coverage for these behaviors in the nav-stack integration tests and keep `desired_behavior.md` aligned with the implementation.

## Changes Made

### 1. NavigationStack model restructured (`app/src/workspace/nav_stack.rs`)
The original model used a single `entries` vector with a `cursor` index but had multiple bugs:
- `can_go_back()` required `cursor > 1` instead of `> 0` (off-by-one)
- `go_back()` accessed `entries[cursor - 1]` after decrementing, causing underflow with a single entry
- Forward navigation was fundamentally broken — the current position was never saved when going back

**Fix**: Replaced the single-vector model with separate `back` and `forward` stacks.
- `push(entry)` adds to `back`, clears `forward`
- `go_back(current)` pops from `back`, pushes `current` to `forward`
- `go_forward(current)` pops from `forward`, pushes `current` to `back`

### 2. View methods updated (`app/src/workspace/view.rs`)
- Factored out `build_navigation_entry()` from `record_navigation_entry()`
- Updated `navigate_back()` / `navigate_forward()` to build the current entry and pass it to the model

### 3. Platform-specific keybindings in integration tests (`integration/src/test/navigation_stack.rs`)
Tests hardcoded `ctrl--` / `ctrl-shift--` (Mac-only bindings). On Linux, `ctrl--` maps to `DecreaseFontSize`, not `NavigateBack`.

**Fix**: Replaced with `PerPlatformKeystroke` constants:
- Mac: `ctrl--` / `ctrl-shift--`
- Linux/Windows: `alt-left` / `alt-right`

### 4. Missing test registration (`integration/src/bin/integration.rs`)
Added 5 `register_test!()` calls for the nav stack integration tests that were present in `ui_tests.rs` but not registered in the integration binary.

### 5. Unit tests updated (`app/src/workspace/nav_stack_tests.rs`)
Rewrote all unit tests to use the new two-stack model, and added a `test_multiple_back_forward` test exercising longer back/forward traversals.

### 6. Navigation stack data model moved to WarpUI framework (`ui/src/navigation.rs`)
Moved the generic navigation stack logic into the UI framework for reuse:
- `NavigationEntry` trait with `should_push()` dedup hook (default: always push)
- Generic `NavigationStack<E>` with `Entity`, `SingletonEntity`, `Default` impls
- `NavigationAction` enum (`GoBack`/`GoForward`) for future child-view dispatching
- App-layer `nav_stack.rs` refactored to a thin wrapper: type alias + `NavigationEntry` trait impl

### 7. Scroll position capture and restoration
- Added `scroll_snapshot()` / `restore_scroll()` implementations on `TerminalPane`, `CodePane`, `NotebookPane`
- `ScrollSnapshot` changed from a struct to an enum (`Terminal(ScrollPosition)` / `Editor(ScrollPositionSnapshot)`) to support multiple pane types
- Added deferred scroll restoration via `pending_nav_scroll_restore` field on `TerminalView` to avoid race with `AfterResize` scroll updates during tab switch layout cycles

### 8. New integration tests
Added 7 integration tests:
- `test_nav_stack_pane_operations_no_entry` — split pane recording
- `test_nav_stack_pane_focus_tracking` — pane focus in entries
- `test_nav_stack_pane_focus_preserved_across_tabs` — pane focus across tab switches
- `test_nav_stack_multi_window_isolation` — cross-window navigation
- `test_nav_stack_cross_window_focus` — window focus restoration
- `test_nav_stack_scroll_position_restored` — scroll snapshot restoration
- `test_nav_stack_session_restore_no_entries` — session restore doesn't populate stack
- `test_nav_stack_forward_after_back` — dedicated forward navigation exercising

### 9. Debounce logic moved into NavigationStack (UI framework)
Moved scroll debounce state management from the Workspace into `NavigationStack<E>` itself:
- Added `pending: Option<E>`, `debounce_duration`, `last_debounced_push` fields
- `push_debounced()` captures the first entry in a scroll burst, tracks timestamp
- `flush()` commits any pending entry (called before navigate-back/forward, focus changes)
- `flush_if_expired()` commits only if the debounce window has elapsed
- Removed `scroll_nav_anchor`, `scroll_nav_debounce_generation`, `flush_scroll_nav_anchor()`, and timer spawning from Workspace
- Added feature flag guards on all `flush()` call sites and `is_navigating()` check in `PaneGroup::focus_pane`
- 7 new unit tests for the debounce API

### 10. Scroll navigation extended to code editor and notebook panes
Previously only terminal panes emitted `PaneUserScrolled` for scroll-based navigation. Extended to code editor and notebook panes:
- Generalized `PaneUserScrolled` event from `ScrollPosition` to `ScrollSnapshot`
- Terminal pane wraps its position in `ScrollSnapshot::Terminal(...)`
- Code editor: added `CodeEditorEvent::UserScrolled`, captures pre/post scroll snapshots in `ScrollVertical` handler, propagates through `CodeViewEvent` → `CodePane` → `PaneUserScrolled` with `ScrollSnapshot::Editor(...)`
- Notebook editor: added `EditorViewEvent::UserScrolled`, captures pre/post scroll snapshots in `RichTextEditorView::scroll()`, propagates through `NotebookEvent` → `NotebookPane` → `PaneUserScrolled`
- Workspace `handle_pane_user_scrolled` now accepts `ScrollSnapshot` directly instead of `ScrollPosition`

### 11. LSP go-to-definition records navigation entry
Added nav stack recording for LSP-initiated navigation (go-to-definition) in code editor panes:
- New `PaneLspNavigated` event on `PaneGroup` carries `pane_id` + pre-navigation `ScrollSnapshot`
- Before opening the definition target, the code editor captures the current scroll position and emits `CodeEditorEvent::LspNavigated`
- Propagated through `CodeViewEvent` → `CodePane` → `PaneGroup::Event::PaneLspNavigated` → Workspace
- Workspace handler flushes any pending debounced entry, then immediately pushes the pre-jump snapshot
- Added `test_nav_stack_code_editor_scroll` integration test

### 12. Nav stack extended to code review panel
The code review right panel is architecturally different from regular panes (it's a `RightPanelView`, not a `PaneGroup` pane). Extended nav stack to track scroll and LSP navigation in code review:
- Added `ScrollSnapshot::CodeReview { file_path, editor_snapshot }` variant
- Added `UserScrolled` / `LspNavigated` variants to `CodeReviewViewEvent`, forwarded through `RightPanelEvent`
- Workspace handles code review events with same debounce/flush pattern as regular panes
- Restoration routes through `RightPanelView` when the code review panel is open
- `pane_id` uses the focused terminal pane's ID (code review is always associated with one)

### 13. Fix panic when closing active tab
Closing the active tab (e.g. settings) with nav stack enabled caused a panic at `active_tab_pane_group().expect("Active tab index entry should exist")`.
- Root cause: `remove_tab` removed the tab from `self.tabs` but `active_tab_index` still pointed to the old (now out-of-bounds) index when `activate_tab_internal` was called
- `activate_tab_internal` saw `new_index != active_tab_index` → called `record_navigation_entry` → `active_tab_pane_group()` → panic
- Fix: set `self.active_tab_index` to the new target index before calling `activate_tab_internal`

## Current Status
- **Back/Forward navigation**: Working correctly at runtime for tab switches, pane focus, scroll, LSP jumps, and code review panel.
- **Scroll restoration**: Working — deferred via `pending_nav_scroll_restore` to avoid layout race condition.
- **Code review panel**: Scroll and LSP navigation entries recorded; restoration is best-effort (works when panel is open).
- **Known issue**: Scroll tracking in the diff view (code review) needs debugging — events may not be firing.

## Test Results (local, macOS)
All 24 tests pass (7 unit + 17 integration).

Original tests:
- `test_nav_stack_empty_on_startup` ✅
- `test_nav_stack_tab_switch_records_and_restores` ✅
- `test_nav_stack_new_action_clears_forward` ✅
- `test_nav_stack_feature_flag_gates_recording` ✅
- `test_nav_stack_multiple_back_forward` ✅

New tests:
- `test_nav_stack_pane_operations_no_entry` ✅
- `test_nav_stack_pane_focus_tracking` ✅
- `test_nav_stack_pane_focus_preserved_across_tabs` ✅
- `test_nav_stack_multi_window_isolation` ✅
- `test_nav_stack_cross_window_focus` ✅
- `test_nav_stack_scroll_position_restored` ✅
- `test_nav_stack_session_restore_no_entries` ✅
- `test_nav_stack_forward_after_back` ✅
- `test_nav_stack_pane_focus_scroll_captured` ✅
- `test_nav_stack_scroll_within_pane` ✅
- `test_nav_stack_code_editor_scroll` ✅
