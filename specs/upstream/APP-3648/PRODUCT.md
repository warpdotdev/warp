# APP-3648: Vertical Tabs Panel — Search + Control Bar

## Overview

Add a fixed control bar at the top of the vertical tabs panel containing a search input and a "new tab" button. This bar sits above the scrollable tab groups and is always visible when the panel is open.

## Milestone 1 Scope

- **Search input**: Renders as a full-width text field with placeholder text (e.g. "Search tabs...") and a magnifying glass icon. **Non-functional** — the input does not accept text and is not focusable. Hovering anywhere on the search input shows a tooltip: **"Not yet implemented"**.
- **Plus button**: A single icon button (`+`) to the right of the search input. Left-click creates a new tab (equivalent to `WorkspaceAction::AddTab`). Right-click opens the new session dropdown menu (equivalent to `WorkspaceAction::ToggleNewSessionMenu`).
- **Configs button**: Out of scope for this milestone.

## Behavior

### Search Input
- Visually resembles a standard search field: magnifying glass icon on the left, placeholder text, themed background/border consistent with the panel.
- The field is **inert** — clicking it does nothing (no focus, no cursor, no text entry).
- On hover, a tooltip appears with the text **"Not yet implemented"**.
- The search input stretches to fill all available horizontal space to the left of the plus button.

### Plus Button
- Renders as a small icon button with a `+` icon.
- **Left-click**: Creates a new tab (same as the existing `AddTab` action — opens a welcome tab or terminal tab depending on feature flags).
- **Right-click**: Opens the new session dropdown menu, anchored below the plus button at the bottom edge of the control bar.
- On hover, shows a tooltip with label **"New Tab"** and the keybinding for the new tab action (Cmd+Shift+T / Ctrl+Shift+T).
- The dropdown menu contains the same items as the existing new session menu (shell options, launch configs, tab configs).

### Layout
- The control bar is a horizontal row at the top of the vertical tabs panel, above the scrollable tab group list.
- It is **not scrollable** — it stays fixed at the top as tab groups scroll.
- It has horizontal padding consistent with the rest of the panel (12px).
- Vertical padding provides comfortable spacing from the panel's top edge and the first tab group below.

## Edge Cases

1. **Narrow panel widths**: At the minimum panel width (200px), the search input text truncates but the plus button remains fully visible and usable. The plus button has a fixed size; the search input absorbs all remaining width.
2. **Tooltip clipping**: The "Not yet implemented" tooltip and the new session dropdown menu must not clip outside the window bounds. Use window-aware positioning.
3. **Focus**: Since the search input is inert, clicking it must **not** steal focus from the active terminal or editor. The control bar itself does not participate in focus management.
4. **Panel toggle**: The control bar is only visible when the vertical tabs panel is open. No special behavior on open/close.
5. **Dropdown menu lifecycle**: The new session dropdown menu follows the same open/close lifecycle as the existing top bar menu — it closes on item selection, clicking outside, or pressing Escape.
6. **Multiple windows**: Each window's vertical tabs panel has its own independent control bar instance. Dropdown menus are scoped to their window.
