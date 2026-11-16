# APP-3832: Vertical Tabs v2 — Hover Detail Sidecar

## Summary

Add a hover-activated detail sidecar to the vertical tabs panel that shows full, un-elided information for the currently hovered item without changing focus.

- When `View as = Panes`, hovering an eligible pane row shows a single pane-scoped sidecar for that pane.
- When `View as = Tabs`, hovering an eligible tab representative row shows a tab-scoped sidecar composed of one pane-scoped section per visible pane in that tab.

In this first iteration, the sidecar supports terminal / agent terminal panes, code panes, and supported Warp Drive object panes. It is hover-only; keyboard focus and selection do not open it.

## Problem

The vertical tabs panel intentionally compresses information so it stays scannable. That compression creates two gaps:

- even in `View as = Panes`, important metadata is clipped or omitted in the row itself
- in `View as = Tabs`, non-active panes disappear from the panel entirely, so the user cannot quickly inspect the rest of a split tab without focusing it

Users need a way to inspect full pane detail from the panel itself without changing focus, opening the tab, or losing the higher-level overview that `View as = Tabs` provides.

## Goals

- Show full, un-elided detail for the currently hovered vertical-tabs item.
- Preserve the user’s current focus; hover detail must not activate tabs or panes just by appearing.
- Make `View as = Tabs` inspectable by exposing all visible panes in the hovered tab.
- Reuse a single pane-scoped detail pattern in both modes so the UI feels consistent.
- Keep the vertical tabs panel layout stable; the detail view should not resize the panel or the workspace.

## Non-goals

- Opening the sidecar from keyboard focus, keyboard navigation, or selection state.
- Adding support for the remaining pane types that still do not render a sidecar in this iteration.
- Changing the existing `View as`, `Density`, `Pane title as`, `Additional metadata`, or `Show` settings.
- Adding new tab naming or tab summarization behavior.
- Turning the sidecar into a general-purpose inspector with editable controls.

## Figma / design references

- Tabs-scoped sidecar mock: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7633-130645&t=LSuxL7FNk3EXOfvJ-0
- Pane-scoped sidecar mock: https://www.figma.com/design/n5d1rK2dMGKqf97XDBN1BV/Agents-mgmt?node-id=7029-53964&m=dev

There is no separate code-pane mock. In this iteration, code panes reuse the same pane-scoped sidecar shell and follow the code-specific content rules defined below.

## User experience

### General behavior

- The detail sidecar is a floating overlay anchored to the right side of the vertical tabs panel.
- It does not resize the vertical tabs panel or the main workspace content.
- Only one detail sidecar is shown at a time.
- The sidecar opens only from pointer hover over an eligible item.
- Keyboard focus, selected state, and programmatic pane activation do not open the sidecar on their own.
- Hovering a different eligible item updates the sidecar to the newly hovered item.
- Moving the pointer from the hovered item into the sidecar keeps the sidecar open; it must not flicker closed during that cursor transition.
- The sidecar closes when the pointer is no longer over either the source item or the sidecar.

### Eligibility

In this first iteration, the sidecar is supported for:

- plain terminal panes
- Oz agent terminal panes
- CLI agent terminal panes
- code panes
- notebooks and plans
- workflows
- environment variable collections
- rules
- MCP servers

The sidecar is not shown for the remaining pane types in this iteration.

### Panes mode

When `View as = Panes`, hovering an eligible pane row opens a pane-scoped sidecar for that exact pane.

- The sidecar contains a single pane-scoped section.
- There is no extra tab-level wrapper, heading, or pane-count summary.
- Hovering an unsupported pane type shows no sidecar.

### Tabs mode

When `View as = Tabs`, hovering an eligible tab representative row opens a tab-scoped sidecar for that tab.

