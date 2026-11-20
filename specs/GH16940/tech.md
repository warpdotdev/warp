# Tech Spec: Spaces — switchable sets of tabs within a window

**Issue:** [warpdotdev/warp#16940](https://github.com/warpdotdev/warp/issues/16940)
**Product spec:** [product.md](product.md)

This spec describes the prototype on branch `spaces` at `527b6b92a` (based on `master` at `fdb20a15f`). Invariant numbers refer to product.md. Line references are against `527b6b92a`, approximate, and will drift.

## Context

### Tabs and groups

- `Workspace` (`app/src/workspace/view.rs:1051`) is window-scoped. It owns `tabs: Vec<TabData>` (1053), `active_tab_index` (1054), `tab_mru_order` (1057), and `tab_groups: HashMap<TabGroupId, TabGroup>` (1063).
- `TabData` (`app/src/tab.rs:366`) holds a `ViewHandle<PaneGroup>` plus `group_id: Option<TabGroupId>` (380). Group membership is a field on the tab, and the group itself lives in the workspace map. `TabGroup` / `TabGroupId(Uuid)` are in `app/src/workspace/tab_group.rs:27` / `:10`.
- Index-based code is pervasive: `view.rs` has 254 uses of `self.tabs` and 114 of `active_tab_index` (254 and 112 at `fdb20a15f`), across activation, cycling, closing, reordering, rendering, and drag.
- Activation: `activate_tab` / `activate_tab_internal` (`view.rs:5525`, `5532`). Jumps from outside the tab list go through `activate_tab_by_pane_group_id` (5407) and `focus_pane` (6056).
- Closing: `remove_tab` (12333), `should_confirm_close_session` (12436), `close_tab` (12554), and `remove_tab_without_undo` (28685, used by cross-window drag).
- Flags: `GroupedTabs` (`crates/warp_features/src/lib.rs:897`), `DragTabsToWindows` (176).

### UI

- Vertical tabs panel: `render_vertical_tabs_panel` (`app/src/workspace/view/vertical_tabs.rs:1713`), with per-panel state in `VerticalTabsPanelState` (699). The tab list is a `ClippedScrollable` around `render_groups`.
- "Move to group": `MOVE_TO_GROUP_LABEL` (`app/src/tab.rs:229`), built in `tab_group_menu_items` (`tab.rs:859`) and `view/tab_grouping.rs:548`. Its sidecar is filled by the workspace (`view.rs:10456`).
- Keybindings are `EditableBinding`s registered in `app/src/workspace/mod.rs`. On macOS `ctrl-1`/`ctrl-2`/`ctrl-3` are taken by panel toggles (`app/src/util/bindings.rs:409–431`) and `ctrl-4` by Warp Drive (`workspace/mod.rs:835`).

### Persistence

- `WindowSnapshot` (`app/src/app_state.rs:46`) carries `tabs: Vec<TabSnapshot>` and `tab_groups`. It is built by `Workspace::snapshot` (`view.rs:11911`) and written by `save_app_state` (`app/src/persistence/sqlite.rs:919`).
- Precedent: migration `2026-06-01-000000_add_tab_groups` (`tab_groups(id, window_id, name, color, collapsed)` + `tabs.tab_group_id`).

### Input

- At `fdb20a15f`, `Event::ScrollWheel { position, delta, precise, modifiers }` had no gesture phase, and `EventHandler::on_scroll_wheel` callbacks received only `delta` and `modifiers`.

### Code outside `Workspace` that reads tabs

- Ctrl+Tab switcher: `app/src/search/command_palette/tabs/data_source.rs`, fed by `Workspace::tab_navigation_data` (`view.rs:5417`) with `TabNavigationData` (`app/src/session_management.rs:237`).
- Cross-window drag: `app/src/workspace/cross_window_tab_drag.rs`, driven from `Workspace::on_tab_drag` (`view.rs:28902`).
- Lookups that locate a pane or session in any tab: `uri/mod.rs`, `pane_group/pane/terminal_pane.rs`, `ai/conversation_navigation/mod.rs`, `ai/agent_management/agent_management_model.rs`, `ai/blocklist/agent_view/orchestration_conversation_links.rs`, `undo_close/stack.rs`.

## Proposed changes

### 1. Feature flag

`FeatureFlag::Spaces` (`crates/warp_features/src/lib.rs:900`), behind the cargo feature `spaces` (`app/Cargo.toml`, `app/src/features.rs:469`) and listed in `DOGFOOD_FLAGS`. It is deliberately not in `RUNTIME_FEATURE_FLAGS`: turning it off at runtime would strand the hidden spaces' tabs. The switcher row needs `VerticalTabs`; keybindings, palette commands, and context menus also work with horizontal tabs. The cross-window parts (product 35–37) additionally need `DragTabsToWindows`, which is on in release builds and via the `drag_tabs_to_windows` cargo feature.

### 2. Data model

`app/src/workspace/space.rs`, mirroring `tab_group.rs`:

```rust
pub struct SpaceId(pub Uuid);
pub struct Space { pub id: SpaceId, pub name: Option<String>, pub color: SelectedTabColor }

/// Tab state of a hidden space, or of tabs moving between spaces.
pub(crate) struct StashedSpace {
    tabs: Vec<TabData>,
    active_tab_index: usize,
    tab_mru_order: Vec<EntityId>,
    tab_groups: HashMap<TabGroupId, TabGroup>,
}
```

`StashedSpace::append` adds another stash after its own tabs, keeping its active tab; pinned tabs join the end of the pinned prefix.

New `Workspace` fields (`view.rs:1065–1074`): `spaces: Vec<Space>` (switcher order, never empty), `active_space: SpaceId`, `stashed_spaces: HashMap<SpaceId, StashedSpace>` (a missing entry is an empty space), `space_swipe_detector`, and `space_chip_drop` (the chip a dragged tab or group is over). All space logic lives in `app/src/workspace/view/spaces.rs` as an `impl Workspace` block, like `tab_grouping.rs`.

### 3. Switching by swapping, not filtering

`switch_space` (`spaces.rs:221`):

1. clears UI state keyed by live tab index (tab/pane/group renames, tab/group/selection menus, the move-to-group sidecar, hover, detail sidecar, multi-selection);
2. swaps the live `tabs` / `active_tab_index` / `tab_mru_order` / `tab_groups` with the target's stash (`replace_live_tabs`) and stores the outgoing state under the old `active_space`;
3. starts the slide (section 8) when the vertical tabs panel is shown;
4. activates the space's remembered active tab through `activate_tab` (focus, MRU, window title, side panels), or, for an empty space, calls `add_terminal_tab(false)`. It does not use `AddDefaultTab`, because some default session modes (e.g. a tab config with parameters) open a modal that reads the active tab, which would panic on an empty live list (product 7).

Invariant: **the live fields always hold exactly the active space.** Existing index-based code (activation, cycling, close, reorder, render, drag) stays correct without change because it never sees other spaces. Across the whole series, `view.rs` changes by +265/−139 lines. `ViewHandle<PaneGroup>`s in a stash stay alive, so PTYs keep running (6). When a hidden tab's pane group exits (`pane_group::Event::Exited`, `view.rs:16397`), `remove_stashed_tab` (`spaces.rs:942`) detaches its panes and fixes the stash's active index, MRU, and orphaned group.

The other mutations reuse two primitives:

- `take_live_tabs(indices)` (`spaces.rs:662`) removes live tabs into a `StashedSpace`. A group goes with them only if every member moves; otherwise the moved members drop their `group_id` (invariant 3). MRU entries are split the same way.
- `append_to_space(space, stash)` appends to the live list or to a stash.

`move_tabs_to_space` (`spaces.rs:610`) and `delete_space` (538) are built from these. When the live list empties, `switch_to_nearest_non_empty_space` (322) picks the nearest space with tabs, ties going to the previous one (24).

### 4. Window-wide accessors and consumers

The swap leaves one class of code to change: code that needs every tab in the window, not just the visible ones. Accessors in `spaces.rs`:

| Accessor | Purpose |
|---|---|
| `all_tabs()` (60) | `(SpaceId, index_in_space, &TabData)` for every tab, in switcher order. |
| `all_tab_views()` (76) | Pane groups of every space. `tab_views()` deliberately stays live-only. |
| `all_tab_groups()` (136) | Groups of every space, for the snapshot. |
| `tabs_in_mru_order()` (84) | Ctrl+Tab order: active space by its MRU, then hidden spaces in switcher order by theirs. Space labels only with 2+ spaces. |
| `reveal_tab_by_pane_group_id()` (113) | Live index of a tab. For a hidden tab, makes it its space's active tab and calls `switch_space` first. |
| `hidden_spaces_have_tabs()` (153) | Whether the live list's last tab is really the window's last tab. |

Consumers changed:

| Consumer | Change |
|---|---|
| `Workspace::snapshot` (`view.rs:11911`) | Serializes `all_tabs()` and `all_tab_groups()`, ordered by `order_snapshot_tabs` (section 9). |
| `activate_tab_by_pane_group_id` (5407), `focus_pane` (6056), `pane_group::Event::FocusPaneGroup` | Go through `reveal_tab_by_pane_group_id` (9). Covers Ctrl+Tab, notification clicks, session and conversation navigation, deep links, undo-close. |
| Lookups that feed those jumps: `focus_terminal_view_locally` / `_in_other_window`, `find_pane_with_ambient_agent_conversation`, `get_pane_group_view_with_id`, URI session deep links and cloud-mode terminals (`uri/mod.rs`), the conversation palette (`ConversationNavigationData::all_conversations`, which showed hidden conversations as closed), orchestration links, agent-management notification metadata, child-agent owner/host lookups (`terminal_pane.rs`) | Search `all_tab_views()`. |
| Ctrl+Tab: `tab_navigation_data` (`view.rs:5417`), `data_source.rs`, `search_item.rs` | Uses `tabs_in_mru_order()`. `TabNavigationData.space_name` is `Some` only with 2+ spaces; the label becomes "title · Space · Tab N", `tab_index` is per space, and the query also matches the space name (29). |
| `workspace_sessions` (5491) | Uses `all_tab_views()`, so window close, quit, logout, and the navigation palette see hidden sessions (33). |
| `remove_tab`, `close_tab`, `should_confirm_close_session` | "Last tab" means `tabs.len() == 1 && !hidden_spaces_have_tabs()`. An emptied live list switches to the nearest space with tabs (24). |
| `on_window_closed` (28362), `handle_reopen`, undo-close discard | Detach, reattach, and clean up the panes of every space. Before this, hidden panes were never detached on close. |
| `on_tab_drag` (28902), `remove_tab_without_undo` (28685) | A single live tab with non-empty hidden spaces is not a whole-window drag: it detaches into a preview like a multi-tab drag, and the source shows a blank content area while the tab floats. On drop, `remove_tab_without_undo` switches to the nearest space with tabs (36). Putting the tab back on the source with only the placeholder live removes the placeholder in place before reinserting. Drops into another window land in its active space by construction (35). |

Left live-only on purpose, because they address "the visible tab": `local_control` (index-based API), synced inputs (`workspace/sync_inputs.rs`), legacy agent toasts (`window_and_tab_idx_id_for_conversation`, index-based; the HOA notifications path is space-aware), `terminal_view()` and auto cloud handoff on sleep (would retarget or focus hidden panes), `pane_group::Event::FocusPane`, `rename_pane`, `list_tab_pane_groups`, and vertical tabs rendering and search (28, 34).

### 5. UI

- **Switcher row:** `render_space_switcher` (`vertical_tabs.rs:1843`) is appended below the tab list's scrollable in `render_vertical_tabs_panel`, gated on `FeatureFlag::Spaces`. It is a wrapping row of chips (color dot + name) plus `+`. Chips use `Hoverable`: click dispatches `SwitchToSpaceById`, double-click `RenameSpace`, right-click `ToggleSpaceRightClickMenu`. Hover and drag states live in `VerticalTabsPanelState`, keyed by `SpaceId` and pruned on delete. `space_display_name` (1830) supplies the positional "Space N" placeholder.
- **Rename** reuses `tab_group_rename_editor`. The state is `WorkspaceState::space_being_renamed`, mutually exclusive with tab, pane, and group renames. `rename_space` is a no-op while the panel is hidden, since there is no chip to edit (26).
- **Chip menu** reuses `tab_right_click_menu`: **Rename space**, `color_picker_menu_items` with `ColorPickerTarget::Space`, and **Delete space** only while there are 2+ spaces.
- **Move to space ▸:** `MOVE_TO_SPACE_LABEL` (`tab.rs:232`) and `move_to_space_menu_items` (258) are added to the tab menu (`tab_group_menu_items`, which also serves horizontal tabs), the group menu (`tab_grouping.rs:550`), and the multi-selection menu. The sidecar reuses `move_to_group_sidecar_menu` and is filled by `build_move_to_space_sidecar_items` (`spaces.rs:775`).

### 6. Chip drag: reorder and drop-on-chip

**Reorder (27).** Each chip is a `Draggable` bounded to the switcher row (`SPACE_SWITCHER_POSITION_ID`) and saves its rect under `space_chip_position_id(space_id)`. `DragSpace { space_id, position }` calls `on_space_chip_drag` (`spaces.rs:360`), which moves the chip one slot when its center passes a neighbor's midpoint (`space_chip_drag_index`, 978, comparing positions in reading order so wrapped rows work). Placeholder names are positional, so an unnamed space that moves gets its current placeholder pinned as its name. `DropSpace` triggers a save through `should_save_app_state_on_action`.

**Drop a tab or group on a chip (23).** This is geometry-based: the cursor is tested against the saved chip rects. `on_tab_drag` and `on_group_drag` call `update_space_chip_drop` (417) first. Over a non-active chip, it records `space_chip_drop` (rendered as the hover background plus an accent border) and the drag skips reordering. `DropTab` (unless cross-window) and `DropGroup` call `drop_on_space_chip` (434), which runs `move_tab_to_space` / `move_tab_group_to_space`. Warp's `DropTarget` wasn't used because a dragged vertical-tab row spans the panel width, so its rect would overlap every chip at once rather than the one under the cursor.

### 7. Actions and keybindings

New `WorkspaceAction`s (`app/src/workspace/action.rs:250–300`): `SwitchToSpace(usize)` (1-based), `NextSpace`, `PrevSpace`, `NewSpace`, `SwitchToSpaceById`, `ToggleSpaceRightClickMenu`, `RenameSpace`, `RenameActiveSpace`, `ToggleSpaceColor`, `DeleteSpace`, `DeleteActiveSpace`, `MoveTabToSpace`, `MoveTabGroupToSpace`, `MoveSelectedTabsToSpace`, `DragSpace`, `DropSpace`, `SwipeSpaces`. The state-changing ones are listed in `should_save_app_state_on_action`; `SwipeSpaces` saves explicitly after a switch.

`EditableBinding`s (`workspace/mod.rs:1037–1090`): `workspace:switch_to_space_{1..9}`, `workspace:next_space`, `workspace:prev_space`, `workspace:new_space`, `workspace:rename_active_space`, `workspace:delete_active_space`. All are unbound by default (see Open questions in product.md), grouped under Navigation, enabled only with the flag, and listed in the command palette.

### 8. Trackpad swipe and slide

#### 8a. `ScrollPhase` on `Event::ScrollWheel` (`warpui_core`)

```rust
pub enum ScrollPhase { Began, Changed, Ended, Momentum }

Event::ScrollWheel {
    position: Vector2F,
    delta: Vector2F,
    precise: bool,
    /// None when the platform or device doesn't report one.
    phase: Option<ScrollPhase>,
    modifiers: ModifiersState,
}
```

- **macOS** (`scroll_phase`, `crates/warpui/src/platform/mac/event.rs:31`): a non-`None` `momentumPhase` → `Momentum`; `phase == Began` → `Began`; `Ended` or `Cancelled` → `Ended`; any other non-`None` phase (`Changed`, `Stationary`, `MayBegin`) → `Changed`; neither → `None` (mouse wheels).
- **winit (Linux, Windows), tests, and synthesized events** pass `None`.
- **`EventHandler::on_scroll_wheel`** (`crates/warpui_core/src/elements/gui/event_handler.rs:204`) callbacks now receive `(ctx, app, delta, precise, phase, modifiers)`; `precise` and `phase` are new. Also, a callback that returns `PropagateToParent` no longer forces the event to count as unhandled: the element returns its children's result. This only matters together with `with_always_handle`, and no existing caller combines the two.

Touched to compile (new field, `..` in exhaustive patterns, or the new callback arguments):

| Crate | Files |
|---|---|
| `warpui_core` | `event.rs` (enum, `Scale` impl), `elements/gui/event_handler.rs`, `scrollable.rs`, `new_scrollable/mod.rs`, `uniform_list.rs`, `scrollable_tests.rs`, `viewported_list_tests.rs` |
| `warpui` | `platform/mac/event.rs`, `windowing/winit/event_loop/mod.rs` |
| `editor` | `render/element/table.rs` |
| `app` | `on_scroll_wheel` callers: `workspace/view.rs` (Linux/Windows-only zoom handler), `ai/blocklist/block/view_impl/common.rs`, `ai/blocklist/inline_action/code_diff_view.rs`. Patterns: `editor/view/element.rs`, `terminal/alt_screen/alt_screen_element.rs`, `terminal/block_list_element.rs`, `terminal/share_block_modal.rs`, `terminal/waterfall_gap_element.rs`. |

`TuiEvent::ScrollWheel` is a separate type and is unchanged.

#### 8b. `SwipeDetector` (`app/src/workspace/swipe_detector.rs`)

A pure, UI-free state machine: `feed(dx, dy, precise, phase, now) -> Option<SwipeDirection>`.

- Non-precise input is ignored (16).
- A gesture starts on `Began` when a phase is present (14). Without one, it starts after a pause of at least `IDLE_GAP` = 200 ms since the previous precise event, in any direction, so a diagonal momentum tail can't end it early (15). Starting a gesture resets the accumulator and the "fired" flag.
- `Momentum` events never accumulate or fire (13). Nothing fires after the gesture has fired once. Vertical-dominant events (`|dx| <= |dy|`) are ignored (16).
- Otherwise `dx` accumulates (reversals subtract), and the detector fires once when `|acc| >= THRESHOLD` = 60 delta units: negative is `Next`, positive is `Prev`. `dx` is `scrollingDeltaX`, which already follows the system scroll-direction setting (18).

#### 8c. Routing

With the flag on, `render_vertical_tabs_panel` wraps the panel in an `EventHandler` with `with_always_handle` (`vertical_tabs.rs:1766`), because the tab list's scrollable consumes scroll events first. For every scroll over the panel it dispatches `SwipeSpaces { delta, precise, phase }` and propagates, so the list keeps scrolling. `handle_space_swipe` (`spaces.rs:277`) feeds the detector with `Instant::now()`. On a direction, unless `blocks_space_swipes` (305) is true, it calls `switch_to_adjacent_space` (no wrap) and saves. Blocking states (17) are a tab, group, or chip drag; any tab, pane, group, or space rename; and focus in the panel's search editor (`ViewHandle::is_focused`). Blocked events are still fed, so a swipe that fires while blocked is used up for the rest of its gesture.

#### 8d. Slide (`app/src/workspace/view/vertical_tabs/space_slide.rs`)

- `SpaceSlide { from: SlideFrom, started_at }` comes from `SpaceSlide::between(from_index, to_index)`: a higher target index enters from the right. `offset_at(width, now)` is `width * (1 - ease_out_cubic(t))` over `DURATION` = 200 ms, signed by side, and `None` once finished.
- `switch_space` calls `VerticalTabsPanelState::start_space_slide` when `uses_vertical_tabs` and the panel is open. A new switch overwrites the slide (11). Restore and `delete_space` don't go through `switch_space`, so they never slide (12).
- `SpaceSlideElement` wraps `render_groups` **inside** the list's `ClippedScrollable` (`vertical_tabs.rs:1724`). `ClippedScrollable` clips to its own rect (`ClipBounds::BoundedBy`), so shifting the scrollable itself would move the clip along with it and paint the list over the terminal. The element shifts only the child's paint origin. `origin()` / `bounds()` report the child's shifted geometry, so scroll-to-tab still finds rows mid-slide.
- Frames: while sliding, `paint` calls `ctx.repaint_after(8 ms)`, the mechanism `ShimmeringText` uses. A redraw re-lays-out and repaints the cached element tree without re-rendering views, and macOS coalesces redraws to the display refresh. Once finished, the element drops the slide, paints in place, and requests nothing, so an idle panel costs nothing.
- The slide is a pure translation. warpui has no layer opacity, so there is no fade.

### 9. Persistence

Migration `crates/persistence/migrations/2026-11-20-000000_add_spaces/`:

```sql
CREATE TABLE spaces (
  id INTEGER PRIMARY KEY NOT NULL,
  window_id INTEGER NOT NULL,
  name TEXT,
  color TEXT,
  position INTEGER NOT NULL,
  active_tab_index INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(window_id) REFERENCES windows(id)
);
ALTER TABLE tabs ADD COLUMN space_id INTEGER REFERENCES spaces(id);
ALTER TABLE windows ADD COLUMN active_space_index INTEGER;
```

- `windows.active_space_index` is an index into the window's spaces by `position`, like `windows.active_tab_index`. It is not a foreign key: `spaces.window_id → windows` plus `windows.active_space_id → spaces` would make diesel generate two conflicting `joinable!` declarations for the same table pair. `schema.rs` was edited by hand in `print-schema` layout.
- `spaces.active_tab_index` is per space, so hidden spaces keep their active tab across restarts (30).
- Models: `model::Space` / `NewSpace`; `active_space_index` on the window models; `space_id` on the tab models.

Snapshot (`app_state.rs`): `WindowSnapshot` gains `spaces: Vec<SpaceSnapshot { id, name, color, active_tab_index }>` and `active_space: Option<SpaceId>`; `TabSnapshot` gains `space_id: Option<SpaceId>`. With the flag off they are empty / `None`.

Ordering (`order_snapshot_tabs`, `spaces.rs:813`):

- `WindowSnapshot.tabs` holds every space's tabs. Effectively pinned tabs of **all** spaces come first, then the unpinned ones, in switcher order. Each space's own order is preserved, because pinned tabs already form a prefix within each space.
- `WindowSnapshot.active_tab_index` stays **global**: the index in `tabs` of the active space's active tab. A restore with the flag off therefore gets a valid pinned prefix and the right active tab, and existing readers of `tabs[active_tab_index]` (`has_horizontal_split`, the SQLite panel-width reads) are unchanged (32).
- `SpaceSnapshot.active_tab_index` is the index among that space's own saved tabs. If the active tab was left out of the snapshot (a tab transferred mid-drag), the next saved tab takes over.

Saving (`save_app_state`): spaces are inserted before the tabs that reference them, as tab groups are, and `SpaceId`s are mapped to row ids. On load, `SpaceId`s are minted fresh, like `TabGroupId`s.

Restore (`restore_spaces`, `spaces.rs:886`, called at `view.rs:4201`): the existing loop still restores every saved tab live, in snapshot order. Then the tabs are partitioned by `space_id` with `take_live_tabs` (groups whole, MRU split), and an unknown or `NULL` space maps to the first space (31). Each stash's active index is clamped, and the saved active space goes live. If it has no tabs, the nearest space with tabs is shown instead, without opening a new tab. If every space is empty, the existing zero-tab path opens a default tab. Empty spaces come back as chips. With the flag off, or with no saved spaces, it returns `None` and every tab stays in one list.

## Alternatives considered

**`space_id` on every `TabData`, filter at render time.** Arguably the "truer" model, with one list and no stash. But every one of the ~370 index-based uses in `view.rs` (see Context) would have to choose between "visible index" and "absolute index". That means a large diff in the most frequently edited file of the repo, and a long tail of off-by-one bugs (cycling, close-neighbor selection, drag insertion). The swap keeps those sites untouched and concentrates the change in the accessors of section 4. If reviewers prefer the filter model, `all_tabs()` is the seam to migrate behind.

**Space = window, with a better window switcher.** Smallest change, but it does not give "one window, many contexts". It overlaps #16305 rather than solving this problem.

**Phase-less swipe detection only.** The first version of the detector used only the 200 ms pause heuristic, to avoid touching `Event::ScrollWheel`. On a real trackpad, the momentum tail after a fast swipe streams without such a pause for up to about a second and a half, so a second swipe started during it merged into the gesture that had already fired and didn't switch. The gesture phase fixes this exactly; the heuristic remains the fallback where no phase is reported.

**`DropTarget` for chips.** See section 6.

**Shifting the scrollable or the whole panel for the slide.** This moves the clip rect with the content and paints over the terminal. See section 8d.

## Testing and validation

60 tests: 43 in `app/src/workspace/view/spaces_tests.rs`, 10 in `app/src/workspace/swipe_detector_tests.rs`, 5 in `app/src/workspace/view/vertical_tabs/space_slide_tests.rs`, and 2 in `app/src/persistence/sqlite_tests.rs`. Rendering tests drive real mouse events through the rendered panel (`drag_mouse` / `press_and_drag` helpers in `spaces_tests.rs`). Tests that click or drag rows right after a switch made with the panel open call `finish_space_slide` first, because rows are painted shifted while sliding.

| Invariants | Tests |
|---|---|
| 1 | `test_space_actions_are_noops_with_flag_disabled`, `test_space_management_actions_are_noops_with_flag_disabled`, `test_restore_with_spaces_disabled_loads_every_tab_in_one_list`; existing suite with the flag off |
| 2–7 | `test_switch_space_restores_each_spaces_active_tab` (per-space active tab, empty space opens a tab, hidden tabs kept), `test_remove_stashed_tab_keeps_hidden_space_consistent`, `test_last_space_cannot_be_deleted`, `test_move_tab_group_to_space_moves_members_and_group` |
| 8 | `test_next_and_prev_space_do_not_wrap`, `test_switch_to_out_of_range_space_is_noop` |
| 9 | `test_jumps_to_a_tab_in_a_hidden_space_switch_to_that_space` (`activate_tab_by_pane_group_id` and `focus_pane`) |
| 10–12 | `test_switching_spaces_slides_the_tab_list_in_from_the_target_side` (every switch path; restart mid-slide), `test_switching_spaces_without_the_vertical_tabs_panel_does_not_slide`, `test_restoring_spaces_does_not_slide`, `test_sliding_tab_list_is_painted_beside_its_resting_place`; `higher_index_slides_in_from_the_right_lower_from_the_left`, `ease_out_cubic_starts_fast_and_lands_at_one`, `offset_starts_a_full_width_out_on_the_entry_side`, `offset_shrinks_monotonically_toward_zero`, `slide_is_over_after_its_duration` |
| 13–15 | `fingers_left_switches_to_next_space_once`, `fingers_right_switches_to_previous_space_once`, `momentum_tail_does_not_fire_again`, `second_swipe_during_momentum_switches_again`, `momentum_does_not_complete_a_short_swipe`, `a_pause_starts_a_new_gesture`, `small_movements_do_not_add_up_across_gestures`, `without_phase_info_a_swipe_during_the_tail_stays_in_the_same_gesture`; routing: `test_swipe_switches_to_adjacent_space_once_per_gesture` |
| 16 | `vertical_scrolling_never_switches`, `mouse_wheel_never_switches`, `test_vertical_and_wheel_scrolls_do_not_switch_spaces` |
| 17 | `test_swipe_is_ignored_while_dragging_renaming_or_searching` |
| 18 | Manual (direction comes from the platform's deltas); no-wrap via `test_swipe_switches_to_adjacent_space_once_per_gesture` |
| 19 | `test_new_tab_lands_in_active_space` |
| 20–22 | `test_move_tab_to_space_keeps_active_space`, `test_move_active_tab_to_new_space_activates_neighbor`, `test_move_tab_group_to_space_moves_members_and_group`, `test_move_selection_to_space_keeps_whole_groups_together` |
| 23 | `test_dropping_a_tab_on_a_space_chip_moves_it_to_that_space` (including "active chip is not a target"), `test_dropping_a_tab_group_on_a_space_chip_moves_the_group` |
| 24 | `test_moving_every_tab_away_switches_to_target_space`, `test_closing_last_tab_of_space_switches_to_previous_non_empty_space`, `test_closing_last_tab_of_space_skips_empty_neighbors`, `test_closing_last_tab_when_other_spaces_are_empty_closes_window` |
| 25 | `test_delete_active_space_moves_tabs_to_previous_space`, `test_delete_first_space_moves_tabs_to_next_space`, `test_last_space_cannot_be_deleted` |
| 26 | `test_rename_and_recolor_space`, `test_space_chip_click_double_click_and_right_click_still_work` |
| 27 | `test_space_chip_drag_index_follows_neighbor_midpoints`, `test_dragging_a_space_chip_reorders_spaces` |
| 28 | `test_vertical_tabs_search_stays_within_active_space` |
| 29 | `test_ctrl_tab_lists_tabs_of_every_space_with_space_labels` (also: no labels with one space) |
| 30–32 | `test_sqlite_round_trips_spaces`, `test_spaces_round_trip_from_workspace_through_sqlite`, `test_restore_puts_tabs_of_unknown_spaces_in_first_space`, `test_restore_without_space_data_keeps_every_tab_in_one_space`, `test_restore_shows_nearest_space_with_tabs_when_active_space_has_none`, `test_restore_with_spaces_disabled_loads_every_tab_in_one_list` |
| 33 | `test_window_sessions_include_hidden_spaces` |
| 34 | By construction (those APIs read only the live list); no dedicated test |
| 35–37 | `test_tab_dropped_into_another_window_joins_its_active_space`, `test_dragging_out_last_tab_of_space_keeps_window_and_switches_space`. These exercise the attach and detach paths directly; a real cross-window drag needs window bounds, which unit tests don't have, so the full drag is verified manually with `DragTabsToWindows` on. |
| Regression | `test_dragging_a_vertical_tab_reorders_tabs` (flag off/on × cross-window drag off/on), `test_dragging_a_vertical_tab_reorders_tabs_after_switching_spaces` |

The migration's `up` runs in the persistence tests; `down` mirrors `add_tab_groups` and was checked by hand with the `sqlite3` CLI. A manual-testing GIF (switching by chip, keybinding, and swipe; move to space; drag to chip; reorder; restart) is attached to the PR, as `CONTRIBUTING.md` requires. `./script/presubmit` must be green.

## Risks

- `view.rs` changes daily, so the branch needs frequent rebases. The swap keeps the footprint there small (+265/−139).
- A hidden consumer of `self.tabs` that needs window-wide state could still be missed. Mitigation: the audit behind section 4 plus the invariant tests.
- **Non-macOS builds.** The `Event::ScrollWheel` change touches winit code paths and a Linux/Windows-only handler in `view.rs` that were not compiled while the prototype was developed on macOS. Linux and Windows CI must confirm them. On those platforms `phase` is always `None`, so swipes use the pause heuristic (15).
- Swipe tuning (`THRESHOLD`, `IDLE_GAP`) may differ across trackpads. Both are constants.
- **Mid-slide hit-testing.** For 200 ms after a switch, rows are painted, and receive clicks, at their shifted positions. A click during a slide lands on whatever row is painted under the cursor at that frame.
- **`reverse_handoff`.** In the rare `InsertedInTarget` escape hatch of cross-window drag, removing the caller's last live tab switches spaces in the middle of the drag.
- **Launch configs flatten spaces.** A launch config saved from a window with spaces contains every space's tabs without space information; opening it puts them all in the active space.
- **Synced inputs.** Sync events, including `StopSyncing`, go only to visible tabs (`tab_views()`), so a tab that was synced and then hidden can keep its synced-input state after syncing is turned off.
- **Drag edge cases.** A window's only tab, with no tabs in other spaces, starts a whole-window drag on the first move, so it can't be dropped on a chip (37). While a cross-window drag is active, chips in the source window are not drop targets.
- **Dogfood exposure.** The flag is in `DOGFOOD_FLAGS`; maintainers may prefer it off by default until the open questions are settled.

## Follow-ups

- **Finger-following swipe.** With `phase` available, the tab list could track the fingers during `Changed` and settle or snap back on `Ended`, instead of switching at the threshold and then sliding.
- **Attention badges** on chips for hidden spaces (finished command, agent awaiting input).
- **Per-space default directory or theme.**
- **Reduce motion.** Needs a Warp or warpui preference first; none exists today.
- **Gesture phase on Linux and Windows** (winit), where `phase` is `None` today.
- **Chip accessibility:** expose name and active state, and announce switches.
- **Launch configs** that keep space assignments.
- **Synced inputs** that are space-aware, or at least stop syncing in hidden tabs.
- Binding a space to a Project (#9233).
