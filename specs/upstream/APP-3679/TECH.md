# APP-3679: New Worktree Modal — Tech Spec

## Problem

The "Create new tab config..." / "New worktree" menu items open a raw TOML template in an editor. This requires users to understand the TOML schema to create worktree configs. The new worktree modal provides a GUI-based "worktree factory" that generates, persists, and opens worktree tab configs without manual TOML editing.

## Relevant Code

- `app/src/tab_configs/new_worktree_modal.rs` — **new** modal body view (created in this change)
- `app/src/tab_configs/mod.rs` — module registration
- `app/src/tab_configs/params_modal.rs` — reference pattern for modal body views
- `app/src/tab_configs/repo_picker.rs` — reused `RepoPicker` component
- `app/src/tab_configs/branch_picker.rs` — reused `BranchPicker` component (includes no-commit fallback and loading placeholder)
- `app/src/view_components/filterable_dropdown.rs` — `FilterableDropdown` (stale selection fix in `set_items`)
- `app/src/workspace/view.rs (4636-4690, 4776-4816)` — menu item definitions for horizontal and vertical tabs
- `app/src/workspace/view.rs (7057-7196)` — event handlers and TOML generation
- `app/src/workspace/view.rs (1447-1476)` — `build_new_worktree_modal`
- `app/src/workspace/action.rs:554-557` — `OpenNewWorktreeModal` and `OpenNewWorktreeRepoPicker` action variants
- `app/src/workspace/util.rs:123` — `is_new_worktree_modal_open` workspace state flag
- `app/src/user_config/mod.rs:188-209` — `find_unused_worktree_config_path` utility
- `app/src/modal.rs` — `Modal<T>` and `ModalViewState<T>` wrappers
- `ui/src/ui_components/checkbox.rs` — `Checkbox` component

## Current State

The horizontal tab menu's last item is `"Create new tab config..."` which calls `create_and_open_new_tab_config` — writes a TOML template to `~/.warp/tab_configs/` and opens it in the user's editor. The vertical tab menu has the same action under the label `"New worktree"`.

Existing infrastructure:
- `RepoPicker`: filterable dropdown of known repos from `PersistedWorkspace`, with "+ Add new repo..." footer.
- `BranchPicker`: filterable dropdown of git branches for a repo, with async fetch and main-branch sorting.
- `Modal<T>` / `ModalViewState<T>`: standard modal pattern used by `TabConfigParamsModal`.
- Tab config filesystem watcher in `user_config/native.rs` automatically reloads `~/.warp/tab_configs/` on file changes.

## Proposed Changes

### 1. New view: `NewWorktreeModal`

File: `app/src/tab_configs/new_worktree_modal.rs`

A `View` + `TypedActionView` body for use inside `Modal<NewWorktreeModal>`:
- **State**: `RepoPicker`, `BranchPicker`, `autogenerate_branch_name: bool`, `selected_repo/branch: Option<String>`, mouse states for checkbox/cancel/open buttons.
- **Events**: `NewWorktreeModalEvent::{ Close, Submit { repo, branch, autogenerate_name }, PickNewRepo }`.
- **Actions**: `NewWorktreeModalAction::{ Cancel, Open, ToggleAutogenerate }`.
- **Render**: Column layout with repo picker, branch picker, checkbox row, and footer bar with Cancel/Open buttons. Body padding handled by the modal body view (not the `Modal` wrapper, which has `padding: 0`).
- **`on_open`**: Rebuilds pickers fresh, resets checkbox to checked, focuses the repo picker.
- **Repo → Branch sync**: When `RepoPickerEvent::Selected` fires, calls `branch_picker.refetch_branches(path)`.
- **`generate_worktree_branch_name()`**: Static `AtomicU32` counter producing `worktree-1`, `worktree-2`, etc.

### BranchPicker improvements

- **Loading placeholder**: When a branch fetch starts, the dropdown shows "Fetching branches…" as a selected placeholder item (inside the dropdown top bar) rather than as a separate text element below the dropdown. This prevents the modal from shifting layout. `selected_value()` returns `None` while `is_loading` is true so the placeholder is never treated as a real branch selection.
- **No-commit repo fallback**: `git for-each-ref refs/heads` only lists refs backed by actual commits, so a freshly initialised repo (`git init`, no commits) returns an empty branch list. The `fetch_branches` async block detects this and falls back to `detect_current_branch` (which uses `git branch --show-current`) to populate the picker with the initial branch (e.g. "main").
- **FilterableDropdown stale selection fix**: `FilterableDropdown::set_items` now clears the cached `selected_item` when the old selection's label is absent from the replacement item list. Previously, calling `set_items(vec![])` left a stale `selected_item` that caused the dropdown top bar to show a ghost label and `selected_item_label()` to return a value for a non-existent item.

