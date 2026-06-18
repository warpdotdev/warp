# APP-3679: New Worktree Modal

Linear: [APP-3679](https://linear.app/warpdotdev/issue/APP-3679/modal-for-new-worktree-and-flow-for-saving-that-to-a-launch-config)

## Summary

Replace the "Create new tab config..." / "New worktree" menu items in both the horizontal and vertical tab bar menus with a "+ New Worktree" button that opens a GUI modal. The modal lets users select a repository, a base branch, and optionally auto-generate the worktree branch name. On submit, it writes a reusable worktree tab config TOML file to `~/.warp/tab_configs/` (which the filesystem watcher picks up for the menu), and immediately opens the worktree as a new tab.

## Problem

Creating a worktree tab config currently requires hand-authoring TOML. The "Create new tab config..." button opens a template file in an editor, which is a poor experience for the common worktree use case. Users need a quick, discoverable way to create reusable worktree configs from a modal — a "worktree factory."

## Goals

- Provide a modal-based flow for creating git worktrees from both the horizontal and vertical tab menus.
- Persist each created worktree config as a `.toml` file so it appears in the menu for future re-use.
- Open the resulting worktree tab immediately after creation.
- Support auto-generating unique worktree branch names when the user checks the autogenerate option.

## Non-goals

- Validating that the selected repository is a valid git repo before submission.
- Listing existing worktrees or providing worktree management beyond creation.
- Replacing the existing "Create new tab config..." flow for non-worktree configs (that still uses the TOML template).
- Running `git worktree` commands from within the modal itself (commands are written into the config and executed when the tab opens).

## Figma

- Modal: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7143-34685&m=dev

## User Experience

### Menu entry point

Both horizontal and vertical tab bar menus replace their last row item with:
- Label: "+ New Worktree"
- Icon: Plus
- Action: Opens the new worktree modal

In the horizontal tab bar, the item appears after any existing tab configs, separated by a `MenuItem::Separator`. In the vertical tab bar, the item appears in the existing "New worktree" position.

### Modal layout (matches Figma)

1. **Header**: "New worktree" with compact-pane close button (X / ESC).
2. **Body**:
   - **Select repository**: Label "Select repository" + `RepoPicker` dropdown (filterable, with "+ Add new repo..." footer to open a folder picker).
   - **Select branch**: Label "Select branch" + `BranchPicker` dropdown (filterable, auto-populates when a repo is selected).
   - **Autogenerate checkbox**: Checkbox labeled "Autogenerate worktree branch name", checked by default.
3. **Footer**: Border-top separator, right-aligned "Cancel" (secondary) and "Open" (accent) buttons. "Open" is disabled until a repo is selected.

### Interaction flow

1. User clicks "+ New Worktree" in either tab menu.
2. Modal opens. The repo picker shows known repos from `PersistedWorkspace`.
3. User selects a repo (or adds a new one via the folder picker).
4. Branch picker populates with that repo's git branches; user optionally selects a base branch.
5. User optionally unchecks "Autogenerate worktree branch name" — if unchecked, the selected branch is used directly; if checked, a placeholder name like `worktree-1` is generated.
6. User clicks "Open":
   - A worktree tab config TOML is generated and written to `~/.warp/tab_configs/worktree_{branch_name}.toml`.
   - The config is immediately parsed and opened as a new tab (running `git worktree add` and `cd` commands).
   - The filesystem watcher picks up the new file, so it appears in the menu for future use.
7. User can later click the saved worktree config in the menu to re-run it.

### Cancel / close behavior

- Clicking "Cancel", pressing ESC, or clicking the X button closes the modal with no side effects.

### Auto-generate branch name

When the checkbox is checked, a placeholder function generates unique names by incrementing a global counter: `worktree-1`, `worktree-2`, etc. This counter resets on app restart (session-scoped). Future iterations may use more sophisticated naming.

## Edge Cases

1. **No repos available**: The repo picker shows an empty list with only the "+ Add new repo..." footer.
2. **No git branches (no commits)**: If the selected repo has no commits yet (e.g. freshly `git init`), `git for-each-ref` returns no refs. The branch picker falls back to `detect_current_branch` (which uses `git branch --show-current`) so the user can still select the initial branch (e.g. "main").
3. **Branch loading state**: While branches are being fetched, the dropdown displays "Fetching branches…" as placeholder text inside the dropdown itself (rather than as a separate label below it) so the modal layout does not shift.
4. **Repo changed mid-modal**: When the user changes the repo selection, the branch picker refetches branches for the new repo and clears any prior branch selection. The dropdown correctly clears stale display text from the previous repo.
5. **Duplicate filenames**: `find_unused_worktree_config_path` appends `_1`, `_2`, etc. to avoid collisions.
6. **Non-local_fs builds (WASM)**: The submit handler is a no-op on WASM — the modal still opens but "Open" does nothing beyond closing the modal.
7. **Invalid TOML parse**: If the generated TOML fails to parse (should not happen), a warning is logged and no tab opens.

## Success Criteria

1. Both horizontal and vertical tab menus show "+ New Worktree" as the last item with a Plus icon.
2. Clicking "+ New Worktree" opens a modal matching the Figma design.
3. The "Open" button is disabled until a repo is selected.
4. Submitting the modal writes a `.toml` file to `~/.warp/tab_configs/` with the correct worktree commands.
5. The new tab config appears in the menu on subsequent menu opens (via filesystem watcher).
6. The new tab opens immediately with the correct worktree commands.
7. The autogenerate checkbox produces unique branch names.
8. Cancel / ESC / X closes the modal without side effects.

## Validation

- Build and run Warp locally; click "+ New Worktree" from both horizontal and vertical tab menus.
- Verify the modal layout matches the Figma mock (repo picker, branch picker, checkbox, footer buttons).
- Select a repo, select a branch, click "Open" — confirm a `.toml` file appears in `~/.warp/tab_configs/` and a new tab opens.
- Re-open the menu — confirm the saved worktree config appears.
- Click the saved worktree config — confirm it opens a new tab with the same worktree commands.
- Verify autogenerate checkbox produces `worktree-1`, `worktree-2`, etc.
- Verify Cancel/ESC/X close the modal.

## Open Questions

(None outstanding.)