- The sidecar contains one pane-scoped section per visible pane in the hovered tab.
- Sections appear in the same top-to-bottom pane order the tab uses internally; the sidecar must not reorder panes just because one is active.
- Each section uses the same pane-scoped layout as the single-pane sidecar from `View as = Panes`.
- The tab-scoped sidecar is purely a stacked collection of pane sections; it does not introduce a separate tab summary header in this iteration.

Because this first iteration only supports the pane types listed above, a tab-scoped sidecar is shown only when every visible pane in the hovered tab is one of those supported types. Tabs containing any other visible pane show no sidecar in this iteration.

### Sidecar shell and sizing

- The sidecar uses a fixed-width card-like container matching the Figma mocks.
- The sidecar has a bounded maximum height based on the available window height; it must not grow off-screen just because a hovered tab contains many panes.
- When the sidecar content exceeds that bounded height, the sidecar content area scrolls vertically within the card.
- The vertical tabs panel itself does not scroll as a side effect of interacting with the sidecar; overflow is handled inside the sidecar.
- Text inside the sidecar wraps as needed; it is not ellipsized just because the corresponding row in the panel is clipped.
- In tabs mode, pane sections are separated by dividers.
- In panes mode, the single pane section uses the same shell but without internal section dividers.

### Relationship to existing display settings

The sidecar is a fixed detail view, not a larger version of the row.

- `Density` does not change the sidecar layout.
- `Pane title as` does not reorder the sidecar fields.
- `Additional metadata` does not hide or swap sidecar fields.
- `Show` does not remove sidecar badges that are part of the fixed detail layout.

The sidecar’s job is to expose the pane’s full supported detail, even when the row is configured to prioritize different information.

### Terminal and agent pane sections

Terminal and agent panes use the pane-scoped layout shown in the mocks.

#### Content order

From top to bottom, a terminal / agent pane section shows:

1. An optional agent-status pill when the pane is an agent session and a conversation status is available (for example `Working` or `Done`).
2. The full working directory, if available.
3. A git-branch row with branch icon and full branch name, if available.
4. The full command / conversation text as the primary descriptive body text.
5. A metadata row containing the pane kind badge on the left and any supported badges on the right.

#### Content rules

- Plain terminal panes omit the status pill.
- Agent panes show the status pill above the directory / branch block.
- The command / conversation text uses the same identity precedence as the row’s terminal title logic, but without clipping:
  - agent conversation title when present
  - CLI agent title when present
  - terminal title when present
  - last completed command text when needed
  - final fallback text such as `New session`
- The working directory and branch are shown as separate rows when available; missing values are omitted rather than replaced with placeholder text.
- The metadata row always includes the pane kind badge (`Terminal`, `Oz`, `Claude Code`, etc.).
- If diff stats are available, show the diff-stats badge in the metadata row.
- If a pull request link is available, show the PR badge in the metadata row.

#### Interaction rules

- The diff-stats badge keeps its current behavior: clicking it opens the code review flow for that pane.
- The PR badge keeps its current behavior: clicking it opens the PR link.
- Clicking empty space inside the pane section does not focus the pane or tab.

### Code pane sections

Code panes use the same pane-scoped shell, but with code-specific content.

#### Content order

From top to bottom, a code pane section shows:

1. The full active file name as the primary line.
2. The full parent directory / path for that file.
3. An optional additional line when the pane has multiple open files, using the same underlying multi-file summary concept as the vertical-tabs row (for example `and N more` when relevant).
4. A metadata row containing the `Code` kind badge and any supported dirty-state indication.

#### Content rules

- Code pane text is never intentionally ellipsized inside the sidecar; wrap instead.
- If the pane has unsaved changes, surface that dirty state in the section using the existing unsaved indicator treatment.
- Do not show clean-state filler text such as `No unsaved changes`.
- Terminal-specific content such as status pills, git branch, diff stats, and PR badges is not shown for code panes in this iteration.

### Selection and focus behavior

