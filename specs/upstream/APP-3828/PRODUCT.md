# APP-3828: Vertical Tabs v2 — View as Panes / Tabs

## Summary

Add a new `View as` control to the vertical tabs display options popup with two modes:

- **Panes**: the current behavior, where each pane is rendered as its own item under its tab.
- **Tabs**: a new overview mode where each tab renders exactly one item, using that tab’s active pane as the representative row.

This first iteration only ships the `View as` toggle and the `Focused session` behavior implicitly. It does not yet add a separate Tabs-only naming control such as `Summary`.

## Problem

The current vertical tabs panel is pane-centric. That works well when a user wants fine-grained visibility into every split, but it becomes noisy when tabs contain multiple panes and the user is trying to scan the workspace at the tab level.

Users need a higher-level overview mode that reduces each tab to a single representative item without introducing a brand-new visual language. The new mode should preserve the current tab structure and reuse the existing row UI so the first iteration is easy to understand and low-risk to ship.

## Goals

- Add a new `View as` setting in the vertical tabs popup with `Panes` and `Tabs` options.
- Preserve the current behavior as the default via `View as = Panes`.
- Introduce `View as = Tabs`, where each tab renders one representative row derived from that tab’s active pane.
- Reuse the existing compact and expanded pane row UI for the representative row rather than inventing a new tab row design.
- Keep the existing tab group/header structure and interactions intact in Tabs mode.
- Persist the `View as` preference across sessions as a synced setting.

## Non-goals

- Shipping the future Tabs-only `Default name` section from the exploratory mock.
- Shipping a `Summary` naming mode or any other alternative tab naming strategy.
- Flattening the panel into a headerless list of tabs.
- Redesigning tab group headers, close affordances, rename behavior, or drag-and-drop behavior.
- Changing the current `Density`, `Pane title as`, `Additional metadata`, or `Show` controls beyond ensuring they continue to work with the reused active-pane row.

## Figma / design references

- Popup exploration: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7628-119835&t=LSuxL7FNk3EXOfvJ-0
- Tabs-selected popup state: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7628-123042&t=LSuxL7FNk3EXOfvJ-0

### Intentional deviation from the exploratory mock

The exploratory Tabs-selected popup includes a Tabs-only `Default name` section with `Focused session` and `Summary`.

This iteration intentionally does **not** ship that extra section. `Focused session` is the only supported Tabs behavior and is implicit when `View as = Tabs`.

## User experience

### Setting

The vertical tabs popup gains a new top-level setting:

- **View as: Panes**
- **View as: Tabs**

`Panes` is the default.

The control is rendered as a two-segment toggle at the top of the popup, above the existing display controls.

### Popup behavior

- Clicking `Panes` or `Tabs` updates the panel immediately.
- The popup remains open after switching modes so the user can see the change in context.
- The existing `Density` and other pane-row display controls remain present in this iteration.
- Those pane-row controls remain accessible in both `Panes` and `Tabs` modes, because the `Tabs` row is still a pane-style row derived from the active pane.
- Switching between `Panes` and `Tabs` does not reset the current density or any existing pane-row display preferences.

### Panes mode

When `View as = Panes`, the panel behaves exactly as it does today:

- each visible pane is rendered as its own item
- items are grouped under their tab header
- compact vs expanded density works as it does today
- existing pane-row display preferences continue to apply as they do today

No visual or behavioral change should be introduced in Panes mode beyond the existence of the new `View as` control in the popup.

### Tabs mode

When `View as = Tabs`, the panel stays tab-grouped, but each tab group renders exactly one representative row instead of one row per visible pane.

#### Representative row source

The representative row is always derived from the tab’s **active pane**.

In this iteration, `View as = Tabs` therefore means the tab item is effectively named and styled using the tab’s **focused session**.

#### Representative row appearance

The representative row reuses the existing pane-row UI for the active pane:

- in **compact density**, it uses the same compact row renderer the active pane would use in Panes mode
- in **expanded density**, it uses the same expanded row renderer the active pane would use in Panes mode

This includes the same icon rules, title rules, subtitle/metadata rules, badges, truncation behavior, and selection styling that already apply to the active pane’s row in Panes mode.

Because the representative row is still a pane-style row, the existing pane-row display controls continue to apply in `Tabs` mode as well. In this iteration that means:

- `Pane title as` still changes how a terminal representative row is labeled
- `Additional metadata` still affects compact terminal representative rows
- `Show` still affects expanded terminal representative rows

These controls are not hidden when `View as = Tabs`.

#### Representative row updates

The representative row updates immediately whenever the active pane for that tab changes. Examples:

