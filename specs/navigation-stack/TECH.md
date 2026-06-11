# Navigation Stack (Tech Spec)

This document describes the implementation currently on `zach/navigation-stack` so the checked-in technical spec matches the code on this branch rather than a speculative future plan.

## 1. Problem

Warp needs browser/IDE-style back and forward navigation across workspace context, not just across files. The implementation on this branch solves that by recording the workspace state the user is leaving — window, tab, pane, and optional scroll state — and restoring that state later across terminals, editors, notebooks, code diffs, and the code review panel.

The core technical challenges are:

- keep the stack generic enough to live in `warpui`, while still carrying Warp-specific state at the app layer
- record only user-meaningful transitions and suppress system-driven restores and session bootstrapping
- support debounced scroll history without spamming the stack
- restore entries across windows and temporarily closed tabs/panes/windows
- prune stale entries when undo-close state expires so back/forward never gets stuck on dead targets

## 2. Relevant Code

- `ui/src/navigation.rs (1-203)` — generic `NavigationStack<E>` model with back/forward vectors, debounced pending entry support, `retain`, and the `is_navigating` guard
- `app/src/workspace/nav_stack.rs (1-75)` — Warp-specific `NavigationEntry` and `ScrollSnapshot` variants (`Terminal`, `Editor`, `CodeDiff`, `CodeReview`)
- `app/src/workspace/mod.rs:82` — workspace initialization registers the singleton nav stack model
- `app/src/workspace/mod.rs (807-833)` — editable bindings for `Go Back`, `Go Forward`, and `Clear Navigation Stack`
- `warp_core/src/features.rs:725` — `FeatureFlag::NavigationStack`
- `warp_core/src/features.rs:827` — dogfood rollout wiring for the feature flag
- `app/src/workspace/tab_settings.rs:197` — persisted `show_navigation_buttons` tab setting, defaulting to `true`
- `app/src/settings_view/features_page.rs (537-542)` — settings toggle for tab-bar navigation buttons
- `app/src/workspace/view.rs (7623-8006)` — navigation entry construction, record paths, stack traversal, and restore logic
- `app/src/workspace/view.rs:3713` — `activate_tab_internal`, which flushes pending scroll state and records tab-switch entries
- `app/src/workspace/view.rs:4378` — right-panel event handling for code review scroll/LSP navigation
- `app/src/workspace/view.rs:11211` and `app/src/workspace/view.rs:11229` — pane focus and pane LSP events flowing into workspace-level navigation recording
- `app/src/workspace/view.rs:13582` — window focus change handling records cross-window history
- `app/src/workspace/view.rs (14395-14760)` — tab bar back/forward button rendering and disabled state
- `app/src/workspace/view.rs (18476-18584)` — `WorkspaceAction` handlers for `NavigateBack`, `NavigateForward`, and `ClearNavigationStack`
- `app/src/root_view.rs (1155-1175)` — opening a new window records the prior active workspace state before focus changes
- `app/src/pane_group/pane/mod.rs (635-642)` — pane-level `scroll_snapshot` / `restore_scroll` extension points
- `app/src/pane_group/pane/terminal_pane.rs (499-509)` and `app/src/pane_group/pane/terminal_pane.rs:1129` — terminal scroll snapshot support and user-scroll event emission
- `app/src/pane_group/pane/code_pane.rs (162-172)` and `app/src/pane_group/pane/code_pane.rs (255-280)` — code editor user-scroll/LSP events plus snapshot/restore
- `app/src/pane_group/pane/notebook_pane.rs (171-183)` and `app/src/pane_group/pane/notebook_pane.rs:274` — notebook snapshot/restore and user-scroll event emission
- `app/src/pane_group/pane/code_diff_pane.rs (164-193)` — code diff snapshot/restore with selected tab and diff view identity
- `app/src/code_review/code_review_view.rs:5958` and `app/src/code_review/code_review_view.rs:5966` — code review panel scroll snapshot and restore
- `app/src/code_review/code_review_view.rs (7109-7114)` — code review list scroll emits user-scroll events
- `app/src/workspace/view/right_panel.rs (1012-1029)` — right panel relays code review scroll/LSP events back to workspace
- `app/src/pane_group/mod.rs:4048` — close-pane behavior and nav stack pruning for permanently removed panes
- `app/src/pane_group/mod.rs:4724` and `app/src/pane_group/mod.rs:4752` — closed-pane cleanup and restoration
- `app/src/pane_group/mod.rs:6089` and `app/src/pane_group/mod.rs:6148` — pane focus behavior and history updates
- `app/src/undo_close/stack.rs (236-303)` — closed window/tab presence checks and restoration lookups used by navigation restore
- `ui/src/navigation_tests.rs:132` and `ui/src/navigation_tests.rs:240` — unit coverage for debounce and retain/pruning semantics
- `app/src/workspace/nav_stack_tests.rs:88` and `app/src/workspace/nav_stack_tests.rs:118` — workspace entry dedupe and back/forward behavior
- `integration/src/test/navigation_stack.rs:889`, `integration/src/test/navigation_stack.rs:1220`, `integration/src/test/navigation_stack.rs:1347`, `integration/src/test/navigation_stack.rs:1381`, `integration/src/test/navigation_stack.rs:1432`, `integration/src/test/navigation_stack.rs:1477` — integration coverage for cross-window restore, code editor focus/scroll restore, closed pane/tab/window restore, and stale-entry pruning

