# APP-3713: Vertical Tabs — Primary Info Selector in Settings Popup

## Summary

Replace the placeholder "Group panes by" section in the vertical tabs settings popup with a "Show first" selector that lets users choose which information appears on the primary (top) line of terminal pane rows. The two options are "Command / Conversation" (default, current behavior) and "Directory / Branch" (swaps the current primary and secondary lines). The selected option shows a checkmark. This applies to both expanded and compact view modes.

## Problem

The vertical tabs settings popup currently contains a "Group panes by" section with a single hardcoded "Tab" option that does nothing useful. This section occupies popup real estate without providing value. Meanwhile, users have different workflows: some care most about _what_ is running (the terminal command or agent conversation) while others care most about _where_ they are (the working directory and git branch). There is no way to customize which of these is most prominent in the pane row.

## Goals

- Let users choose whether terminal pane rows prioritize the terminal command / agent conversation title or the working directory / git branch as the primary line.
- Remove the non-functional "Group panes by" section from the settings popup.
- Persist the preference across sessions as a synced cloud setting.
- Apply the preference to both expanded and compact view modes.

## Non-goals

- Adding group-by functionality (the section being removed was a placeholder for future work; re-adding it is a separate concern).
- Changing non-terminal pane rows (code, notebook, settings, etc.) — this only affects terminal pane rows.
- Changing the segmented compact/expanded control in the popup (it remains unchanged below the divider).

## Figma / design references

Figma: none provided

## User experience

### Setting

A new user setting (`VerticalTabsPrimaryInfo`) controls which content appears on the primary line of terminal pane rows. It has two variants:

- **`Command`** (default): Terminal command / agent conversation title is the primary line. Working directory / git branch is the secondary line. This matches the current behavior.
- **`WorkingDirectory`**: Working directory / git branch is the primary line. Terminal command / agent conversation title is the secondary line.

The setting is a synced cloud setting (same sync behavior as `VerticalTabsViewMode`).

### Settings popup layout

The popup replaces the current "Group panes by" section. The new layout is:

```
┌──────────────────────────────────┐
│  Show first                      │  ← section header (sub-text color)
│  ✓  Command / Conversation       │  ← option (selected state shown)
│     Directory / Branch           │  ← option
│  ───────────────────────────     │  ← divider (unchanged)
│  [ compact ] [ expanded ]        │  ← segmented control (unchanged)
└──────────────────────────────────┘
```

#### Section header

- Text: **"Show first"**
- Styled identically to the current "Group panes by" header: sub-text color, 12px, 16px horizontal padding, 8px bottom margin.

#### Option items

Each option is a single row inside the popup. The currently selected option shows a checkmark icon on the left; the unselected option shows an empty space of the same width (so text stays aligned).

- **"Command / Conversation"**: When selected, terminal pane rows use the current primary/secondary line assignment (terminal command or agent conversation title on top, working directory on the second line).
- **"Directory / Branch"**: When selected, terminal pane rows swap their primary and secondary lines (working directory / git branch on top, terminal command or agent conversation title on the second line).

Clicking an option:
1. Updates the `VerticalTabsPrimaryInfo` setting immediately.
2. The panel re-renders with the new line order.
3. The popup **stays open** (so the user can see the checkmark move and the change take effect, then dismiss manually).

Each option row:
- Has 16px horizontal padding (matching the current "Tab" item).
- Shows the checkmark icon (16×16, main-text color) for the selected option, or a 16×16 transparent spacer for the unselected option.
- 8px gap between the icon/spacer and the label text.
- Label text is 12px in main-text color.
- Has a hover highlight (same `fg_overlay_1` pattern used elsewhere in the popup).
- Cursor changes to pointing hand on hover.

#### Divider and segmented control

Unchanged from the current implementation. The divider separates the "Show first" section from the compact/expanded segmented control.

### Effect on expanded terminal pane rows

#### When `Command` is selected (default — current behavior)

No change from today. The expanded terminal row layout remains:
1. **Primary line** (main text color): terminal title, agent conversation status + title, CLI agent title, or last completed command (per the existing precedence rules from APP-3651).
2. **Secondary line** (sub text color): working directory • git branch.
3. **Tertiary line**: kind badge + badges (unchanged).

