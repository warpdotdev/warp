# APP-3656: Vertical Tabs Panel — Compact Mode + View Toggle

## Summary

Add a compact display mode for the vertical tabs panel and a settings icon button that opens a popup to switch between compact and expanded views. In compact mode, each pane is rendered as a single-line row (icon + title) instead of the current multi-line card layout.

## Problem

The current expanded pane rows show rich detail (working directory, branch, agent status, diff stats, PR badge) and occupy significant vertical space. When a user has many tabs and panes, the panel requires heavy scrolling. Users need a denser view that lets them quickly scan and switch between panes without scrolling past multiple lines of metadata per item.

## Goals

- Let users switch the vertical tabs panel between a dense compact view and the current detailed expanded view.
- Persist the user's preference across sessions.
- Add a control-bar icon button that opens a settings popup for toggling the view mode.

## Non-goals

- **Group-by**: The settings popup in the Figma mocks also contains "Group panes by" options (Tab, Directory/Environment, Branch, Status). These are out of scope for this ticket.
- **Compact group headers**: Group headers remain unchanged in both modes. Iterating on header layout is a separate concern.
- **Search functionality**: The search input in the control bar remains inert (per APP-3648).

## Figma / design references

- Compact view: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7079-25549&m=dev
- Compact view (settings button active): https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7079-24535&m=dev
- Expanded view + settings popup: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7103-34398&m=dev

**Intentional deviation from mocks**: In compact mode, non-agent terminal panes display the **terminal title** (e.g. process name, user-set title) instead of the working directory shown in the mock.

## User experience

### View modes

The vertical tabs panel supports two view modes:

- **Expanded** (default, current behavior): Multi-line pane rows with full metadata (working directory, git branch, conversation status, agent badge, diff stats, PR badge). This is the existing layout, unchanged.
- **Compact**: Single-line pane rows with just an icon and a title string. Described in detail below.

### Setting

A new user setting (`VerticalTabsViewMode`) controls which mode is active. It is a synced setting (cloud-persisted) with two variants: `Compact` and `Expanded`. Default is `Expanded`.

### Settings icon button

A new icon button is added to the existing control bar in the vertical tabs panel header, to the left of the existing Configs button and Plus button.

- **Icon**: `settings-04` (a sliders/filter icon, per the Figma mock).
- **Size**: 16×16 icon in a 20×20 hit target with 2px padding, matching other icon buttons in the control bar.
- **Behavior**: Clicking the button toggles the settings popup open/closed.
- **Active state**: When the popup is open, the button has a highlighted background (`fg_overlay_3`), matching the active state shown in mock 2.
- **Tooltip**: On hover (when popup is closed), show "View options".

### Settings popup

The popup appears anchored below the settings icon button.

- **Contents**: A segmented control with exactly two segments:
  - **Left segment**: Hamburger/list icon (`menu-01`), representing compact mode.
  - **Right segment**: Grid icon (`grid-01`), representing expanded mode.
- The segment corresponding to the current view mode is visually selected (highlighted background).
- Clicking a segment immediately switches the view mode setting and the panel re-renders.
- **Dismiss**: The popup closes when the user clicks outside it, presses Escape, or clicks the settings icon button again.
- **Positioning**: Anchored to the bottom edge of the settings button, left-aligned with it. If the popup would clip outside the window, reposition within window bounds.
- **Style**: Rounded container with border (`neutral_4`), subtle background, and drop shadow, matching the existing option menu pattern.

### Compact pane rows

In compact mode, each pane row within a tab group is a single-line element:

```
[icon 16×16] [4px gap] [title text, 12px, single line, ellipsis truncation]
```

With horizontal padding (12px left and right) and vertical padding (8px top and bottom) per row. Rows have 4px corner radius, consistent with the expanded layout.

#### Per-pane-type content

**Terminal pane (non-agent)**:
- Icon: Terminal icon (same as expanded tertiary line).
- Title: The terminal title from the shell (e.g. the running process name or user-set title). This intentionally differs from the mock, which shows the working directory.

**Terminal pane (agent session)**:
- Icon: Conversation status icon (the colored status badge — running, stopped, completed, etc.) at 16×16.
- Title: The conversation display title (e.g. "Refactor the button component to use..."). Truncated with ellipsis if it overflows.

**Terminal pane (ambient agent)**:
- Icon: `OzCloud` icon.
- Title: The conversation display title if available, otherwise the terminal title.

**Code pane**:
- Icon: Code file icon (language-specific if available, falling back to generic code icon).
- Title: The file name/path as shown in the pane title.