## 3. Current State

### Generic stack model

The reusable stack lives in `warpui` as `NavigationStack<E>`. It owns:

- `back` and `forward` vectors
- a single `pending` debounced entry
- `is_navigating` to suppress recursive recording during restore
- a fixed debounce duration of 1.5s
- a max stack size of 100 entries

The debounce model intentionally keeps the first entry in a scroll burst. Repeated scroll events within the debounce window update only the timestamp, not the stored pending entry, so navigating back returns to the pre-scroll anchor rather than an intermediate point.

### Warp-specific entry shape

Warp’s app-layer entry is `workspace::nav_stack::NavigationEntry`:

- `window_id`
- `tab_index`
- `pane_id`
- optional `scroll_snapshot`

`ScrollSnapshot` is currently one of:

- terminal viewport position
- generic editor viewport snapshot
- code diff snapshot containing diff view ID, selected editor tab, and editor scroll state
- code review list position using `(scroll_index, scroll_offset_px)`

Consecutive duplicate entries are deduplicated via `NavigationEntry::should_push`.

### Feature and UI wiring

The branch already wires the feature through all intended user entry points:

- `FeatureFlag::NavigationStack` is defined and enabled for dogfood builds
- `workspace:navigate_back`, `workspace:navigate_forward`, and `workspace:clear_navigation_stack` are registered as editable bindings
- a `show_navigation_buttons` tab setting is persisted and defaults to `true`
- Settings > Features exposes a toggle for the tab-bar buttons
- the horizontal tab bar renders chevron buttons when both the feature flag and setting are enabled

This means the branch implements one extra user-facing affordance beyond the product spec: a command-palette-accessible `Clear Navigation Stack` action.

Note for OSS checkouts: the default `cargo run` launches the `warp-oss` binary, which applies only `DEBUG_FLAGS` — not `DOGFOOD_FLAGS` — so `FeatureFlag::NavigationStack` is off. Use `cargo run --features navigation_stack` (compile-time enablement via `app/src/features.rs`) or a channel binary that applies dogfood flags (`warp`/`dev`).

### Recording flow

The workspace owns all record-time decisions. The current branch records history from these paths:

- tab changes via `activate_tab_internal`
- pane focus changes via `pane_group::Event::PaneFocused`
- window focus loss via `handle_window_state_change`
- opening a new window via `root_view::open_new_window_get_handles`
- pane user scroll via `PaneUserScrolled`
- pane LSP navigation via `PaneLspNavigated`
- code review panel user scroll and code review LSP navigation through the right panel event bridge

`record_navigation_entry` records the currently focused pane, while `record_navigation_entry_for_pane` is used when focus changes and the departing pane must be recorded explicitly. Scroll-driven entries use the debounced `pending` path.

During session restore, the workspace sets `is_navigating = true` around reconstruction so restored tabs, panes, and focus changes do not backfill history.

### Restore flow

Back/forward restoration is centralized in `Workspace::navigate_stack_history` and `Workspace::restore_navigation_entry`.

The flow is:

1. Flush any pending debounced scroll entry.
2. Build a “current” entry representing where the user is now.
3. Peek the destination candidate from the back or forward stack.
4. Skip and discard stale candidates until one is restorable.
5. Move the current entry onto the opposite stack via `go_back` / `go_forward`.
6. Restore the destination locally or in another window.

