# APP-3875: Vertical Tabs v2 — Summary Tab Item Mode

## Summary

Add a new `Summary` tab item mode for vertical tabs when `View as = Tabs`.

This is a follow-up to APP-3828. `Focused session` remains the default tab item for `View as = Tabs`, while `Summary` provides a tab-level overview of the work inside the tab. In Summary mode, each tab renders as a fixed expanded-style summary card with:

- a primary line derived from conversation / command labels across the tab
- a second line derived from working directories across the tab
- branch lines derived from the tab's unique branch contexts, with diff stats and PR chips shown on the branch line rather than on pane rows

## Problem

The current `View as = Tabs` mode is still fundamentally pane-scoped. It reduces each tab to a single representative row, but that row is just the focused pane rendered with the existing pane-row UI.

That works when the user only cares about the current focused pane, but it breaks down when a tab contains multiple active panes or multiple related workflows. In that case, the representative row hides the rest of the tab's work:

- the row title only describes the focused pane
- the working directory only describes the focused pane
- diff stats and PR chips only describe the focused pane's branch context
- the tab gives no concise overview of the other work happening inside it

Users need a second Tabs-mode representation that answers "what is inside this tab?" rather than only "which pane in this tab is focused right now?"

## Goals

- Add a new `Tab item` section directly under `View as` in the vertical tabs settings popup.
- Make `Tab item` available only when `View as = Tabs`.
- Keep `Focused session` as the default tab item mode for `View as = Tabs`.
- Add `Summary` as a second tab item mode.
- Make `Summary` render a tab-level summary card rather than a focused-pane row.
- Make `Summary` hide `Density` and the other focused-pane row controls that do not apply to the summary card.
- Define the summary card in terms of:
  - work labels on the first line
  - working directories on the second line
  - branch lines below, with diff stats and PR chips keyed to the branch context
- Keep the summary card stable and predictable rather than heuristic-heavy.
- Preserve the user's previous focused-session display settings when they temporarily switch into and out of `Summary`.

## Non-goals

- Adding `Summary` to `View as = Panes`.
- Adding a compact-density summary layout.
- Making branch lines or chips independently clickable.
- Replacing branch lines with pane-preview lines.
- Introducing fuzzy matching, semantic merging, or aggressive rewriting of work labels.
- Redesigning tab headers, drag-and-drop, rename behavior, or the existing tab-level hover sidecar.
- Reworking the existing `Focused session` behavior beyond adding the new `Tab item` control next to it.

## Figma / design references

- Popup exploration: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7628-124093&t=LSuxL7FNk3EXOfvJ-0
- Summary card exploration: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7633-127452&t=LSuxL7FNk3EXOfvJ-0

### Intentional deviations from the exploratory mocks

- `Summary` is a Tabs-only `Tab item` mode, not a density option.
- When `Summary` is selected, `Density` is hidden rather than adapted.
- The lower lines in Summary mode are branch lines, not pane-preview lines.
- Branch rows are grouped by branch context rather than rendered once per pane.

## User experience

### Popup structure

The vertical tabs settings popup keeps `View as` as its top section:

- `Panes`
- `Tabs`

When `View as = Panes`:

- the `Tab item` section is hidden
- the existing pane-oriented controls continue to behave as they do today

When `View as = Tabs`:

- a `Tab item` section appears directly under `View as`
- the `Tab item` section has exactly two options:
  - `Focused session`
  - `Summary`

`Focused session` is the default tab item mode for `View as = Tabs`.

### Popup state transitions

- Clicking `Focused session` or `Summary` updates the panel immediately.
- The popup remains open after changing `Tab item`.
- Switching from `Focused session` to `Summary` hides:
  - `Density`
  - `Pane title as`
  - `Additional metadata`
  - `Show`
- Switching back from `Summary` to `Focused session` restores those sections exactly as they were before; Summary does not overwrite the stored values for those controls.
- Switching from `Tabs` back to `Panes` hides `Tab item` entirely.
- If the user later returns to `View as = Tabs`, Warp restores the last selected `Tab item` mode.

### Focused session behavior

When `View as = Tabs` and `Tab item = Focused session`, behavior remains the same as APP-3828:

- each tab renders one representative row
- the representative row is derived from the focused pane in that tab
- `Density`, `Pane title as`, `Additional metadata`, and `Show` continue to affect that row exactly as they do today

This mode remains the default Tabs behavior.

### Summary behavior

When `View as = Tabs` and `Tab item = Summary`, each tab renders a fixed expanded-style summary card instead of a focused-pane row.

The summary card represents the tab as a whole, not any specific pane.

Summary mode does not expose density variants. It always uses the Summary card layout described below.

### Summary card structure

Each summary card contains up to four regions, in this order:

1. primary line
2. working-directory line
3. up to three visible branch lines
4. optional overflow line (`+ N more`)

The card may omit regions that have no data. For example:

- a tab with no branch context has no branch lines
- a tab with no working-directory data has no working-directory line

