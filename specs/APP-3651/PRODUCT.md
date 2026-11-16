# APP-3651: Vertical Tabs Panel — Pane Row Layout Iteration

## Summary

Iterate on the vertical tabs panel pane row rendering to improve information density, relevance, and interaction. The changes restructure how terminal and non-terminal pane rows display their content, introduce start-clipping for path text, replace the group collapse chevron with a close button, and deduplicate redundant terminal titles.

## Problem

The current vertical tabs layout has several information hierarchy issues:
- Terminal panes show the working directory as the primary line, but users more often care about *what they're doing* (the terminal title, conversation status, or last command) than *where they are*.
- Non-terminal panes waste a full row on a "kind badge" (e.g. "Code", "Notebook") that duplicates information already conveyed by the icon.
- Path text clips from the end, hiding the most distinguishing part (the filename) when paths are long.
- The expand/collapse chevron on tab group headers is no longer needed and occupies space that would be better used for a close button.
- When a shell's terminal title is just the working directory (the default for most shells), both the primary and secondary lines show the same information.

## Goals

- Make the most task-relevant information (terminal title, conversation status, or last command) the primary line for terminal panes.
- Reduce visual noise for non-terminal panes by inlining the kind icon next to the title and removing the standalone badge row.
- Show the most distinguishing portion of paths by clipping from the start.
- Provide a direct way to close a tab from the vertical tabs panel.
- Eliminate redundant information when the terminal title matches the working directory.

## Non-goals

- Implementing a "compact" single-row rendering mode (shown in the Figma mock but out of scope for this iteration).
- Adding new pane types or changing how pane titles/subtitles are set in `PaneConfiguration`.
- Changing the tab group header title, count label, or drag behavior.
- Search functionality in the control bar (already non-functional/placeholder).

## Figma / design references

Figma: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7079-22324&m=dev

Note: The Figma mock shows a compact single-row rendering that is not in scope. The close button (X) design on the tab group header and the inline kind icons next to titles are relevant references.

## User experience

### Terminal pane rows

Currently the terminal row layout is:
1. **Primary line** (main text color): working directory • git branch
2. **Secondary line** (sub text color): conversation title/status, or terminal title if it differs from working directory
3. **Tertiary line**: kind badge ("Terminal" or "Oz") + right-side badges (diff stats, PR)

The new layout reverses lines 1 and 2:
1. **Primary line** (main text color): terminal title or conversation status (see rules below)
2. **Secondary line** (sub text color): working directory • git branch
3. **Tertiary line**: unchanged (kind badge + right-side badges)

#### Primary line rules for terminal panes

The primary line content is determined by, in order of precedence:
1. If the pane has an active agent conversation with a display title: show the conversation status indicator followed by the conversation display title.
2. If the terminal title differs from the displayed working directory: show the terminal title.
3. If the terminal title matches the displayed working directory exactly (trimmed comparison) and there is a last completed user command: show the last completed user command string.
4. If the terminal title matches the displayed working directory and there is no completed user command (brand new session): show "New session" in the UI font.

The primary line always uses main text color.

#### Secondary line rules for terminal panes

The secondary line shows the working directory and git branch (same content as the current primary line) in sub text color. This line is always shown when a working directory is available.

#### Terminal title / working directory deduplication detail

"Matches exactly" means a case-sensitive, trimmed string comparison between `terminal_title_from_shell()` and `display_working_directory()`. The deduplication only applies when there is no active agent conversation display title.

When deduplication triggers and we show the last completed command:
- The command string should be rendered in the monospace font family currently used for terminal titles.
- If the last completed command is empty or unavailable (brand new session with no commands run), show the text "New session" in the UI font (not monospace).

### Non-terminal pane rows

Currently non-terminal panes render:
1. **Title line**: title text (main text color)
2. **Subtitle line** (if non-empty): subtitle text (sub text color)
3. **Meta row**: kind badge (icon + label like "Code", "Notebook") on the left, optional badge on the right

The new layout removes the standalone meta row and inlines the kind icon:
1. **Primary line**: kind icon (12px, sub text color) followed by title text (main text color). For code panes, the icon is the programming language icon for the active file (falling back to the generic `Code2` icon if no language icon exists).
2. **Subtitle line** (if non-empty): subtitle text (sub text color)

No separate badge row is rendered. The kind badge row is removed entirely.

#### Code panes with multiple tabs

For code panes with more than one open tab:
- **Primary line**: kind icon + first file's path (the active tab's path from `PaneConfiguration.title()`).
- **Secondary line**: "and X more" where X is `tab_count - 1`, rendered in sub text color. This replaces the current `(+N)` secondary title set by `CodeView::set_title`.

For code panes with a single tab, render normally using the title from `PaneConfiguration`.

#### Kind icon for code panes

Use `crate::code::icon_from_file_path` to attempt to get a language-specific icon (Rust, TypeScript, Python, etc.) from the active file's path. If `icon_from_file_path` returns `None`, fall back to the generic `WarpIcon::Code2` icon rendered as a `to_warpui_icon` in sub text color.

For non-code, non-terminal panes (Notebook, Settings, Workflow, etc.), use the existing `TypedPane::icon()` value.

