# Tech Spec: Tab search matches tab group names

**Issue:** [warpdotdev/warp#14689](https://github.com/warpdotdev/warp/issues/14689)

## Problem

The vertical tabs panel's search filter is built entirely from pane and tab text. `TabGroup::name` is never read, so no query can match a group by name, and there is no mechanism to admit a tab into the results on the strength of its group's name rather than its own.

Separately, `render_grouped_tab_container` hides member rows when `TabGroup::collapsed` is set. Because the filter runs upstream of that, a collapsed group can survive the filter with a matching member and still render as a bare header, showing the user nothing.

## Relevant code

All in `app/src/workspace/view/vertical_tabs.rs` unless noted.

- `render_groups` (1748) — builds the filtered list and renders it.
- `VerticalTabsPanelState::matching_tab_indices` (1166) — **a second, near-duplicate copy of the same filter**, driving tab cycling under an active search via `activate_next_tab` / `activate_prev_tab` (`app/src/workspace/view.rs:11934`, `11958`). Both sites must admit the same tabs, or the panel shows tabs the keybindings refuse to visit. #9666 documents this same duplication causing a separate bug.
- `render_groups` (1784-1886) — the existing per-tab filter, producing `Vec<(usize, Option<Vec<PaneId>>)>`: tab index plus either `None` ("render all pane rows") or `Some(ids)` ("render only these pane rows"). Branches on `VerticalTabsResolvedMode`; `Summary` matches summary fragments, `Panes`/`FocusedSession` match per-pane via `pane_matches_query`.
- `render_groups` (1911) — the `"No tabs match your search."` empty state.
- `render_groups` (1950) — the group-container branch. Already clones the `TabGroup` out of `workspace.tab_groups`.
- `render_groups` (1955-1958) — computes a group's member run with `take_while` over **consecutive** entries sharing a `group_id`. This is why result ordering is load-bearing.
- `render_grouped_tabs_header` (2815) — renders the group title, previously inlining the `"New Group"` fallback.
- `render_grouped_tab_container` (2976, 3001, 3080) — takes `&TabGroup`, reads `group.collapsed` into `is_collapsed`, and gates member rendering on it.
- `crates/../app/src/workspace/tab_group.rs` — `TabGroup { id, name: Option<String>, color, collapsed, draggable_state, pinned }` and `TabGroupId`.
- `app/src/workspace/mod.rs` — `Workspace::tab_groups` (map of `TabGroupId → TabGroup`) and `TabData::group_id: Option<TabGroupId>`.

## Current state

`render_groups` computes `visible_tabs` and hands it to a render loop. Everything downstream — group container runs, drop targets, ghost slots, the empty state — consumes that one vector. Adding group-name matching is therefore a matter of adding entries to it, with no downstream changes.

The render loop assumes a group's members are a **contiguous** subslice of `visible_tabs`. Any insertion that breaks tab-index ordering would cause one group to render as several containers.

## Proposed changes

### 1. Shared display-name helper

Extract the group header's title text, including the untitled fallback, so search and render read the same string:

```rust
/// Header text for a tab group the user has never named.
const UNTITLED_GROUP_NAME: &str = "New Group";

fn group_display_name(group: &TabGroup) -> String {
    group
        .name
        .clone()
        .unwrap_or_else(|| UNTITLED_GROUP_NAME.to_string())
}
```

`render_grouped_tabs_header` (2815) switches to calling it, replacing its inlined `unwrap_or_else(|| "New Group".to_string())`. This is what makes invariant 9 true by construction rather than by coincidence: the text searched and the text displayed cannot drift.

### 2. Shared matched-group helpers

Both filter sites derive their matched groups from the same two functions, so the rendered list and the navigable list cannot drift apart:

```rust
fn matched_group_ids(
    tab_groups: &HashMap<TabGroupId, TabGroup>,
    query_lower: &str,
) -> HashSet<TabGroupId>

fn tab_admitted_by_group_name(
    group_id: Option<TabGroupId>,
    matched_groups: &HashSet<TabGroupId>,
) -> bool
```

`matching_tab_indices` gains a `tab_groups` parameter and short-circuits to `true` for any tab admitted by its group's name. Its two callers in `view.rs` pass `&self.tab_groups`.

### 3. Merge helper

```rust
fn merge_group_name_matches(
    tab_group_ids: &[Option<TabGroupId>],
    matched_groups: &HashSet<TabGroupId>,
    own_matches: Vec<(usize, Option<Vec<PaneId>>)>,
) -> Vec<(usize, Option<Vec<PaneId>>)>
```

Walks `tab_group_ids` in index order, advancing a peekable iterator over `own_matches` in lockstep. For each tab index:

- in a matched group → push `(tab_index, None)`, discarding any narrower own-match
- not in a matched group, has an own-match → push the own-match unchanged
- neither → skip

Early-returns `own_matches` untouched when `matched_groups` is empty, which is the common path and gives invariant 6 for free.

Two properties this encodes:

- **Ordering** (invariant 7). Output is ascending by tab index because it is produced by walking the tab list, not by appending matches. This is what keeps the `take_while` run at 1955 intact.
- **Upgrade, not duplicate** (invariants 3 and 4). A member already present as `Some(pane_ids)` becomes `None`, so the whole tab shows. A group-name match means whole tabs, not pane-filtered slices.

Taking `tab_group_ids: &[Option<TabGroupId>]` rather than the `Workspace` keeps the function free of `AppContext`, so it is unit-testable directly.

### 4. Wire into `render_groups`

In the non-empty-query branch, after the existing filter produces `own_matches`:

```rust
let matched_groups = matched_group_ids(&workspace.tab_groups, &query_lower);
let tab_group_ids: Vec<Option<TabGroupId>> =
    workspace.tabs.iter().map(|tab| tab.group_id).collect();

merge_group_name_matches(&tab_group_ids, &matched_groups, own_matches)
```

The `to_lowercase().contains()` test is the same case-insensitive substring rule the existing filter uses, so no new matching semantics enter the code. A group with no members contributes nothing, because nothing in `tab_group_ids` references it (invariant 8).

### 5. Collapse override during search

At the group branch (1950), the `TabGroup` is already an owned clone, so the binding becomes `mut` and:

```rust
if !query.is_empty() {
    group.collapsed = false;
}
```

This expands *every* group surviving the filter, not only name-matched ones — a collapsed group holding a title-matching tab would otherwise render as a header with its match hidden (invariant 10). Because it mutates a render-time clone, the stored flag is untouched and the next frame after the query clears restores the user's state, with no save/restore bookkeeping to get out of sync (invariant 11).

## Data flow

```
query
  ├─→ existing per-tab/pane filter ──────────────→ own_matches
  │                                                    │
  └─→ group_display_name × workspace.tab_groups ──→ matched_groups
                                                       │
              merge_group_name_matches(tab_group_ids, ─┘) → visible_tabs
                                                              │
                                    render loop (unchanged) ──┘
                                      └─ group.collapsed = false while query active
```

## Tradeoffs

- **Expanding all surviving groups, not just matched ones.** Slightly broader than the stated problem, but the alternative leaves title matches invisible inside collapsed groups, which reads as a bug. The cost is that a previously-collapsed group renders with a chevron and members instead of its collapsed icon collage during search.
- **Matching the displayed name including the fallback.** Means a query of "new group" matches every unnamed group. Accepted: the user is searching for what is on screen, and the alternative (skipping unnamed groups) makes them unfindable.
- **Recomputing `matched_groups` and `tab_group_ids` per frame.** Both are O(groups) and O(tabs) over small collections, in a path that already does far more work per tab. Not worth caching.
- **Passing slices instead of `&Workspace`.** Costs one small allocation per frame under a non-empty query; buys a helper testable without a full app context.

## Testing and validation

Unit tests in `app/src/workspace/view/vertical_tabs_tests.rs`, matching the pure-function style already there. Mapping to the product spec's invariants:

| Invariant | Test |
|---|---|
| 1, 9 | `group_display_name` returns the name when set; returns `"New Group"` when unset |
| 2 | a group-name match pulls in members whose own titles do not match |
| 3, 4 | a member present as `Some(pane_ids)` is upgraded to `None` |
| 5 | tabs outside the matched group stay filtered |
| 6 | empty `matched_groups` returns the input untouched |
| 7 | results stay ordered by tab index rather than appended |
| 8 | a matched group with no members is a no-op |
| 1, 9 (nav) | `matched_group_ids` selects by displayed name, case-insensitively, including the untitled placeholder |
| 2 (nav) | `tab_admitted_by_group_name` admits members of matched groups and never ungrouped tabs |

Invariants 10-14 are render-path behavior that this test file does not reach. Validate manually: create a group named `backend` with tabs whose titles lack that word, collapse it, search `backend`, confirm the group renders expanded with all members, then clear the query and confirm it re-collapses. Repeat in each of the three display modes for invariant 14.

Gates: `./script/presubmit`, `./script/format --check`, and `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`.

## Risks

- **Group contiguity in `workspace.tabs`** is a pre-existing assumption of the render loop at 1955, not one this change introduces. If members of a group are ever non-contiguous, the group already renders as multiple containers today; group-name matching inherits that without worsening it.
- **The `"New Group"` literal appears elsewhere** — `app/src/workspace/view.rs:7316` and `:20036` seed the rename editor with the same fallback. Those are outside this module and unchanged here; a future consolidation should route them through `group_display_name` too.

## Follow-ups

- Ctrl+Tab command palette switcher (`app/src/search/command_palette/tabs/`) has no group concept; making it group-aware is separate work.
- #9666 covers an adjacent gap in the same `render_groups` function: the `title_override` gate drops a tab's custom title from search fragments in Panes mode. Independent of this change.
- End-to-end coverage under `crates/integration/` for the search-matches-group flow.