The absence of a later region does not insert placeholder text or empty rows.

### Summary card icon

Summary mode renders the pane circle icon as a tab-level pane-kind summary instead of using only the focused pane's icon.

- If all visible panes in the tab are the same pane kind, render one icon for that kind.
- Terminal panes are distinguished by their semantic terminal icon treatment, so plain terminals, Oz/ambient-agent terminals, and CLI-agent terminals do not all collapse to the generic terminal icon.
- If the tab has two or more pane kinds, render two icons using the same stacked arrangement as the current agent/status icon treatment:
  - the oldest pane kind is the main icon
  - the second-oldest distinct pane kind is the smaller secondary icon in the bottom-right
- Choose pane kinds by sorting the tab's visible panes by pane creation order, then taking the first two distinct pane kinds.
- If the oldest pane is closed, recompute from the remaining visible panes.
- Ignore any additional pane kinds beyond the first two distinct kinds for the icon treatment.

### Primary line

The primary line summarizes the work happening in the tab.

It is derived from work labels gathered across the tab's visible panes in tab order.

#### Work-label sources

For terminal-like panes, the work label should prefer conversation / command-oriented text, using the same general precedence as the current focused-session labeling:

- CLI agent display title, if present
- conversation display title, if present
- terminal title, if it is meaningful
- last completed command, if present
- otherwise the terminal fallback label such as `New session`

For non-terminal panes, the work label falls back to the pane title or pane-type label when no conversation / command label exists.

#### Normalization rules

Work-label normalization is intentionally conservative:

- trim leading and trailing whitespace
- collapse repeated internal whitespace
- drop empty labels
- dedupe exact-equivalent normalized labels while preserving the first-seen display text

Summary mode must not:

- semantically rewrite labels
- fuzzy-match different labels
- merge distinct labels like `cargo test` and `cargo + code review`
- treat different agent names as interchangeable

#### Rendering rules

- Render the first four unique work labels in first-seen order.
- Join them with ` • `.
- If more than four unique labels remain, append ` + N more`.
- The line is single-line and truncates visually when it does not fit.

Examples:
- `Claude • Oz • cargo • code review`
- `Claude • cargo • code review + 2 more`

### Working-directory line

The second line summarizes where the work in the tab is happening.

- Gather unique working directories exposed by panes in first-seen order.
- Render them as a list separated by ` • `.
- The line is single-line and truncates visually when it does not fit.
- If no pane exposes working-directory data, omit the line.

Warp should not try to pick one "most representative" directory in Summary mode. The card should show the stable list of unique working directories instead.

Examples:

- `~/warp-internal`
- `~/warp-internal • ~/warp-server • ~/warp-terraform`

### Branch lines

The lower lines in Summary mode are branch lines, not pane-preview lines.

Each branch line represents a unique branch context present in the tab and is the place where diff stats and PR chips appear in Summary mode.

#### Branch grouping

Branch lines are coalesced by unique repository + branch context.

That means panes from different repositories do not collapse into one branch line merely because they share the same branch name, such as `main`.

#### Branch-line ordering

- Branch lines are ordered by first appearance in the tab's visible pane order.
- Render at most three branch lines.
- If more than three unique branch contexts exist, render an overflow line below them:
  - `+ N more`

#### Branch-line contents

Each visible branch line shows:

- the branch label on the left
- diff stats on the right, if available for that branch context
- a PR chip on the right, if available for that branch context

A branch line may show:

- just the branch label
- the branch label plus diff stats
- the branch label plus PR chip
- the branch label plus both diff stats and PR chip

If a pane contributes no branch context, it does not create a branch line.

#### Coalesced metadata behavior

Within one coalesced repository + branch group:

- diff stats are shown once for the group
- the PR chip is shown once for the group

If multiple panes in the same coalesced group expose the same logical branch metadata, Warp should simply coalesce them into that one rendered line rather than treating them as separate rows.

### Summary-card interactions

In v1, the summary card remains a single tab-level target.

- clicking the card activates the tab
- clicking the card focuses the tab's active pane
- branch lines are informational only
- diff stats and PR chips inside Summary mode are informational only

Summary mode does not introduce child-level click targets for branches or chips.

### Selection, hover, and tab-level behavior

Summary mode keeps the existing tab-level interaction model:

- the active tab retains active/selected styling
- hover styling remains tab-level
- tab header behavior remains unchanged
- rename behavior remains unchanged
- drag-and-drop behavior remains unchanged
- close behavior remains unchanged
- the existing tab-level hover sidecar behavior remains unchanged

### Single-pane tabs

A single-pane tab can still render a summary card.

In that case, the summary card may contain:

- one primary work label
- one working directory
- one branch line

Summary mode should still look intentional for single-pane tabs, even though it provides less aggregation benefit there than for multi-pane tabs.

### Mixed tabs and missing data

Summary mode must handle mixed-content tabs gracefully.