#### When `WorkingDirectory` is selected

The primary and secondary lines swap:
1. **Primary line** (main text color): working directory • git branch. Uses the same layout as the current secondary line but rendered in main-text color. Working directory clips from the start; git branch clips from the end.
2. **Secondary line** (sub text color): terminal title, agent conversation status + title, CLI agent title, or last completed command (same precedence rules as the current primary line, but rendered in sub-text color). For agent conversations, the status indicator icon still precedes the title text.
3. **Tertiary line**: kind badge + badges (unchanged).

### Effect on compact terminal pane rows

#### When `Command` is selected (default — current behavior)

No change. The compact row shows the terminal icon + terminal title (or agent status icon + conversation title) as a single line.

#### When `WorkingDirectory` is selected

The compact row shows:
- **Non-agent terminal**: Terminal icon + working directory (instead of terminal title). The working directory clips from the start.
- **Agent terminal (Oz or CLI agent)**: Conversation status icon + working directory (instead of conversation title). The working directory clips from the start.
- **Ambient agent**: `OzCloud` icon + working directory.

#### Icon behavior (both modes)

The kind icon at the start of the compact row is always determined by the pane type and agent state, not by the primary info setting. A non-agent terminal always shows the terminal icon; an agent terminal always shows the conversation status icon; an ambient agent always shows the `OzCloud` icon. Only the *text* portion of the row changes when the setting is toggled.

### Non-terminal panes

Non-terminal pane rows (code, notebook, settings, etc.) are not affected by this setting in either view mode. Their layout remains unchanged.

### Search behavior

The search input already indexes both the primary text and the working directory for terminal panes. This behavior is unchanged — both fields remain searchable regardless of which is shown as the primary line.

## Success criteria

1. The "Group panes by" header and "Tab" item are removed from the settings popup.
2. A "Show first" header appears at the top of the popup with two options: "Command / Conversation" (checked by default) and "Directory / Branch".
3. Clicking "Directory / Branch" moves the checkmark to that option and immediately swaps the primary and secondary lines for all terminal pane rows.
4. Clicking "Command / Conversation" restores the default line order and moves the checkmark back.
5. In expanded mode with "Directory / Branch" selected: the primary line shows the working directory (start-clipped) and git branch in main-text color; the secondary line shows the terminal title or agent conversation title in sub-text color.
6. In expanded mode with "Directory / Branch" selected and an active agent conversation: the secondary line shows the conversation status indicator followed by the conversation title in sub-text color.
7. In compact mode with "Directory / Branch" selected: the single-line row shows the terminal icon + working directory (start-clipped) instead of the terminal title.
8. In compact mode with "Directory / Branch" selected and an agent conversation: the row shows the conversation status icon + working directory.
9. The setting persists across sessions. Quitting and relaunching with "Working directory" selected shows the same line order.
10. The popup stays open after clicking an option, allowing the user to see the change and dismiss manually.
11. The segmented compact/expanded control below the divider is unchanged.
12. Non-terminal panes are unaffected by the setting.

## Validation

- **Manual toggle**: Open the settings popup, switch between "Command / Conversation" and "Directory / Branch". Verify the pane rows update immediately and the checkmark moves.
- **Expanded mode**: With "Directory / Branch" selected, verify the primary line shows the working directory (start-clipped) + git branch in main-text color, and the secondary line shows the terminal title or agent info in sub-text color.
- **Compact mode**: Switch to compact mode with "Directory / Branch" selected. Verify the single-line row shows terminal icon + working directory.
- **Agent panes**: Start an agent conversation. With "Directory / Branch" selected, verify the expanded secondary line shows the status indicator + conversation title. In compact mode, verify the status icon + working directory.
- **Persistence**: Select "Directory / Branch", quit Warp, relaunch, and verify the setting is preserved.
- **Non-terminal panes**: Open a code pane or notebook. Verify changing the primary info setting has no effect on these rows.
- **Search**: With "Directory / Branch" as primary, search for a terminal title string. Verify it still matches (search indexes both fields regardless of display order).

## Open questions

None — all resolved. Agent panes swap uniformly with non-agent terminals (working directory becomes primary when "Directory / Branch" is selected).
