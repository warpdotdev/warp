# APP-4271: Vertical Tabs Summary v2 — Per-Line Titles, Working Directories, and Conversation Status Icons

## Summary

Refine the Tabs / Summary tab item mode introduced in APP-3875. Render each work label and each working directory on its own line instead of coalescing them with ` • `, prefix conversation title lines with a status icon, sort title lines so conversations come before non-conversation lines, and lock the card's region order to titles → working directories → branches.

## Problem

The v1 Summary card from APP-3875 keeps the primary line and working-directory line each as a single `•`-joined line. Two failure modes show up in practice:

- When a tab has more than two or three work labels or working directories, the joined line truncates and hides everything past the first one or two values, defeating the purpose of the summary.
- Conversation status — the most actionable piece of information about an agent pane — is not visible on the Summary card; it currently lives only on the focused-session row in `Tab item = Focused session`.

The card needs to surface each work label and each working directory directly, and convey conversation status alongside the title that owns it.

## Figma

Figma: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7633-129739&t=0DkBL0SwricwRNSz-11

The mock is the reference for status-icon styling on conversation title lines. The mock's metadata layout is **not** authoritative — the metadata content, ordering, and overflow behavior described in this spec take precedence over what the mock shows.

## Behavior

The Summary card's region order, top to bottom, is:

1. Title region (one line per work label)
2. Working-directory region (one line per working directory)
3. Branch region (one line per coalesced branch context)

Regions with no data are omitted entirely. The card never renders placeholder text for an empty region.

### Region order

1. The Summary card always renders regions in the order titles → working directories → branches. No setting changes this order.
2. If the title region has any visible lines, it appears first. If the working-directory region has any visible lines, it appears below the title region. If the branch region has any visible lines, it appears below the working-directory region.
3. Omitting an earlier region does not change the relative order of later regions; e.g. a tab with no work labels but with working directories and branches renders working directories first, then branches.

### Title region

4. Each unique work label gathered for the tab renders on its own line. Labels are not joined with ` • ` or any other separator.
5. Title lines appear in first-seen order across the tab's visible panes, matching v1's existing label-gathering order, with one exception: lines whose contributing pane has a known `ConversationStatus` are sorted ahead of lines without one. The relative first-seen order is preserved within each group, so the title region effectively renders all conversation lines first (in first-seen order) followed by all non-conversation lines (in first-seen order).
6. Title-line normalization and dedupe rules from APP-3875 carry over unchanged: trim leading and trailing whitespace, collapse repeated internal whitespace, drop empty labels, and dedupe exact-equivalent normalized labels while preserving the first-seen display text. The card must not semantically rewrite, fuzzy-match, or merge distinct labels.
7. The title region renders at most three title lines. If more than three unique labels exist for the tab, the title region ends with a `+ N more` overflow line, where `N` is the number of additional unique labels not visible. Because conversation lines are sorted first (invariant 5), they take precedence in the visible 3-line cap; non-conversation lines spill into the `+ N more` overflow before any conversation line does.
8. Each title line truncates with an end-ellipsis when its content does not fit the card's available width on a single line.
9. A tab with no work labels for any visible pane omits the title region entirely. No `+ 0 more` line is ever rendered.

### Conversation status icon prefix

10. A title line whose contributing pane has a known conversation status renders a small status pill at the start of that line, before the title text. The pill contains only the status icon — no agent (Oz or CLI agent) icon — and is styled like the conversation status pill used in the pane header / detail sidecar (icon over a 10%-opacity colored background with rounded corners).
11. A title line is eligible for the status pill when its contributing terminal pane has a `ConversationStatus` available — CLI agent sessions that support rich status (their session status), or Oz agent / ambient agent conversations (their `selected_conversation_status_for_display`). Plain terminals, CLI agents without rich status, and conversations without a known status do not get a prefix.
12. The status the pill reflects is the conversation's current `ConversationStatus` (in progress, success, error, cancelled, blocked).
13. A title line whose underlying source is not a conversation pane — plain terminal commands, code panes, notebooks, workflows, settings, file viewers, etc. — does not render a status icon prefix; the line begins directly with the title text.
14. If two distinct panes contribute the same normalized work label and the dedupe rule keeps only the first-seen one, the status pill shown on that one visible line reflects the first-seen pane's status.
15. The `+ N more` overflow line in the title region never renders a status icon prefix, even if some of the hidden labels would otherwise qualify.

### Working-directory region

16. Each unique working directory gathered for the tab renders on its own line. Directories are not joined with ` • ` or any other separator.
17. Directory lines appear in first-seen order across the tab's visible panes, matching v1's gathering order.
18. Working-directory normalization and dedupe rules from APP-3875 carry over unchanged: trim, collapse internal whitespace, drop empty values, and dedupe exact-equivalent normalized values while preserving the first-seen display text. No "most representative" directory is heuristically chosen.
19. The working-directory region renders at most three directory lines, followed by a `+ N more` overflow line when more unique directories exist.
20. Each directory line uses start-clip truncation when it does not fit, so the trailing path segment stays visible (consistent with how working directories truncate in the focused-session row today).
21. Working-directory lines never render a status icon prefix.
22. A tab with no working-directory data on any visible pane omits the working-directory region entirely.

### Branch region

23. Branch-region behavior is unchanged from APP-3875. Branches are coalesced by repository + branch context, ordered by first-appearance in the tab's visible pane order, and capped at three visible branch lines with a `+ N more` overflow line when more unique branch contexts exist.
24. Diff stats and PR chips continue to render on the right side of each branch line, keyed to the branch context rather than to any specific pane.
25. Two different repositories on the same branch name (e.g. both on `main`) continue to render as separate branch lines.

### Card-level icon and interactions

26. The card's left-side pane-kind icon (a single icon for homogeneous tabs, or a stacked pair of icons for heterogeneous tabs, chosen by pane creation order) is unchanged from APP-3875. The new per-line conversation status icons are scoped to the title region; they do not replace or modify the card's left-side icon.
27. Clicking the Summary card activates the tab and focuses its active pane, unchanged from APP-3875.
28. Per-line status icons, individual title lines, individual directory lines, branch lines, diff stats, and PR chips are all informational. None of them introduce new click targets in v2.
29. Tab-level selection, hover, drag-and-drop, rename, close, and hover-sidecar behavior remain unchanged.

### Mixed and missing data

30. A tab whose visible title lines have no conversation status renders title lines without any status pill prefix; the absence of statuses must not push the title text rightward as if a prefix slot were reserved.
31. A tab whose visible title lines all carry a conversation status renders each line prefixed by its own status pill.
32. A tab whose visible title lines mix lines with and without status renders status pill prefixes only on the lines with status; the lines without status share the same horizontal text start by reserving an empty prefix slot, so titles align vertically when at least one visible line in the region has a prefix.
33. A tab with exactly one work label, one working directory, and one branch context still renders three single-line regions in the documented order; nothing collapses to a one-line layout.
34. The card never inserts placeholder copy such as `No branch`, `No directory`, or `No title` for an empty region.

### Search behavior

35. Search / filtering in Summary mode continues to match against the full underlying summary dataset for the tab — every gathered work label, every gathered working directory, every coalesced branch label, every PR label, and every diff-stat text — including values currently hidden behind a `+ N more` overflow line in any region.

### Settings popup

36. The vertical tabs settings popup, including `View as` / `Tab item` structure and the controls hidden by `Tab item = Summary`, is unchanged from APP-3875. This v2 only changes how the Summary card itself renders.