- the user changes focus between split panes inside the tab
- the active pane is closed and a different pane becomes active
- the active pane’s displayed metadata changes (for example, terminal title, branch, badges, or unsaved state)

Tabs mode should always reflect the tab’s current active pane, not the pane that happened to be active when the user first switched into Tabs mode.

#### Relationship to tab headers

Tabs mode does **not** remove or redesign the existing tab group header.

The current tab group header behavior remains intact, including:

- tab title display
- pane count display
- rename behavior
- close behavior
- drag-and-drop behavior
- existing header context menu behavior

The change in Tabs mode is only the number of rows rendered beneath each header: one representative row per tab instead of one row per pane.

#### Single-pane tabs

For tabs that only contain one visible pane, Tabs mode and Panes mode look effectively the same below the header, because the active pane is also the only pane.

#### Multi-pane tabs

For tabs that contain multiple visible panes:

- **Panes mode** renders one item per visible pane
- **Tabs mode** renders one item total for that tab, based on the active pane only

Non-active panes in the tab do not get their own rows in Tabs mode.

### Interaction behavior in Tabs mode

The representative row remains actionable in the same spirit as the active pane row it reuses:

- clicking the row activates that tab and focuses its active pane
- selection/highlight state continues to represent the active tab / focused pane as it does today

This iteration should not introduce new row-specific interactions unique to Tabs mode.

### Search behavior

Search/filtering operates on the items currently rendered in the chosen mode.

That means:

- in **Panes mode**, matching remains pane-based as it is today
- in **Tabs mode**, matching is based on each tab’s representative row only

In this first iteration, a non-active pane that is hidden by Tabs mode does not create its own separate match result.

## Success criteria

1. The display options popup shows a new top-level `View as` segmented control with `Panes` and `Tabs`.
2. `Panes` is selected by default, so existing users see no change in the panel until they opt into `Tabs`.
3. Switching to `Tabs` updates the panel immediately without requiring the popup to close.
4. In `Tabs` mode, each tab group renders exactly one row beneath its header.
5. The row shown for a tab in `Tabs` mode is derived from that tab’s current active pane.
6. If the active pane changes within a tab, the representative row updates immediately to reflect the newly active pane.
7. A tab with only one visible pane looks the same in `Panes` and `Tabs` modes below the header.
8. A tab with multiple visible panes shows multiple rows in `Panes` mode and exactly one row in `Tabs` mode.
9. The representative row in `Tabs` mode reuses the same compact or expanded row UI, icons, metadata, badges, and truncation rules the active pane already uses in `Panes` mode.
10. Existing tab header behavior remains unchanged in `Tabs` mode, including pane count, close, rename, and drag behavior.
11. Existing `Density` and pane-row display preferences continue to apply after switching between `Panes` and `Tabs`.
12. `Pane title as`, `Additional metadata`, and `Show` remain visible and usable in `Tabs` mode, and they continue to affect the representative row.
13. The `View as` preference persists across app relaunches as a synced setting.
14. Tabs mode does not surface `Summary` or any other alternate naming mode in this iteration.
15. In search/filter mode, Tabs mode returns matches for representative rows only, not hidden non-active panes.

## Validation

- **Popup toggle**: Open the display options popup and verify `View as` appears above the existing controls. Toggle between `Panes` and `Tabs` and verify the panel updates immediately while the popup remains open.
- **Default behavior**: With the default setting, verify the panel still renders one row per visible pane exactly as before.
- **Single-pane tab**: Open a tab with one pane, switch between `Panes` and `Tabs`, and verify there is no meaningful change below the header.
- **Multi-pane tab**: Create a tab with multiple split panes. Verify `Panes` shows all pane rows and `Tabs` shows exactly one row for that tab.
- **Active pane switching**: In a multi-pane tab, switch focus between panes and verify the representative row in `Tabs` mode updates to match the newly active pane.
- **Density coverage**: Verify the representative row works in both compact and expanded density modes.
- **Row parity**: For a given active pane, compare its appearance in `Panes` mode vs `Tabs` mode and verify the row content matches.
- **Pane-row controls in Tabs mode**: With `View as = Tabs`, change `Pane title as`, `Additional metadata`, and `Show`, and verify they still affect the representative row rather than disappearing.
- **Header regression**: In `Tabs` mode, verify header rename, close, pane count, drag behavior, and context menu behavior still work.
- **Persistence**: Select `Tabs`, relaunch Warp, and verify the panel reopens in `Tabs` mode.
- **Search**: In a multi-pane tab, ensure only the active pane’s representative row is matched and rendered in `Tabs` mode.

## Open questions

None for this iteration.
