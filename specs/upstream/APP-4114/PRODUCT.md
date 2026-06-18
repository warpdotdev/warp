# APP-4114: Vertical Tabs — Custom Pane Names

## Summary

Add support for custom names on individual panes in Vertical Tabs. This gives users a way to label split panes independently from the tab name, so a multi-pane tab can show meaningful pane labels like `server`, `tests`, or `review` without changing the tab header or the underlying terminal title.

The feature is scoped to pane items in the Vertical Tabs panel. Custom pane names should replace the main pane item title wherever pane rows are shown, but they should not change the pane header itself or any tab-level naming behavior.

## Problem

Warp already supports renaming tabs, but vertical tabs expose pane-level items. When a tab has multiple panes, the existing tab rename feature is too coarse: it names the whole tab rather than the individual split panes inside it.

Users need a pane-level naming affordance so they can distinguish related panes at a glance while keeping the tab title and pane header behavior unchanged.

## Goals

- Add a pane-level rename entry point in the Vertical Tabs right-click menu.
- Make right-click rename apply to the pane that was right-clicked.
- Keep existing overflow / kebab menus tab-scoped, including their existing tab rename behavior.
- In `View as = Panes`, allow double-clicking pane title text to start renaming that pane.
- Make a custom pane name take precedence over the `Pane title as` setting for the pane item's main title text.
- Scope custom pane names to pane items in Vertical Tabs only.
- Preserve existing tab rename behavior for tab group headers.

## Non-goals

- Replacing or removing the existing tab rename feature.
- Applying custom pane names to the pane header, terminal title, shell title, window title, or tab group header.
- Adding a global pane-name management UI outside Vertical Tabs.
- Adding rename affordances to the horizontal tab strip or pane header.
- Changing the available `Pane title as`, `Additional metadata`, `Show`, `View as`, or `Tab item` settings.
- Changing how terminal titles, working directories, branches, or summary cards are computed when no custom pane name exists.

## Figma / design references

Figma: none provided.

## User experience

### Conceptual model

Custom pane names are user-authored labels for pane rows in Vertical Tabs.

A custom pane name is distinct from:

- the tab name shown in the tab group header
- the pane header title shown inside the main workspace area
- terminal title text reported by the shell
- generated pane titles derived from command, conversation, working directory, branch, file, notebook, workflow, or other pane metadata

When a custom pane name exists, it controls only the main title text of the corresponding pane item in Vertical Tabs.

### Menu entry point

Vertical Tabs should expose pane rename from the right-click context menu on pane rows. Overflow / kebab menus remain tab-group-level menus in this iteration.

#### Right-click menu

When the user right-clicks a pane item in Vertical Tabs:

- the context menu includes a rename action for the pane
- choosing the rename action starts editing the pane that was right-clicked
- the action does not rename the tab group header, even if the pane is the only pane in the tab
- the action is available for pane rows in `View as = Panes`
- the action is not exposed in `View as = Tabs`; tab-level rename remains the available rename action there

If a right-click occurs on a tab group header, existing tab-level context menu behavior remains unchanged.

#### Overflow / kebab menu

Vertical Tabs currently supports tab-group-level overflow menus only; individual pane rows do not have their own overflow menu.

Expected behavior:

- overflow / kebab menus continue to expose tab-level actions only
- the rename action in an overflow / kebab menu renames the tab, not a pane
- `View as = Tabs` should expose only tab rename actions, because the rendered item is being used as a tab-level representative row
- a custom pane name created while `View as = Panes` was enabled can still affect `View as = Tabs` focused-session display later, according to focused-session semantics

### Double-click entry point

When `View as = Panes`, double-clicking the pane title text starts editing that pane's custom name.

Behavior rules:

- Double-clicking the title text of a pane row opens an inline editor in place of that title text.
- The editor is scoped to the pane whose title text was double-clicked.
- Double-clicking whitespace, metadata, badges, or the row background should continue to behave according to existing row behavior unless the implementation already treats those regions as part of the title hit target.
- Double-clicking the tab group header should continue to trigger tab rename, not pane rename.
- Double-clicking title text in `View as = Tabs` should continue to open the existing inline editor for renaming the tab, not the pane.
- Double-clicking a Summary tab item is not a pane rename entry point, because Summary represents the tab as a whole rather than a single pane title.

### Inline editor behavior