### 2. Workspace wiring

Follows the exact same pattern as `tab_config_params_modal`:
- Field: `new_worktree_modal: ModalViewState<Modal<NewWorktreeModal>>` on `Workspace`.
- Builder: `build_new_worktree_modal(ctx)` — creates body, subscribes to body and modal events, wraps in `Modal` with compact header, 460×400px size, zero body padding.
- **Action**: `OpenNewWorktreeModal` — gets CWD from active terminal, calls `body.on_open(cwd)`, opens the modal.
- **Action**: `OpenNewWorktreeRepoPicker` — opens a folder picker, registers the new path in `PersistedWorkspace`, updates the modal's repo picker.
- **Render**: `if self.new_worktree_modal.is_open() { stack.add_child(...) }` in the overlay section.

### 3. Menu updates

Both `new_session_menu_items` (horizontal) and `vertical_tabs_new_session_menu_items` (vertical) replace their last "tab config creation" item with `"+ New Worktree"` → `WorkspaceAction::OpenNewWorktreeModal` with `icons::Icon::Plus`.

### 4. TOML generation on submit

`handle_new_worktree_submit(repo, branch, autogenerate_name)`:
1. Determine branch name: use `generate_worktree_branch_name()` if autogenerate, else use the selected branch (fallback to generated name if none).
2. Build TOML string with `name = "Worktree: {branch_name}"`, single `[[panes]]` with `type = "terminal"`, `cwd`, `worktree_name_autogenerated`, and `commands = ["git worktree add ...", "cd ..."]`.
3. Write to `~/.warp/tab_configs/worktree_{sanitized_branch}.toml` using `find_unused_worktree_config_path`.
4. Parse the TOML back into a `TabConfig` and call `open_tab_config(config, ctx)`.
5. The filesystem watcher picks up the new file and makes it available in the menu.

### 5. Utility: `find_unused_worktree_config_path`

In `user_config/mod.rs`. Sanitizes branch name for filename safety (alphanumeric, `-`, `_` only), tries `worktree_{name}.toml`, then `worktree_{name}_1.toml`, etc.

## End-to-End Flow

1. User clicks `+ New Worktree` in either tab menu.
2. `WorkspaceAction::OpenNewWorktreeModal` dispatched.
3. Workspace calls `body.on_open(cwd)`, opens the `ModalViewState`.
4. User interacts with repo/branch pickers and checkbox.
5. User clicks "Open" → `NewWorktreeModalAction::Open` → `try_submit()` → emits `NewWorktreeModalEvent::Submit`.
6. Workspace receives `Submit` event → `handle_new_worktree_submit()` generates TOML, writes file, parses it, calls `open_tab_config()`.
7. New tab opens with the worktree commands. File watcher detects the new `.toml` and adds it to `WarpConfig.tab_configs`.
8. Modal closes.

## Risks and Mitigations

- **TOML format drift**: The generated TOML must match what `TabConfig::deserialize` expects. Mitigation: the TOML is immediately parsed back after writing; any mismatch is caught and logged.
- **Branch name sanitization**: Special characters in branch names could produce invalid filenames. Mitigation: `find_unused_worktree_config_path` sanitizes to alphanumeric + `-` + `_`.
- **Counter reset**: `WORKTREE_COUNTER` is a static `AtomicU32` that resets on app restart, so branch names may collide across sessions. Mitigation: the TOML content includes the full branch name so collisions only affect filenames (handled by the `_N` suffix).

## Testing and Validation

- **Build check**: `cargo check -p warp` passes with no errors or warnings.
- **Manual testing**: Open both horizontal and vertical tab menus, verify "+ New Worktree" appears, opens the modal, and the full flow works.
- **Filesystem**: Verify `.toml` files appear in `~/.warp/tab_configs/` with correct content.
- **Re-use**: Verify saved worktree configs appear in the menu and can be re-opened.

## Follow-ups

- Replace the placeholder `generate_worktree_branch_name()` with a more sophisticated naming scheme (e.g., based on date, repo name, or user initials).
- Add validation that the selected repo is a valid git repository before allowing submission.
- Consider adding a text input for manual branch name entry when autogenerate is unchecked.
- Explore support for worktree deletion or management from the menu.
- Consider showing a distinct empty state in the branch picker when the repo genuinely has no branches (i.e. the `detect_current_branch` fallback also fails).