For the code review panel, `build_current_navigation_entry_for_stack_navigation` overrides the current scroll snapshot with a `CodeReview` snapshot when the right panel is open so forward navigation preserves right-panel context.

### Pane adapters

The pane system exposes navigation through `PaneContent::scroll_snapshot` and `PaneContent::restore_scroll`.

Current implementations:

- terminal panes snapshot `ScrollPosition` and restore via `set_pending_nav_scroll_restore`
- code panes snapshot the active editor render state and restore it directly
- notebook panes use the notebook editor render state in the same `Editor` snapshot form
- code diff panes snapshot selected editor tab + diff view ID + editor scroll

The code review panel is not a pane, so it bypasses `PaneContent` and uses a separate right-panel event bridge plus a distinct `ScrollSnapshot::CodeReview` variant.

### Closed-target restoration and pruning

The branch integrates navigation with undo-close and cleanup:

- if a destination window is closed but still present in `UndoCloseStack`, navigation reopens it
- if a destination tab is closed but still undoable, navigation restores it by tab index
- if a destination pane is hidden-for-close, navigation calls `restore_closed_pane`
- if a code diff pane was temporarily closed, navigation can reopen the underlying `CodeDiffView` using the stored `view_id`
- when panes are permanently cleaned up or tabs are removed without keeping undo-close state, the nav stack uses `retain` to prune dead entries

This pruning model keeps navigation from getting stuck on stale history after the undo-close grace period expires.

### Current limitations that matter

- Entries identify tabs by `tab_index`, not a stable tab ID. That is workable with the current undo-close model and stale-entry discard loop, but it is still more fragile than stable IDs when tab order changes.
- Code review navigation is intentionally special-cased outside `PaneContent`, so the right panel follows a different data path from regular panes.
- Notebook, code diff, and code review restoration rely on their own view-layer restore methods for best-effort clamping; the workspace layer does not normalize invalid scroll positions itself.
- The branch contains good automated coverage for terminals, tabs, panes, windows, undo-close restore, and code editor focus/scroll, but less integration coverage for notebook, code diff, and code review panel flows.

## 4. Proposed Changes

This branch implements the feature with the following architecture.

### 4.1 Keep the stack model generic in `warpui`

`ui::navigation::NavigationStack<E>` stays application-agnostic and only owns generic stack behavior:

- push / back / forward semantics
- pending debounced entry handling
- stale entry pruning via `retain`
- recursion suppression via `is_navigating`

This keeps the reuse boundary clean: Warp-specific state is encoded only in `NavigationEntry`.

### 4.2 Make the workspace the single owner of navigation policy

The workspace, not the panes, decides:

- when to record
- what the “current” entry is
- whether a candidate is restorable
- how to reopen missing windows/tabs/panes
- when to discard stale destinations

That matches the existing ownership boundaries in the repo: window, tab, pane-group, and panel coordination already live in `Workspace`, so navigation restore naturally belongs there too.

### 4.3 Treat scroll history as “the place the user left”

All scroll-capable surfaces emit a pre-scroll snapshot. The stack stores the first snapshot in a burst and flushes it:

- when the debounce expires
- immediately before any focus-changing event
- immediately before Back or Forward runs

This gives the branch IDE-style “return me to where I started scrolling” behavior rather than “return me to an arbitrary intermediate offset.”

### 4.4 Use pane-local adapters for normal panes and a dedicated path for the right panel

The current branch splits restoration into two paths:

- normal panes: `PaneContent::scroll_snapshot` / `restore_scroll`
- code review panel: right-panel event bridge + `ScrollSnapshot::CodeReview`

That is the right fit for the codebase today because the code review surface is not modeled as a pane and already flows through `RightPanelView`.

### 4.5 Restore closed targets through existing undo-close mechanisms

The branch deliberately does not invent a second restoration store for navigation. Instead it reuses:

- `UndoCloseStack` for windows and tabs
- hidden-for-close pane state inside `PaneGroup`
- live `CodeDiffView` identity for temporary code diff panes

This reduces duplicate ownership and keeps navigation aligned with the repo’s existing “temporarily closed but still restorable” model.

### 4.6 Expose the feature through standard Warp bindings and settings

The branch follows existing Warp patterns for user-facing affordances:

- editable bindings for keyboard shortcuts and command palette discoverability
- a persisted tab setting for button visibility
- button rendering inside the existing tab bar layout (added to the shared left-toolbar row in `render_tab_bar_contents` before the vertical-tabs branch, so the buttons render in both horizontal and vertical layouts)
- feature-flag gating at the binding, setting, and rendering layers

### 4.7 Fixes required by computer-use verification

End-to-end verification of the ported branch surfaced gaps whose required implementations are recorded here.

**Cross-window forward preservation.** `restore_navigation_entry` raises the destination window and then clears `is_navigating`. The departing window's focus-loss `StateEvent` arrives asynchronously after that reset, so `handle_window_state_change` recorded a fresh entry and truncated the forward stack (integration tests deliver the event synchronously inside the guard, masking this). Fix: `NavigationStack` carries an `expected_focus_loss: Option<WindowId>`. The cross-window restore path calls `expect_focus_loss(current_window)` before raising the destination window; `handle_window_state_change` consumes the expectation via `take_expected_focus_loss(window_id)` and skips flush/record for that one focus loss. The expectation is cleared by any subsequent `push` or `clear` so a stale expectation cannot suppress a later legitimate record.

**Closed-window restore.** `navigation_workspace_for_window` can return a workspace whose platform window is already closed (registry staleness in the async world). The restore path then "restores" into the zombie workspace, never reaching the undo-close reopen path, and even when reopening it skipped `Workspace::handle_reopen`, leaving panes detached. Fix: both `navigation_entry_can_restore` and `restore_navigation_entry` treat a registry hit as valid only when `ctx.is_window_open(window_id)`; otherwise they fall through to `UndoCloseStack::take_closed_window` + `ctx.reopen_closed_window`, followed by `handle_reopen` to reattach panes — mirroring `UndoCloseStack::undo_close`.

**Editor caret co-location.** `ScrollPositionSnapshot` already anchors on `first_character_offset`; restoring scroll without moving the caret meant the first caret-relative keystroke autoscrolled back to the stale caret. Fix: `CodeEditorView::restore_scroll_position_with_caret` sets the selection cursor to the snapshot's first character offset and then applies `scroll_to`. The code-pane `restore_scroll` adapter and `CodeDiffView::restore_selected_editor_scroll` use this method.

**Keybinding non-interference.** The `workspace:navigate_back` / `workspace:navigate_forward` bindings add `& !id!("LongRunningCommand")` to their context predicates so a focused foreground program receives Alt+Left/Right. Typing in the input editor intentionally keeps IDE precedence.

**Discoverability.** `render_tab_bar_icon_button` attaches the tooltip in the disabled branch as well, and gains an `emphasized` parameter so the nav chevrons use `main_text_color` when enabled (matching neighboring toolbar icons) instead of the muted sub-text color. Tooltips read "Go back" / "Go forward" to match the palette/binding names. The palette toggle label is made state-aware by inserting `flags::SHOW_NAVIGATION_BUTTONS_FLAG` into the workspace keymap context when the setting is on (the `ToggleSettingActionPair` enable/disable split keys off that context flag). The settings row gains descriptive subtext.

**Palette search keywords.** `BindingDescription` carries search-only `search_keywords` (builder `with_search_keywords`, preserved through `materialized`). Both palette action searchers honor them: the fuzzy searcher falls back to keyword matching when the description does not match (returning the best keyword score with no highlight indices), and the full-text searcher appends keywords to the indexed text while filtering highlight indices past the rendered description length. The three navigation bindings attach `navigate` / `navigation` / `history` keywords so synonym queries surface `Go Back`, `Go Forward`, and `Clear Navigation Stack`. Covered by `app/src/search/action/data_source_tests.rs`.

**Minimum scroll delta.** Near-duplicate detection (`NavigationEntry::is_near_duplicate_of`, threshold 8 lines for terminal snapshots via `ScrollPosition::is_within_lines`; exact comparison for editor/code-diff/code-review snapshots) is applied in two places. `should_push` dedupes consecutive anchors at record time, and — critically — `navigate_stack_history` discards Back/Forward candidates that are near-duplicates of the user's *live* current entry before restoring. The traversal-time check is what prevents the net-zero-twitch case: the twitch anchor is far from the previous stack entry (so record-time dedupe cannot catch it) but imperceptibly close to where the user actually is.

## 5. End-to-End Flow

### Representative flow: user scrolls, switches context, then navigates back