The pane rename editor should behave consistently with existing inline rename interactions in Warp.

Expected behavior:

- The editor is prefilled with the current custom pane name if one exists.
- If no custom pane name exists, the editor is prefilled with the currently displayed generated title for that pane item.
- Pressing `Enter` commits the edit.
- Pressing `Escape` cancels the edit and restores the previous displayed title.
- Blurring the editor commits the edit if the value changed.
- Submitting an unchanged value should leave state unchanged.
- Submitting only whitespace should clear the custom pane name and restore generated title behavior.
- Leading and trailing whitespace should be trimmed before saving.

Only one inline rename editor should be active in Vertical Tabs at a time. Starting a different rename action should close or replace the previous editor according to the existing rename-editor pattern.

### Custom-name display precedence

If a pane has a custom name, that name is always used as the pane item's main title text in Vertical Tabs.

This precedence applies regardless of the `Pane title as` setting:

- `Pane title as = Command / Conversation`
- `Pane title as = Working Directory`
- `Pane title as = Branch`

The generated `Pane title as` value should remain available as secondary or metadata text only where existing layouts already render it outside the main title position. The custom pane name should not erase the underlying command, conversation, working directory, or branch metadata from other configured metadata slots.

If the custom pane name is cleared, the pane item immediately returns to the title generated by the current `Pane title as` setting and pane metadata.

### Display scope

Custom pane names apply only to pane items in Vertical Tabs.

They should affect:

- pane rows in `View as = Panes`
- focused-session rows in `View as = Tabs`, because those rows are derived from an active pane
- search text for pane rows, so searching for the custom pane name can find the pane item

They should not affect:

- the pane header inside the main workspace area
- the tab group header title
- the horizontal tab strip
- details shown in the hover sidecar
- terminal titles reported by the shell
- shell integration metadata
- notebook, workflow, code, or file object titles outside the Vertical Tabs pane item
- Summary mode's tab-level generated primary line

### Relationship to `View as` modes

#### `View as = Panes`

Each visible pane row can have its own custom name.

When a custom name exists:

- that row's main title text shows the custom name
- the row's icon, badges, secondary text, metadata rows, hover state, selection state, and click-to-focus behavior are unchanged
- changing the `Pane title as` setting does not change the row's main title text
- clearing the custom name makes the row follow `Pane title as` again

#### `View as = Tabs` with `Tab item = Focused session`

The representative row is derived from the tab's active pane. If that active pane has a custom name, the representative row's main title text shows the custom pane name.

When the active pane changes:

- the representative row updates to the newly active pane
- if the newly active pane has a custom name, that custom name is shown
- if the newly active pane does not have a custom name, the row uses generated title behavior

The tab group header title is still the tab name, not the custom pane name.

Rename entry points in this mode remain tab-scoped:

- double-clicking title text opens the existing inline tab rename editor
- overflow / kebab rename actions rename the tab
- right-click menus expose tab rename, not pane rename

#### `View as = Tabs` with `Tab item = Summary`

Summary mode is a tab-level representation and is not primarily a pane item. Custom pane names should not replace the Summary card's generated tab-level primary line.

Summary mode should continue to summarize the tab based on its existing summary rules.

### Pane lifecycle behavior

Custom pane names should remain associated with the pane they were set on.

Expected behavior:

- Switching focus between panes does not move custom names between panes.
- Moving a pane within the same tab preserves its custom name.
- Moving a pane to another tab preserves its custom name.
- Splitting a pane creates a new pane without automatically copying the source pane's custom name unless existing split behavior already clones all pane metadata.
- Closing a pane removes its custom name with the pane.
- Closing a tab removes custom names for panes in that tab.
- Reopening Warp should preserve custom pane names for restored panes if those panes are part of persisted workspace state.

### Search behavior

Search in Vertical Tabs should include custom pane names.

Expected behavior:

- In `View as = Panes`, searching for a custom pane name matches and shows that pane row.
- In `View as = Tabs` with `Tab item = Focused session`, searching for the active pane's custom name matches that tab's representative row.
- Clearing a custom name removes that custom name from future search matching.
- Generated metadata should remain searchable according to existing rules.

### Empty states and errors