### Path clipping

All path text in pane rows should clip from the start instead of the end. This applies to:
- Working directory text in terminal pane secondary lines
- File path text in code pane primary lines
- Any other path-like text rendered in pane rows

Use `ClipConfig::start()` instead of `ClipConfig::end()`. The `ClipConfig::start()` variant already exists in the codebase and fades text at the leading edge.

Git branch text should continue to clip from the end (branch names are most distinguishing at the start).

### Close button on tab group headers

Replace the expand/collapse chevron button in the tab group header with a close button (X icon):
- The close button uses an X icon instead of `ChevronDown`/`ChevronRight`.
- Clicking the close button dispatches `WorkspaceAction::CloseTab(tab_index)` to close the entire tab.
- The close button has the same hover styling as the current collapse button (background highlight on hover, pointing hand cursor).
- The close button is always visible in the header (not only on hover).

Remove all expand/collapse state and behavior:
- Remove the `collapsed_tab_groups` set from `VerticalTabsPanelState`.
- Remove the `toggle_group_collapsed`, `toggle_all_groups_collapsed`, and `is_group_collapsed` methods.
- Remove the `ToggleVerticalTabsGroupCollapsed` action handling.
- Tab groups are always expanded (pane rows are always visible).

### No behavioral changes

- Clicking a pane row still focuses that pane (dispatches `WorkspaceAction::FocusPane`).
- Clicking the tab group header title still activates the tab.
- Double-clicking the header still triggers rename.
- Right-clicking still opens the tab context menu.
- Drag-and-drop tab reordering is unchanged.
- The pane count label next to the close button is unchanged.

## Success criteria

1. **Terminal primary line**: For a terminal pane with no agent conversation and a non-default terminal title (e.g. `vim`, `htop`, a custom title), the primary line shows that terminal title in main text color.
2. **Terminal primary line (agent)**: For a terminal pane with an active agent conversation, the primary line shows the conversation status indicator + conversation display title.
3. **Terminal secondary line**: The working directory and git branch are always shown on the second line in sub text color, with the same layout as the current primary line.
4. **Terminal title dedup**: For a terminal pane where `terminal_title_from_shell()` trims to the same string as `display_working_directory()`, the primary line shows the last completed user command (if available) instead of the terminal title.
5. **New session fallback**: For a brand new terminal session with no completed commands and a default terminal title matching the working directory, the primary line shows "New session" in the UI font.
6. **Non-terminal kind icon inline**: For a Notebook pane, the primary line shows the Notebook icon followed by the title — no separate "Notebook" badge row.
7. **Code pane language icon**: For a code pane with a `.rs` file open, the primary line shows the Rust language icon (not the generic Code2 icon).
8. **Code pane language fallback**: For a code pane with a `.txt` file (no language icon), the primary line shows the generic Code2 icon.
9. **Code pane multi-tab**: For a code pane with 3 tabs open, the primary line shows the active file path and the secondary line shows "and 2 more".
10. **No badge row**: Non-terminal pane rows have no standalone kind badge or badge row beneath the title/subtitle.
11. **Path start-clipping**: A long working directory path like `~/very/long/path/to/my-project` clips from the left (showing `…my-project`) rather than from the right.
12. **Git branch end-clipping**: A long git branch name clips from the right (showing `feature/my-long-br…`).
13. **Close button**: Clicking the X button on a tab group header closes the tab (equivalent to `CloseTab`). The close button shows a hover highlight.
14. **No collapse**: There is no expand/collapse chevron. All pane rows in a tab group are always visible.
15. **Pane row click**: Clicking a pane row within a group still focuses that pane.

## Validation

- **Manual testing**: Open Warp with vertical tabs enabled. Create terminal tabs with various states (default shell, running `vim`, agent conversations, multiple directories). Verify primary/secondary line content matches the rules above.
- **New session**: Open a brand new terminal tab. Before running any commands, verify the primary line says "New session" and the secondary line shows the working directory.
- **Code pane testing**: Open code panes with single and multiple files of various languages. Verify language icons appear for supported extensions and fall back to Code2 for unsupported ones. Verify multi-tab "and X more" rendering.
- **Path clipping**: Resize the vertical tabs panel to a narrow width and verify that long paths clip from the start (filename visible) and git branches clip from the end.
- **Close button**: Click the X on a tab group header and verify the tab closes. Verify no collapse/expand behavior remains.
- **Terminal dedup**: In a shell where the terminal title defaults to the working directory, run a command and verify the primary line shows the last command rather than the (redundant) working directory.
- **Regression**: Verify tab group header click (activate tab), double-click (rename), right-click (context menu), and drag reorder still work. Verify pane row click still focuses the pane.

## Open questions

1. **Last completed command source**: The terminal model tracks block metadata, but the exact API to retrieve "the last completed user command string" from `TerminalView` needs to be identified or added. The current codebase doesn't expose a simple `last_completed_command_text()` accessor — this will need to be addressed in the tech spec.
2. **"and X more" click behavior**: Should clicking the "and X more" secondary line text do anything special (e.g. cycle to the next tab in the code pane), or should it behave the same as clicking anywhere else on the pane row (focus the code pane)?
