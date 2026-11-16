# Don't auto-add indexed repos to Directory tab colors
Linear: [APP-4115](https://linear.app/warpdotdev/issue/APP-4115/dont-auto-add-indexed-repos-including-worktrees-to-directory-tab)
## Summary
The Directory tab colors list in Settings > Appearance should stop auto-populating itself from the set of indexed codebases. Instead, the user explicitly chooses which directories to color, assisted by a searchable `Add directory color` dropdown when known repos are available. If there are no candidate repos left to show, the control falls back to a plain `Add directory color` button that opens the native folder picker directly.
## Problem
Today, every time `CodebaseIndexManager` emits a sync-state update, Warp merges all currently-indexed codebase paths into the user's `appearance.tabs.directory_tab_colors` setting as `Unassigned` entries. This produces two user-visible problems reported in `#feedback-app`:
1. Worktrees under paths like `~/.warp-dev/worktrees/warp-internal/...` get indexed as codebases and then silently added to the colors list, flooding the settings panel with entries the user never wanted to manage.
2. The Directory tab colors list grows to reflect indexing state rather than intentional user configuration, making the settings panel noisy and hard to use.
The relevant subscription lives in `app/src/workspace/view.rs:2775-2790`. The existing folder-picker button lives in `app/src/settings_view/appearance_page.rs:4632-4671`.
## Goals
- Stop auto-adding any indexed codebase (including worktrees) to `directory_tab_colors`.
- Make known repos one click to add when candidates are available.
- Preserve a direct folder-picker path when there are no known repos left to offer.
## Non-goals
- Migrating or cleaning up entries that were already auto-added on prior app versions. Users remove clutter manually via the existing per-row X button.
- Changing how `color_for_directory` resolves the active tab color for any working directory. Longest-prefix matching is unchanged.
- Changing the set of ANSI color dots, the Suppressed state, or the overall layout of the Directory tab colors settings card.
- Hiding worktrees from the candidate set. Worktrees remain selectable so users can intentionally color them.
- Introducing a new setting to toggle the auto-add behavior.
## Figma / design references
Figma: none provided. Keep the searchable dropdown behavior aligned with the `New worktree config` sidecar in `app/src/workspace/view.rs:8143-8301`. Keep the fallback button aligned with the prior `Add directory color` button treatment in `app/src/settings_view/appearance_page.rs:4674-4720`.
## User experience
### Removal of auto-add
- Newly indexed repos (including worktrees) never appear in `appearance.tabs.directory_tab_colors` on their own.
- Existing indexed or ambiently-detected repos are not added when the user opens Settings, switches tabs, starts new sessions, or triggers reindexing.
- Previously auto-added entries remain in the setting until the user removes them via the X button. The visible list and X-button behavior in `appearance_page.rs:4674-4815` is unchanged.
### `Add directory color` control
The header control is conditional:
- If candidate repos exist, render the searchable `Add directory color` dropdown.
- If no candidate repos exist, render a plain `Add directory color` action button with the leading plus icon.
Candidate set:
- The candidate set is the union of indexed codebase paths from `CodebaseIndexManager::get_codebase_paths` and persisted workspace paths from `PersistedWorkspace::workspaces()`.
- Paths are deduped by canonical form.
- Linked worktrees are kept in the list.
- Entries whose canonical path is already a key in `directory_tab_colors` with a color other than `Suppressed` are filtered out.
- Entries keyed as `Suppressed` remain selectable so users can undo a prior removal.
- Entries whose path no longer exists on disk are filtered out.
- Remaining entries are sorted alphabetically by canonical path, matching the visible colors list.
When candidates exist:
- Each dropdown row shows the user-friendly path, using the same `user_friendly_path` helper used by the list below.
- The dropdown search field filters rows by case-insensitive substring match on the full path.
- A pinned footer labeled `+ Add directory…` remains visible at the bottom of the dropdown.
- Selecting a row adds the corresponding canonical path to `directory_tab_colors` with `DirectoryTabColor::Unassigned`.
- Clicking the pinned footer closes the dropdown and opens the native folder picker.
When no candidates exist:
- Clicking the fallback button opens the native folder picker directly.
- If the user cancels the folder picker, nothing changes.
Interaction details and invariants:
- Opening the dropdown or clicking the fallback button never mutates `directory_tab_colors` by itself. Mutation only happens on selecting a row or completing the folder picker.
- The visible colors list below the header is rendered exactly as it is today: same row layout, same color dots, same X button, same alphabetical sort order.
- The per-row color picker and X button continue to read/write through the existing `SetDefaultDirectoryTabColor` and `RemoveDefaultDirectoryTabColor` actions.
- `RemoveDefaultDirectoryTabColor` continues to persist `Suppressed` rather than deleting the key.
## Success criteria
- Starting Warp fresh, opening and closing repos, triggering codebase indexing, or creating worktrees does **not** add any entries to `appearance.tabs.directory_tab_colors` in the TOML settings file.
- When candidate repos exist, the `Add directory color` control is the searchable dropdown.
- The dropdown lists indexed codebases and persisted workspaces that are not already keyed in `directory_tab_colors` with a non-`Suppressed` color. Entries are deduped by canonical path.
- Worktrees (paths whose repository has an external gitdir) are included in the candidate set and can be added.
- Selecting a dropdown row adds that path to `directory_tab_colors` as `Unassigned`. The row appears in the visible colors list below immediately.
- The pinned `+ Add directory…` footer remains visible at the bottom of the dropdown and opens the native folder picker.
- When every known repo is already present (with a non-`Suppressed` color), the control falls back to the plain `Add directory color` button and clicking it opens the native folder picker directly.
- Entries that were auto-added by the old behavior on prior app versions are not removed, altered, or reordered by this change.
- Longest-prefix matching in `DirectoryTabColors::color_for_directory` returns the same color for any given working directory that it returned before this change, given the same persisted configuration.
## Validation
- Unit tests on a helper that computes the candidate set, covering dedupe across indexed and persisted sources, filtering out keys present with non-`Suppressed` colors, keeping `Suppressed` keys, filtering out missing paths, and worktree inclusion.
- Unit test confirming that handling a `CodebaseIndexManagerEvent::SyncStateUpdated` does not mutate `directory_tab_colors`.
- Manual validation:
  1. With several worktrees under `~/.warp-dev/worktrees/warp-internal/...` indexed, confirm none appear in the Directory tab colors list on a fresh profile.
  2. Confirm that when candidate repos exist, `Add directory color` renders as the searchable dropdown and its list contains those worktrees.
  3. Select a repo from the dropdown, confirm it appears in the list below with the `Unassigned` dot selected.
  4. With all known repos added, confirm the control falls back to the plain button and clicking it opens the native folder picker.
  5. Pick a folder via either the dropdown footer or the fallback button, confirm it is added to the list just like the prior file-picker flow.
- Visual check via `verify-ui-change-in-cloud` that the dropdown renders correctly when candidates exist and that the plain fallback button appears when there are none.
## Open questions
- None at spec creation.