- Hovering an item opens the sidecar without changing which pane or tab is focused.
- Clicking the original row keeps its existing behavior; the presence of the sidecar does not change row activation semantics.
- In tabs mode, the sidecar may show sections for non-focused panes in the hovered tab, but it does not change which pane is active.

### Empty and unsupported states

- If a supported pane is missing some metadata, omit the missing row rather than showing placeholder copy.
- If the hovered item is unsupported, show no sidecar.
- If a hovered tab in `View as = Tabs` contains any unsupported visible pane, show no sidecar for that tab in this iteration.

## Success criteria

1. Hovering an eligible item in the vertical tabs panel shows a floating detail sidecar without changing focus.
2. The sidecar is hover-only; keyboard focus or selection alone does not open it.
3. In `View as = Panes`, hovering an eligible pane row shows exactly one pane-scoped section.
4. In `View as = Tabs`, hovering an eligible representative row shows one pane-scoped section per visible pane in that tab.
5. The tab-scoped sidecar preserves the tab’s pane order rather than reordering sections around the active pane.
6. The sidecar stays open while the pointer moves from the source item into the sidecar and closes only after the pointer leaves both regions.
7. Terminal and agent pane sections show full, un-elided directory, branch, command / conversation text, and metadata badges when those fields exist.
8. Agent pane sections show a status pill when conversation status is available; plain terminal sections do not.
9. Code pane sections show full file and path information, plus dirty-state information when applicable, without terminal-specific metadata.
10. Supported Warp Drive object panes show their full title and pane-kind metadata in the sidecar.
11. `Density`, `Pane title as`, `Additional metadata`, and `Show` do not rearrange or remove the sidecar’s fixed detail layout.
12. Hovering an unsupported pane type shows no sidecar.
13. In `View as = Tabs`, a tab containing any unsupported visible pane shows no sidecar in this iteration.
14. Clicking the diff-stats or PR badge inside a terminal / agent sidecar section preserves the badge’s existing action.
15. The sidecar never resizes the vertical tabs panel or main workspace content.
16. When a tab-scoped sidecar is taller than the available window space, the sidecar remains bounded in height and becomes internally scrollable instead of extending off-screen.

## Validation

- **Panes mode / terminal**: Hover a plain terminal row and verify a single pane-scoped sidecar appears with full working directory, branch, command text, kind badge, and any available diff / PR badges.
- **Panes mode / agent**: Hover an Oz or CLI agent row and verify the sidecar shows the status pill, full conversation text, and terminal metadata without clipping.
- **Panes mode / code**: Hover a code row and verify the sidecar shows the full filename and path, plus dirty-state indication when applicable.
- **Panes mode / Warp Drive object**: Hover a supported notebook, plan, workflow, environment-variable collection, rule, or MCP server row and verify the sidecar shows the full title with the correct kind badge.
- **Tabs mode / multi-pane tab**: Hover a tab representative row for a tab with multiple supported panes and verify the sidecar shows one section per visible pane in the same pane order as the tab.
- **Large multi-pane tab**: Hover a representative row for a tab with enough supported panes to exceed the available vertical space and verify the sidecar stays bounded and scrolls internally.
- **Focus preservation**: Hover items and verify the currently focused pane does not change until the user explicitly clicks a row or an existing interactive badge.
- **Cursor transition**: Move the pointer diagonally from a row into the sidecar and verify the sidecar does not flicker closed.
- **Unsupported panes**: Hover an unsupported pane type in `View as = Panes` and verify no sidecar appears.
- **Mixed-type tab**: Hover a tab in `View as = Tabs` that contains at least one unsupported visible pane and verify no sidecar appears.
- **Settings independence**: Change `Density`, `Pane title as`, `Additional metadata`, and `Show`, then hover the same item and verify the sidecar layout and field order remain fixed.
- **Badge actions**: In a terminal / agent sidecar, click the diff-stats badge and PR badge and verify they perform the same actions they do from the row.

## Open questions

None for this iteration.
