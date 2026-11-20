# Tech Spec: Spaces — switchable sets of tabs within a window

**Issue:** TBD
**Product spec:** [product.md](product.md)

Line references are against `master` at `fdb20a15f` and will drift.

## Context

### Tabs and groups today

- `Workspace` (`app/src/workspace/view.rs:1047`) is window-scoped. It owns `tabs: Vec<TabData>` (1049), `active_tab_index` (1050), `tab_mru_order` (1053), and `tab_groups: HashMap<TabGroupId, TabGroup>` (1059).
- `TabData` (`app/src/tab.rs:353`) holds a `ViewHandle<PaneGroup>` plus `group_id: Option<TabGroupId>` (367). Group membership is a field on the tab, and the group itself lives in the workspace map.
- `TabGroup` / `TabGroupId(Uuid)` live in `app/src/workspace/tab_group.rs:27` / `:10`.
- Index-based code is pervasive: `view.rs` alone has **253** uses of `self.tabs` and **250** of `active_tab_index` across activation, cycling, closing, reordering, rendering, and drag.
- Activation entrypoints: `activate_tab` / `activate_tab_internal` (`view.rs:5500`, `5507`) and `activate_tab_by_pane_group_id` (`view.rs:5370`, used for jumps from outside the tab list).
- Close confirmation: `should_confirm_close_session` (`view.rs:12387`).
- `GroupedTabs` feature flag: `crates/warp_features/src/lib.rs:897`.

### UI

- Vertical tabs panel: `render_vertical_tabs_panel` (`app/src/workspace/view/vertical_tabs.rs:1671`), control bar (1400), row click → `WorkspaceAction::ActivateTab` (3315).
- "Move to group" submenu: `MOVE_TO_GROUP_LABEL` (`app/src/tab.rs:228`), built at `tab.rs:865` and `view/tab_grouping.rs:548`.
- Tab actions: `WorkspaceAction::{ActivateTab, ActivatePrevTab, ActivateNextTab}` (`app/src/workspace/action.rs:130`). Keybindings are registered as `EditableBinding`s in `app/src/workspace/mod.rs` (~555–640, e.g. `workspace:activate_eighth_tab` → `cmdorctrl-8`).
- macOS `ctrl-1`/`ctrl-2`/`ctrl-3` are taken by panel toggles (`app/src/util/bindings.rs:407–435`), and `ctrl-4` by Warp Drive (`workspace/mod.rs:833`).

### Persistence

- `WindowSnapshot` (`app/src/app_state.rs:45`) carries `tabs: Vec<TabSnapshot>` and `tab_groups: Vec<TabGroupSnapshot>` (63). It is built by `Workspace::snapshot` (`view.rs:11868`) and written by `save_app_state` (`app/src/persistence/sqlite.rs:917`).
- Tab groups were added by migration `crates/persistence/migrations/2026-06-01-000000_add_tab_groups/` (`tab_groups(id, window_id, name, color, collapsed)` + `tabs.tab_group_id`). Diesel models are in `crates/persistence/src/model.rs:360–389`.

### Input

- `Event::ScrollWheel { position, delta, precise, modifiers }` (`crates/warpui_core/src/event.rs:102`). On macOS, `delta` comes from `scrollingDeltaX/Y` and `precise` from `hasPreciseScrollingDeltas` (`crates/warpui/src/platform/mac/event.rs:240–250`). **There is no gesture phase** (`NSEvent.phase` / `momentumPhase`).

### Code outside `Workspace` that reads the live tab list

- Ctrl+Tab switcher: `app/src/search/command_palette/tabs/data_source.rs` (receives a tab list at :30, filters at :45).
- Cross-window drag: `app/src/workspace/cross_window_tab_drag.rs` (884, 910, 1454; and 1801, `workspace.tabs.clear()` on the drag-preview window).

## Proposed changes

### 1. Data model

New `app/src/workspace/space.rs`, mirroring `tab_group.rs`:

