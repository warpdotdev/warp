# Tab Context Menu Copy Metadata — Product Spec
## Summary
Add copy actions to tab and vertical-tabs context menus for metadata Warp already knows about the visible tab or active pane.
## Problem
Vertical tabs surface useful metadata such as branch, working directory, and pull request link, but users must manually select or rederive that information when they want to share it elsewhere. The attached context menu is the natural place to expose quick copy actions because it is already where users manage tabs and active panes.
## Goals
- Show copy actions for branch, tab or pane title, working directory, and pull request link when the relevant metadata is visible and exists.
- Avoid disabled or empty menu items; if Warp does not have a value, omit the corresponding copy action.
- Use the same metadata sources that vertical tabs already use, so menu availability matches what Warp knows about the tab or pane.
- Support regular tab context menus, vertical-tabs tab context menus, and vertical-tabs pane context menus.
## Non-goals
- Fetch branch or pull request data synchronously when metadata is not already available.
- Add new settings or feature flags for the copy actions.
- Change how tab titles, branch labels, working directories, or pull request badges are rendered in vertical tabs.
- Add toast notifications for these copy actions.
## User Experience
When a user opens the context menu, Warp includes a copy metadata section if at least one copyable metadata value is available.
Available actions in vertical tabs:
- Copy branch
- Copy tab title
- Copy pane title
- Copy working directory
- Copy pull request link
The section appears only when one or more of those actions are present. Each item copies the corresponding raw value to the system clipboard. The menu keeps the existing separator behavior, so the copy metadata section is visually grouped with the rest of the menu.
For horizontal tab context menus, only Copy tab title is shown because the other metadata is not visible in that layout. For vertical tabs grouped by tabs, terminal-specific metadata comes from the focused terminal session in the tab and the title action is Copy tab title. For vertical tabs grouped by panes, terminal-specific metadata comes from the active pane and the title action is Copy pane title.
## Success Criteria
1. A vertical tab or pane with a known branch shows Copy branch, and selecting it copies the branch name.
2. A tab with a non-empty display title shows Copy tab title, and selecting it copies that title.
3. A vertical pane with a non-empty pane title shows Copy pane title, and selecting it copies that title.
4. A vertical terminal tab or pane with a known working directory shows Copy working directory, and selecting it copies the directory.
5. A vertical terminal tab or pane with a known pull request URL shows Copy pull request link, and selecting it copies the URL.
6. Copy actions are omitted individually when their metadata is unavailable, empty, or not visible in the current layout.
7. Existing context menu actions and separators continue to behave as before.
## Validation
- Manually verify the horizontal tab context menu only includes the tab-title copy item.
- Manually verify the vertical-tabs tab context menu on a terminal in a git repository with and without a pull request chip.
- Manually verify the vertical-tabs pane context menu with pane grouping enabled.
- Run formatting and targeted integration tests for the affected flows.