Examples:

- A tab may include panes that contribute work labels but no branch context.
- A tab may include panes that contribute branch context but no working-directory data.
- A tab may include only one visible branch line and no working-directory line.

Warp must not insert placeholder copy such as `No branch` or `No directory`.

### Search behavior

When `View as = Tabs` and `Tab item = Summary`, search/filtering operates on the tab summary data rather than on a focused-pane row.

Search matching includes the full underlying summary content for the tab, including:

- all normalized work labels, including labels not visible because of `+ N more`
- all working-directory values gathered for the tab
- all coalesced branch labels, including branch groups not visible because of the branch overflow line
- PR labels / identifiers present in the summary card
- diff-stat text shown for branch lines

This means a tab can match the search query even if the matching branch or work label is currently hidden behind an overflow line.

## Success criteria

1. The vertical tabs popup shows a `Tab item` section directly under `View as` when `View as = Tabs`.
2. The `Tab item` section is hidden when `View as = Panes`.
3. `Tab item` has exactly two options: `Focused session` and `Summary`.
4. `Focused session` remains the default Tabs-mode representation.
5. Selecting `Summary` updates the panel immediately and leaves the popup open.
6. When `Summary` is selected, `Density`, `Pane title as`, `Additional metadata`, and `Show` are hidden.
7. Returning from `Summary` to `Focused session` restores the previously selected values for those hidden controls.
8. In Summary mode, each tab renders one summary card rather than one focused-pane row.
9. The summary card's first line is derived from work labels across the tab, not just the focused pane.
10. Work-label deduplication is conservative and exact-equivalent only after whitespace normalization.
11. The primary line uses ` • ` between visible work labels.
12. The working-directory line shows the unique working directories in stable first-seen order rather than choosing one heuristic "best" directory.
13. The working-directory line uses ` • ` between visible directories.
14. The Summary pane circle renders one pane-kind icon for homogeneous tabs.
15. The Summary pane circle renders two pane-kind icons for heterogeneous tabs, selected from the two oldest distinct pane kinds by pane creation order, with the secondary icon in the same bottom-right placement as the existing agent/status composite icon.
16. The lower rows in Summary mode are branch lines, not pane previews.
17. Branch lines are coalesced by repository + branch context, so two repositories on `main` do not collapse into one row.
18. Each branch line can show branch label, diff stats, and PR chip in one row.
19. Summary mode shows at most three visible branch lines, followed by `+ N more` when additional branch contexts exist.
20. Branch lines and chips in Summary mode are informational only and do not create new click targets.
21. Tabs with no working-directory data or no branch data still render sensible summary cards without placeholder text.
22. Search in Summary mode matches the full summary dataset for the tab, including values hidden behind overflow lines.
23. Switching away from Summary does not erase the user's focused-session density or pane-row display preferences.
24. Existing tab header, selection, hover, close, rename, drag, and sidecar behavior remain unchanged.

## Validation

- Open the vertical tabs popup, switch to `View as = Tabs`, and verify a `Tab item` section appears directly beneath `View as`.
- Verify `Focused session` is selected by default in Tabs mode.
- Switch between `Focused session` and `Summary` and verify the panel updates immediately without closing the popup.
- While `Summary` is selected, verify `Density`, `Pane title as`, `Additional metadata`, and `Show` are hidden.
- Switch back to `Focused session` and verify those controls return with their previous values intact.
- Create a tab with multiple terminal/agent panes and verify the Summary primary line contains multiple work labels rather than just the active pane's label.
- Verify exact-duplicate work labels are deduped, while distinct labels remain distinct.
- Create a tab spanning multiple working directories and verify the second line shows the unique directories in first-seen order.
- Verify the primary and working-directory lines use ` • ` separators between visible values.
- Create a homogeneous tab and verify Summary renders one pane-kind icon.
- Create a heterogeneous tab and verify Summary renders two pane-kind icons selected from the two oldest distinct pane kinds, with the oldest as the main icon and the second-oldest distinct kind as the bottom-right secondary icon.
- Create a tab with multiple panes on the same repository + branch and verify they produce one branch line rather than multiple pane-preview rows.
- Create a tab with two different repositories both on `main` and verify they render as separate branch lines.
- Verify diff stats and PR chips appear on the branch line rather than being keyed to a focused pane row.
- Create more than three unique branch contexts in one tab and verify only three branch lines are shown, followed by `+ N more`.
- Verify tabs with no branch data omit the branch section entirely.
- Verify tabs with no working-directory data omit the working-directory line entirely.
- Click a Summary card and verify Warp activates the tab and focuses its active pane.
- Verify clicking on branch lines or chips does not trigger separate actions.
- Search for a work label, directory, hidden-overflow branch, PR number, and diff-stat text, and verify the correct tab still matches in Summary mode.
- Verify tab headers, drag-and-drop, rename, close, and hover-sidecar behavior are unchanged while Summary mode is selected.

## Open questions
None.