```rust
pub struct SpaceId(pub Uuid);
pub struct Space { pub id: SpaceId, pub name: Option<String>, pub color: SelectedTabColor }

/// Tab state of a space that is not currently shown.
struct StashedSpace {
    tabs: Vec<TabData>,
    active_tab_index: usize,
    tab_mru_order: Vec<EntityId>,
    tab_groups: HashMap<TabGroupId, TabGroup>,
}
```

New `Workspace` fields: `spaces: Vec<Space>` (display order), `active_space: SpaceId`, `stashed_spaces: HashMap<SpaceId, StashedSpace>`.

### 2. Switching by swapping, not filtering

`Workspace::switch_space(id)` moves the live `tabs`, `active_tab_index`, `tab_mru_order`, and `tab_groups` into `stashed_spaces[active_space]`, then moves the target's stash into the live fields.

Invariant: **the live fields always hold exactly the active space.** All existing index-based code (activation, cycling, close, reorder, render, drag) stays correct without change, because it never sees other spaces. `ViewHandle<PaneGroup>`s in the stash stay alive, so PTYs keep running (product invariant 6).

Switching into an empty stash opens a tab through the normal new-tab path (invariant 7).

### 3. Window-wide consumers

The swap leaves one class of code to change: code that needs **every** tab in the window, not just the visible ones. Add one accessor:

```rust
/// All tabs in the window: the active space first, then stashed spaces in display order.
pub(crate) fn all_tabs(&self) -> impl Iterator<Item = (SpaceId, &TabData)>
```

Callers to switch to it or to make space-aware:

| Consumer | Change |
|---|---|
| `Workspace::snapshot` (`view.rs:11868`) | Serialize every space, not just the live one. |
| `activate_tab_by_pane_group_id` (`view.rs:5370`) | Look up the tab's space in the stash and `switch_space` first (invariant 9). |
| `should_confirm_close_session` and the close-window path (`view.rs:12387`) | Count running sessions in stashed spaces too. |
| Ctrl+Tab data source (`search/command_palette/tabs/data_source.rs`) | Feed `all_tabs()` and add the space label (invariant 21). |
| Close-last-tab path | Before closing the window, check `stashed_spaces` for non-empty spaces (invariant 17). |
| `cross_window_tab_drag.rs` | Drop target inserts into the live (active) space (25). On the source side, the window is empty only if all stashes are empty too (26). Drag-preview windows always have a single space, so `tabs.clear()` at 1801 is safe. |

During implementation, every remaining `self.tabs` use outside `view.rs` gets audited with `rg '\.tabs\b' app/src` (~10 files today, most of them on `WindowSnapshot.tabs`, which is unaffected).

### 4. UI

- Switcher row: new `render_space_switcher` in `vertical_tabs.rs`, appended in `render_vertical_tabs_panel` and gated on `FeatureFlag::Spaces`. Chips reuse tab-group color rendering. `+` creates a space, and the chip context menu offers Rename / Change color / Delete, reusing `tab_group_rename_editor` and the group color menu.
- **Move to space ▸** submenu next to `MOVE_TO_GROUP_LABEL` in the tab, group, and multi-selection menus. Moving a group moves every tab with that `group_id`, plus the `TabGroup` entry.

### 5. Actions and keybindings

New `WorkspaceAction::{SwitchToSpace(usize), NextSpace, PrevSpace, NewSpace, MoveTabsToSpace(..), DeleteSpace(SpaceId), …}`. New `EditableBinding`s `workspace:switch_to_space_{1..9}`, `workspace:next_space`, and `workspace:prev_space`, all unbound by default pending the open question in product.md. All are exposed in the command palette.

### 6. Trackpad swipe

A pure, UI-free `SwipeDetector` in `space.rs`:

- feed it `(delta, precise, timestamp)`;
- ignore input unless `precise` and `|dx| > |dy|`;
- accumulate `dx` and emit `Next`/`Prev` once `|acc| > THRESHOLD`;
- then **lock until input pauses for `IDLE_GAP` (~200 ms)**. The momentum tail arrives as a continuous stream, so it is absorbed by the lock (invariant 10).

The vertical tabs panel's event handler routes `Event::ScrollWheel` into the detector only when the cursor is inside the panel and no drag, rename, or search-focus is active (11, 12).