- If the user opens rename for a pane that no longer exists, the editor should not appear and no unrelated pane should be renamed.
- If the targeted pane closes while the editor is open, the editor should close without applying the edit to another pane.
- If saving fails, the UI should keep the previous title and avoid showing a stale custom name as if it succeeded.
- A blank or whitespace-only submitted name should clear the custom name rather than showing an empty pane title.

## Success criteria

1. Right-clicking a pane row in Vertical Tabs exposes a pane rename action.
2. Choosing the right-click pane rename action starts editing the pane that was right-clicked.
3. Overflow / kebab menus remain tab-group-level menus and do not expose pane rename.
4. The rename action in an overflow / kebab menu renames the tab, not a pane.
5. In `View as = Tabs`, rename entry points expose tab rename only.
6. In `View as = Panes`, double-clicking pane title text opens an inline editor for that pane's custom name.
7. Double-clicking the tab group header still renames the tab, not a pane.
8. The inline editor is prefilled with the current custom name when one exists.
9. The inline editor is prefilled with the currently displayed generated title when no custom name exists.
10. Pressing `Enter` commits a trimmed custom pane name.
11. Pressing `Escape` cancels without changing the displayed title or stored custom name.
12. Submitting an empty or whitespace-only name clears the custom pane name.
13. A pane with a custom name shows that name as its main title text in Vertical Tabs.
14. A custom pane name takes precedence over `Pane title as = Command / Conversation`.
15. A custom pane name takes precedence over `Pane title as = Working Directory`.
16. A custom pane name takes precedence over `Pane title as = Branch`.
17. Clearing a custom pane name immediately restores generated title behavior based on the current `Pane title as` setting.
18. Custom pane names do not change the pane header shown in the main workspace area.
19. Custom pane names do not change the tab group header or horizontal tab title.
20. In `View as = Tabs` with `Tab item = Focused session`, a representative row shows the active pane's custom name when that pane has one.
21. In `View as = Tabs` with `Tab item = Focused session`, changing the active pane updates the representative row to the newly active pane's custom name or generated title as appropriate.
22. In Summary mode, custom pane names do not replace the generated summary primary line.
23. Searching for a custom pane name finds the corresponding pane item in `View as = Panes`.
24. Searching for the active pane's custom name finds the focused-session representative row in `View as = Tabs`.
25. Closing a pane with a custom name does not leave a stale custom name visible on any other pane.
26. Details shown in the hover sidecar continue to use generated detail content and are not replaced by custom pane names.

## Validation

- **Right-click rename**: In `View as = Panes`, create a tab with at least two panes. Right-click the second pane row, choose pane rename, enter a custom name, and verify only that pane row changes.
- **Overflow rename regression**: Open the tab-group overflow / kebab menu, choose rename, and verify the tab is renamed rather than any pane.
- **Double-click rename**: In `View as = Panes`, double-click pane title text, enter a custom name, and verify the row title updates after commit.
- **Tab rename regression**: Double-click the tab group header and verify existing tab rename behavior still works. In `View as = Tabs`, double-click title text and verify it opens the tab rename editor. Verify pane rename does not change the tab group header title.
- **Precedence**: Set a custom pane name, then switch `Pane title as` between `Command / Conversation`, `Working Directory`, and `Branch`. Verify the pane row's main title remains the custom name.
- **Clear custom name**: Rename a pane to whitespace or otherwise clear the name. Verify the row returns to generated title behavior and updates when `Pane title as` changes.
- **Pane header and sidecar scope**: Rename a pane from Vertical Tabs and verify the pane header in the main workspace area and details shown in the hover sidecar do not show the custom name.
- **Focused-session mode**: Switch to `View as = Tabs` and `Tab item = Focused session`. Focus panes with and without custom names and verify the representative row updates correctly.
- **Summary mode**: Switch to `View as = Tabs` and `Tab item = Summary`. Verify the summary primary line is not replaced by a custom pane name.
- **Search**: Search for the custom pane name in Panes mode and focused-session Tabs mode and verify the expected row is matched.
- **Lifecycle**: Move a named pane to another tab, close named panes, and relaunch Warp with restored workspace state. Verify names stay attached to the intended panes and disappear when panes are gone.

## Open questions

1. Should the pane rename menu item be labeled `Rename Pane`, `Rename pane`, or reuse the existing tab rename label style with pane-specific wording?
2. If saving custom pane names requires persistence beyond existing workspace restoration, what persistence boundary should apply for panes created by ephemeral workflows that are not normally restored?
