# APP-3743: Unified New Tab Menu — Tech Spec

## Problem

The horizontal tab bar's chevron menu and the vertical tab bar's `+` menu showed different items. This change unifies them into a single menu with a "Worktree in" submenu-parent item that shows a sidecar panel on hover, following the proven model picker pattern. The worktree sidecar now includes a scrollable search row at the top of its content, live repo filtering, and a pinned footer for "Add new repo". On Windows, Terminal also gets a sidecar for shell selection; on other platforms it's a regular menu item. Clicking a repo in the "Worktree in" sidecar creates a worktree immediately via a default tab config.

## Relevant Code

- `app/src/menu.rs` — `Menu`, `MenuItem`, `MenuItemFields`, safe triangle handling, custom item padding overrides, and content padding overrides for sidecar layout
- `app/src/workspace/view.rs` — `unified_new_session_menu_items()`, `build_menus()`, `build_worktree_sidecar_search_input()`, `build_worktree_sidecar_items()`, `configure_worktree_new_session_sidecar()`, `refresh_new_session_sidecar_for_active_kind()`, `update_new_session_sidecar()`, and sidecar overlay rendering
- `app/src/workspace/action.rs` — `OpenWorktreeInRepo`, `OpenWorktreeAddRepoPicker`
- `app/src/user_config/mod.rs` — `default_tab_configs_dir()`, `ensure_default_worktree_config()`
- `app/resources/tab_configs/default_worktree.toml` — embedded worktree template
- `app/src/terminal/profile_model_selector.rs` — reference implementation of the sidecar pattern

## Current State

Before this change, the horizontal tab bar chevron and vertical tab bar `+` button generated different menu items via separate functions (`new_session_menu_items()` and `vertical_tabs_new_session_menu_items()`). There was no submenu/sidecar support for grouping shells under "Terminal" or repos under "Worktree in".