Tradeoff: without a gesture phase this is a heuristic, and a very slow continuous swipe could be read as one long gesture. The upgrade path is adding `phase` / `momentum_phase` to `Event::ScrollWheel` (`warpui_core`, MIT crate; macOS fills it from `NSEvent`). That touches every match site of the event, so it is proposed as a follow-up, not part of v1.

### 7. Persistence

Migration `add_spaces`, mirroring `add_tab_groups`:

```sql
CREATE TABLE spaces (
  id INTEGER PRIMARY KEY NOT NULL,
  window_id INTEGER NOT NULL REFERENCES windows(id),
  name TEXT,
  color TEXT,
  position INTEGER NOT NULL
);
ALTER TABLE tabs ADD COLUMN space_id INTEGER REFERENCES spaces(id);
ALTER TABLE windows ADD COLUMN active_space_id INTEGER REFERENCES spaces(id);
```

`WindowSnapshot` gains `spaces: Vec<SpaceSnapshot>` and `active_space`, and `TabSnapshot` gains `space_id: Option<..>`. On restore, tabs are partitioned by `space_id`: the active space goes live, the others go to the stash, and unresolved or `NULL` values go to the first space (invariant 23). With the flag off, restore ignores `space_id` and loads all tabs live (24).

### 8. Feature flag

`FeatureFlag::Spaces` next to `GroupedTabs` in `crates/warp_features/src/lib.rs`. It depends on `VerticalTabs` for the switcher row; keybindings and palette commands work with horizontal tabs too.

## Alternatives considered

**`space_id` on every `TabData`, filter at render time.** Arguably the "truer" model, with one list and no stash. But every one of the ~500 index-based sites in `view.rs` would need to choose between "visible index" and "absolute index". That means a large diff in the most frequently edited file of the repo, and a long tail of off-by-one bugs (cycling, close-neighbor selection, drag insertion). The swap approach keeps those sites untouched and concentrates the change in `all_tabs()` and the ~6 consumers above. If reviewers prefer the filter model, `all_tabs()` is the seam to migrate behind.

**Space = window, with a better window switcher.** Smallest change, but it does not give "one window, many contexts". It overlaps #16305 rather than solving this problem.

## Testing and validation

| Invariants | Test |
|---|---|
| 10–13 | Unit tests for `SwipeDetector`: horizontal swipe → exactly one event; vertical scroll → none; `precise=false` → none; swipe + momentum tail → one; two swipes separated by `IDLE_GAP` → two. |
| 4–9, 14–18 | `Workspace` tests in the style of `app/src/workspace/view/vertical_tabs_tests.rs`: per-space active tab restored; new tab lands in active space; move tab / group / multi-selection; delete space moves tabs; close-last-tab switches instead of closing; `activate_tab_by_pane_group_id` switches space. |
| 20–21 | Search filters the live space only; Ctrl+Tab data source returns tabs from all spaces with labels. |
| 22–24 | Snapshot round-trip through SQLite with 3 spaces; restore with a dangling `space_id`; restore with the flag off loads every tab. |
| 25–26 | Integration test (`crates/integration`) for cross-window drag into a window with multiple spaces, and dragging out the last tab of a non-last space. |
| 1 | Existing test suite passes unchanged with the flag off. |

Migration `up`/`down` are exercised by the persistence tests. A manual-testing GIF (switching by chip, keybinding, and swipe; move to space; restart) is attached to the PR, as `CONTRIBUTING.md` requires. `./script/presubmit` must be green.

## Risks

- `view.rs` changes daily, so the branch needs frequent rebases. The swap design keeps our footprint in that file small.
- Hidden consumers of `self.tabs` that need window-wide state could be missed by the audit. Mitigation: grep audit plus the invariant tests above.
- Swipe heuristic tuning (`THRESHOLD`, `IDLE_GAP`) differs across trackpads. Both are constants and easy to adjust.

## Follow-ups

- Attention badges on chips for hidden spaces (finished command, agent awaiting input).
- Gesture phase on `Event::ScrollWheel`, for an exact swipe and finger-following animation.
- Per-space default directory or theme.
- Binding a space to a Project (#9233).