1. The user scrolls in a terminal, code editor, notebook, or code review panel.
2. The surface emits a pre-scroll snapshot:
   - pane surfaces emit `PaneUserScrolled`
   - code review emits `CodeReviewViewEvent::UserScrolled`, which is relayed through `RightPanelView`
3. `Workspace` wraps that snapshot in a `NavigationEntry` and stores it as the debounced pending entry.
4. The user switches tabs, focuses another pane, opens a new window, or invokes Back/Forward.
5. The workspace flushes the pending entry before recording the focus change or traversing history.
6. When Back runs, the workspace:
   - builds the current entry
   - peeks the destination
   - discards stale entries until it finds a restorable one
   - moves the current entry onto the forward stack
   - restores the destination
7. Restore may reopen a closed window, closed tab, hidden pane, or code diff surface before applying scroll restoration.
8. The workspace sets `is_navigating` during the restore so the focus and scroll work needed to land on the destination does not create a second synthetic history entry.

### Representative flow: code review panel LSP navigation

1. The user navigates from the code review panel into another file via LSP.
2. `CodeReviewView` emits `LspNavigated` with the pre-navigation list position.
3. `RightPanelView` relays that to `Workspace`.
4. The workspace flushes any pending scroll anchor, pushes the code review position immediately, and later restores it by reopening the right panel and calling `restore_scroll_position`.

## 6. Risks and Mitigations

### Tab-index identity can drift

Entries currently use `tab_index`, so close/reorder behavior depends on undo-close restoration and stale-entry pruning rather than on a stable tab identifier.

Mitigation:

- the restore loop validates every candidate before consuming it
- closed tabs can be restored by index while still undoable
- permanently stale entries are pruned or discarded instead of breaking navigation

Follow-up if this feature expands further: move from tab indices to a stable tab identity.

### Recursive recording during restore

Restoring an entry necessarily changes focus and scroll state, which could recursively generate new history.

Mitigation:

- `NavigationStack::is_navigating`
- explicit `set_navigating(true/false)` around restore and session reconstruction
- pane focus events skip `PaneFocused` emission while navigating

### Scroll restoration quality depends on per-surface adapters

The workspace only routes snapshots. Exact restore behavior lives in the terminal/code/notebook/code diff/code review surfaces.

Mitigation:

- a common `PaneContent` adapter API for normal panes
- a dedicated restore API for the code review panel
- automated coverage for terminal and code editor restoration
- manual validation for notebook, code diff, and code review cases

### Code review is a special case

The right panel is not a pane, so its navigation behavior is more bespoke than the rest of the system.

Mitigation:

- isolate the special case to `ScrollSnapshot::CodeReview`
- keep the event bridge narrow (`CodeReviewView` → `RightPanelView` → `Workspace`)
- let `Workspace` continue to own stack traversal and stale-entry handling so only snapshot capture/restore differs

## 7. Testing and Validation

### Unit coverage

- `ui/src/navigation_tests.rs` covers generic stack semantics, including max stack size, debounce behavior, and `retain`
- `app/src/workspace/nav_stack_tests.rs` covers Warp entry semantics such as dedupe and back/forward behavior with workspace entries

### Integration coverage already on this branch

`integration/src/test/navigation_stack.rs` already covers:

- empty stack on startup
- feature-flag gating
- keybindings, command palette actions, and tab-bar buttons
- forward-stack clearing after new navigation
- pane focus tracking
- cross-window back/forward behavior
- terminal scroll restore
- code editor focus and scroll restore
- restoration of recently closed panes, tabs, and windows
- stale-entry pruning after undo-close expiry

### Manual validation still warranted

Because automated coverage is currently lighter for some specialized surfaces, manual validation should still cover:

- notebook scroll restoration
- code diff reopen + selected-editor-tab restoration
- code review panel scroll and LSP return flow
- best-effort restoration after content changes invalidate an exact saved position

## 8. Follow-ups

- Replace `tab_index`-based identity with a stable tab identifier if future navigation features need stronger reordering guarantees.
- Add targeted integration coverage for notebook, code diff, and code review restoration.
- Remove any temporary navigation debug instrumentation before shipping if it is no longer needed for branch debugging.
- Add an onboarding/changelog affordance introducing the feature (deferred from this release).
- Consider caret co-location for notebook scroll restoration (currently editor panes and code diffs only).