The `Menu` component had a `MenuItem::Submenu` variant (added in PR #13305 by Andrew Sweet, Jan 2025) marked `#[deprecated("Submenus are not ready for use yet")]`. A SafeZone attempt was made and reverted (PRs #15171 / #15422, May 2025). The safe triangle infrastructure was later added (PR #22158, Feb 2026) but only wired up for the model picker's external sidecar approach, never for built-in `MenuItem::Submenu`.

## Key Design Decision: Sidecar vs. MenuItem::Submenu

We initially attempted to use the built-in `MenuItem::Submenu` variant. After extensive debugging, we identified fundamental issues:

1. **No safe triangle wiring**: `MenuItem::Submenu` renders the child menu as an overlay inside the same `Menu` view. The safe triangle infrastructure (`with_safe_triangle()`, `set_safe_zone_target()`) was designed for external callers (model picker), not for internal submenu rendering. Wiring it internally would require the `Menu`'s render method to feed back the submenu panel's bounding rect to the action handler — a cross-concern that doesn't fit the render-then-act model.

2. **Hover event routing**: All submenu item actions (`HoverSubmenuLeafNode`, `Select`) are dispatched to the single `Menu<A>` view. The depth-0 `SubMenu` handles them, but depth-1 submenu items share the same action namespace. `MenuAction::Select` has no depth parameter, so a click on submenu item at row 0 sets `selected_row_index = 0` on the depth-0 menu, highlighting the wrong item ("Agent" instead of the first repo).

3. **Race conditions in hover callbacks**: The `on_hover` callback fired `HoverSubmenuLeafNode` on both hover-in AND hover-out (due to `is_hovered || is_enabled`). When moving between items, the unhover event from item A fired after the hover-in event from item B, resetting the selection back to A. (We fixed this specific bug: `is_hovered` only.)

4. **`UnhoverSubmenuParent` immediately closes the submenu**: When the mouse leaves a submenu parent to move toward the child panel, intermediate items trigger `HoverSubmenuLeafNode` which closes the submenu. Without a safe triangle, diagonal mouse movement is impossible.

**Decision**: Pivot to the model picker's proven sidecar pattern — two separate `Menu` views managed by the Workspace, with the existing safe triangle infrastructure wired up externally.

## Proposed Changes

### 1. Unified menu items (`unified_new_session_menu_items`)

Replaced both `new_session_menu_items()` and `vertical_tabs_new_session_menu_items()` with a single function. Menu order: Agent → Terminal → Cloud Oz → Worktree in (submenu parent) → [user tab configs] → separator → New Tab Config.

On macOS/Linux, Terminal is a regular `MenuItemFields::new("Terminal")` with `AddTerminalTab` as its action and the ⌘T shortcut. On Windows (`#[cfg(target_os = "windows")]`), Terminal itself is a submenu parent using `MenuItemFields::new_submenu()` — this shows a sidecar with a "Default Terminal" row plus available shells on hover.

"Worktree in" uses `MenuItemFields::new_submenu()` which sets `has_submenu = true` → renders a chevron `>` indicator. It has no `on_select_action` since it's activated by hover, not click.

### 2. Sidecar menu (`new_session_sidecar_menu`)

New field on `Workspace`: `new_session_sidecar_menu: ViewHandle<Menu<WorkspaceAction>>`. Created in `build_menus()` as a plain `Menu::new()` with width `NEW_SESSION_SIDECAR_WIDTH` (currently 300px), scrollable variant, and max height 400px.

### 3. Main menu configuration

The main `new_session_dropdown_menu` is created with `.with_safe_triangle().with_ignore_hover_when_covered()`. This enables:
- **Safe triangle**: Suppresses `HoverSubmenuLeafNode` events when the mouse is moving within the triangular safe zone toward the sidecar panel.
- **Ignore hover when covered**: Prevents depth-0 items under the sidecar overlay from firing hover events.

### 4. Hover-driven sidecar orchestration

In `handle_new_session_menu_event`, on `MenuEvent::ItemHovered` or `MenuEvent::ItemSelected`:
1. Read `menu.hovered_index()` (not `selected_index()` — see bug fix below) and the hovered item's label.
2. If label is "Terminal" (Windows only, `#[cfg(target_os = "windows")]`): populate sidecar with available shells.
3. If label is "Worktree in": populate sidecar with a custom search row, filtered repos from `PersistedWorkspace`, and a pinned "Add new repo" footer.
4. If label is None (separator): hide sidecar.
5. Otherwise: hide sidecar, clear safe zone and `submenu_being_shown_for_item_index`.
6. If hovered is None (mouse left menu, possibly onto sidecar): keep current state.
7. Read the sidecar panel's rect from the previous frame via `element_position_by_id_at_last_frame(window_id, "new_session_sidecar")`.
8. Set `main_menu.set_safe_zone_target(sidecar_rect)` and `main_menu.set_submenu_being_shown_for_item_index(Some(hovered_index))`.
### 5. Worktree search row and filtering

The worktree sidecar owns three new pieces of state on `Workspace`:

- `new_session_sidecar_kind` — tracks whether the current sidecar is Terminal or Worktree
- `worktree_sidecar_search_editor` — a dedicated `EditorView`
- `worktree_sidecar_search_query` — the current filter text

`build_worktree_sidecar_search_input()` constructs the single-line editor and subscribes to:

- `EditorEvent::Edited(_)` → updates `worktree_sidecar_search_query` and rebuilds the active sidecar
- `EditorEvent::Escape` → clears the query, clears the buffer, and rebuilds the sidecar

`build_worktree_sidecar_items()` prepends a custom first row implemented via `MenuItemFields::new_with_custom_label(...)`. This row is:

- non-interactive
- not hover-highlighted
- rendered with custom menu item padding overrides
- styled as a compact bordered search field

The search row is part of the normal scrollable menu content rather than a pinned header, so it scrolls away with the repo list. Repo rows are filtered by checking whether the lowercased repo path contains the lowercased trimmed query string.

### 6. Sidecar rendering
### 5. Sidecar rendering

When `show_new_session_sidecar` is true, the workspace render adds a positioned overlay child:
- Anchored to the hovered item's `SavePosition` (each `MenuItemFields` wraps its element in `SavePosition(label)` at render time).
- Wrapped in `SavePosition("new_session_sidecar")` so the safe zone rect can be read on the next frame.
- Positioned at TopRight → TopLeft with 4px gap.
### 7. Pinned footer and menu layout tweaks

The worktree sidecar keeps "Add new repo" visible via `set_pinned_footer_builder(...)` on the sidecar menu. The repo list scrolls independently above it.

To match the current design:

- the menu uses `set_content_padding_overrides(Some(0.), None)` so the first content row sits flush with the top of the sidecar
- the search field uses top-only rounded corners
- the pinned footer uses bottom-only rounded corners

Supporting this required a small `Menu` enhancement in `app/src/menu.rs` to allow depth-0 content top/bottom padding overrides in addition to the existing per-item padding overrides.

### 8. Sidecar event handling
### 6. Sidecar event handling

On `MenuEvent::Close { via_select_item: true }`: item clicked in sidecar → also close the main menu.
On `MenuEvent::Close { via_select_item: false }`: dismissed without selecting → hide sidecar only.

### 9. Worktree-in-repo action

`OpenWorktreeInRepo { repo_path }`: loads `~/.warp/default-tab-configs/worktree.toml` (created from embedded template if missing), substitutes template variables, and opens the tab immediately.

The worktree template parameterizes the pane type via `{{pane_type}}` (instead of hardcoding `type = "terminal"`). The `open_worktree_in_repo` handler reads the user's `DefaultSessionMode` setting and sets `pane_type` to `"agent"` when AI is enabled and the default is Agent, or `"terminal"` otherwise. This means worktree sessions respect the user's preference — if they prefer Agent mode, the worktree opens in Agent mode.

**Important**: Template variables (`{{repo}}`, `{{branch_name}}`, `{{pane_type}}`) are substituted in the raw TOML string BEFORE parsing into `TabConfig`, because the TOML deserializer validates enum fields like `type` against known variants (`terminal`, `agent`, `cloud`) and would reject `{{pane_type}}` as invalid.

Params substituted: `repo` (selected path), `branch_name` (auto-generated via `generate_worktree_branch_name()`), `pane_type` (from default session mode). On macOS, the data directory is channel-specific (`~/.warp-local/` for Local, `~/.warp/` for Stable).

### 10. Bug fixes in `menu.rs` and search-row layout

**on_hover race condition**: Changed `MenuItemFields::render` on_hover callback from `is_hovered || is_enabled` to `is_hovered` only. Previously, when a leaf node was unhovered (`is_hovered=false`), it dispatched `HoverSubmenuLeafNode` with a stale `row_index`. If this event was processed after the entering item's hover event, it overwrote `hovered_row_index` with the wrong value — causing the sidecar to not show when entering a submenu parent from above. Continuous position tracking for the safe triangle is handled by the separate `on_mouse_in` handler.

**`hovered_index` over `selected_index`**: `update_new_session_sidecar` uses `hovered_index()` (not `selected_index()`) as the source of truth. `hovered_row_index` accurately tracks the mouse and survives `reset_selection()` (which only clears `selected_row_index`/`selected_item_index`). Previously, `selected_index` got stuck on a submenu parent because `UnhoverSubmenuParent` was suppressed when `submenu_being_shown_for_item_index` was set — the sidecar persisted even when the user moved to unrelated items.

**Removed `UnhoverSubmenuParent` suppression**: The blanket suppression of `UnhoverSubmenuParent` when `submenu_being_shown_for_item_index.is_some()` was removed. The safe triangle already handles diagonal mouse movement toward the sidecar (by suppressing `HoverSubmenuLeafNode`). The blanket suppression was redundant and prevented `selected_index` from ever clearing when the user moved away.

**Finite-width search editor layout**: The custom worktree search row originally rendered the editor through an extra clipped container path that could hand the editor an infinite width constraint at runtime. The final implementation uses the same `icon + Shrinkable::new(1., ChildView::new(&editor))` pattern used elsewhere in the codebase, which keeps the search field visually compact while ensuring the editor receives a finite width.

## End-to-End Flow

1. User clicks `+` button (vertical tabs) or chevron (horizontal tabs).
2. `toggle_new_session_dropdown_menu()` calls `unified_new_session_menu_items()` and populates the main menu.
3. User hovers "Worktree in" → `MenuEvent::ItemHovered` fires → `update_new_session_sidecar()` reads `hovered_index()`, sees label is "Worktree in", populates sidecar with repos, sets safe zone target, shows sidecar overlay.
4. Sidecar renders a scrollable search row followed by repo items, with a pinned footer at the bottom.
5. User types in the search field → `EditorEvent::Edited(_)` updates `worktree_sidecar_search_query` → `refresh_new_session_sidecar_for_active_kind()` rebuilds the sidecar with filtered repo rows.
6. User moves mouse toward sidecar → safe triangle suppresses intermediate `HoverSubmenuLeafNode` events → sidecar stays visible.
7. User clicks a repo in sidecar → `OpenWorktreeInRepo` action dispatched → sidecar close event fires with `via_select_item: true` → main menu also closes.

## Risks and Mitigations

- **Safe zone first-frame delay**: On the first hover that opens a sidecar, the sidecar rect from the previous frame is `None` (panel wasn't rendered yet). The safe zone is set to `None`, meaning the first frame has no safe triangle protection. On the next frame, the rect is available. Mitigation: the delay is one frame (~16ms), imperceptible in practice.
- **Label-based item identification**: The hover handler identifies submenu parents by comparing the hovered item's label string ("Terminal", "Worktree in"). If labels change, the sidecar won't show. Mitigation: these are hardcoded UI strings unlikely to change without updating the handler.
- **Sidecar dismiss**: Clicking outside both menus triggers the main menu's `Dismiss` handler, which closes everything. The sidecar's own `Dismiss` is not active since it's not wrapped in one — it's a positioned overlay within the main menu's dismiss scope.

## Testing and Validation

- Build check: `cargo check -p warp` passes with no errors.
- Manual testing: Open both horizontal and vertical tab menus → verify identical items. On macOS/Linux: click Terminal → verify terminal tab opens. On Windows: hover Terminal submenu parent → verify sidecar shows the default terminal row plus shells.
- Hover Worktree in → verify the sidecar shows the search row, repo items, and pinned footer.
- Type into "Search repos" → verify repo rows filter live and the footer remains pinned.
- Move mouse diagonally to sidecar → verify safe triangle prevents premature closing.
- Click a sidecar item → verify the action fires and both menus close.
- Verify that existing menus (tab right-click, overflow, model picker) are unaffected by the `is_hovered` fix and menu padding overrides.

## Follow-ups

- **New Tab Config skill invocation**: V0 opens the TOML template. Follow-up: auto-invoke the `tab-configs` skill via Oz agent.
- **`MenuItem::Submenu` cleanup**: The built-in submenu variant remains in the codebase (deprecated). Consider removing it or completing the safe-triangle wiring if a future use case requires inline submenus.
- **Sidecar left-fade for long paths**: `ClipConfig::start()` exists in the text layout system but right-aligns the text. A proper left-aligned + left-fade clip mode would need UI framework work.
- **macOS/Linux shell selector**: Currently only Windows shows the Terminal sidecar with shell choices. If shell selection is desired on other platforms, this can be re-enabled by removing the `#[cfg(target_os = "windows")]` gate.