**Other panes** (Notebook, Workflow, Settings, Rules, Plan, MCP Server, etc.):
- Icon: The type-specific icon (same icon used in the expanded view's kind badge).
- Title: The pane configuration title.

#### Split panes (multiple visible panes in one tab group)

Each visible pane in a tab group gets its own single-line compact row, same as in expanded mode. There is no combined-row or "(+N)" treatment — the group collapse/expand chevron and the pane count label in the group header already communicate how many panes exist.

#### Row interactions

All interactions are identical to the expanded view:
- **Click**: Focus the pane (`WorkspaceAction::FocusPane`).
- **Right-click**: Open the tab right-click context menu.
- **Hover**: Highlight with `fg_overlay_1` background (or `fg_overlay_2` if selected). Cursor changes to pointing hand.
- **Selected state**: The focused pane in the active tab has a `fg_overlay_2` background and a 1px `fg_overlay_2` border.
- **Drag**: Pane rows are not individually draggable (only entire tab groups are draggable, unchanged).

#### Row indicators

- **Unsaved changes indicator**: For code panes with unsaved changes, show a small filled circle icon (`circle-filled`, 16×16) to the right of the title text, same as the expanded view's badge. This is right-aligned in the row.

#### Tab color support

Per-pane and per-group tab colors work the same as in expanded mode. In compact mode, the color applies as a background tint on the pane row (at `TAB_COLOR_OPACITY` / `TAB_COLOR_HOVER_OPACITY`).

### Group headers

Group headers are unchanged between compact and expanded modes. They continue to show:
- Left: Group title (uppercase, 10px, sub-text color)
- Right: Pane count label + collapse/expand chevron

The collapse/expand behavior for groups works the same in both modes. Collapsing a group hides all its pane rows (whether compact or expanded).

### Transitions

Switching between compact and expanded mode re-renders all pane rows immediately. No animation is required. The scroll position should be preserved as closely as possible (the scroll state handle is shared).

## Success criteria

1. A `VerticalTabsViewMode` setting with `Compact` and `Expanded` variants is persisted as a synced cloud setting.
2. The settings icon button appears in the control bar between the search input and the Configs button.
3. Clicking the settings icon button opens a popup with a two-segment control (compact/expanded). Clicking a segment switches the mode immediately.
4. The popup closes on outside click, Escape, or re-clicking the settings button.
5. In compact mode, every pane type renders as a single-line row with the correct icon and title per the rules above.
6. Non-agent terminal panes in compact mode show the terminal title (not the working directory).
7. Agent terminal panes in compact mode show the conversation status icon and conversation title.
8. Code panes show the unsaved-changes circle indicator when applicable.
9. Tab-color tinting works correctly on compact rows.
10. Group headers, collapse/expand, drag-to-reorder tabs, and right-click context menus all work unchanged in compact mode.
11. Switching modes preserves the scroll position and does not reset collapsed/expanded group states.
12. The setting defaults to `Expanded`, matching current behavior — no user-visible change until the user explicitly switches.

## Validation

- **Visual inspection**: Toggle between compact and expanded modes. Verify that compact rows are single-line, icons are correct per pane type, and text truncates with ellipsis.
- **Terminal title vs pwd**: Open a non-agent terminal, set a custom title or run a process, switch to compact mode, and verify the terminal title (not the pwd) is shown.
- **Agent panes**: Start an agent conversation, switch to compact mode, verify the status icon and conversation title are shown.
- **Unsaved code indicator**: Open a code file, make an unsaved edit, switch to compact mode, and verify the filled circle indicator appears.
- **Tab colors**: Assign a tab color, switch to compact mode, verify the color tint is visible on the compact row.
- **Settings popup**: Click the settings icon, verify the popup appears anchored below it with the correct segment selected. Click the other segment, verify the mode switches. Click outside, verify the popup closes.
- **Persistence**: Switch to compact mode, quit and relaunch, verify the panel opens in compact mode.
- **Narrow panel**: Resize the panel to minimum width (200px) in compact mode. Verify rows truncate gracefully and the control bar remains usable.
- **Group collapse**: Collapse a group in compact mode, switch to expanded, verify it remains collapsed.

## Open questions

None — all resolved:

1. ~~Should the compact view hide the group pane count label?~~ No. Keep it unchanged.
2. ~~Keyboard shortcut to toggle compact/expanded?~~ Out of scope for now; to be added later.
3. ~~Where does the compact/expanded toggle live when group-by is added?~~ It stays inside the same popup, per the Figma mock.
