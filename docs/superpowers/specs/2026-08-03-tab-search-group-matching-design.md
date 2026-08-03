# Tab search matches tab groups

Date: 2026-08-03
Status: Approved, ready for implementation planning

## Problem

The vertical tabs panel has a search field that filters the tab list in place
(`app/src/workspace/view/vertical_tabs.rs`, `render_groups`). It matches against
pane and tab text only. Tab group names — `TabGroup::name`, the feature gated
behind `FeatureFlag::GroupedTabs` — are never consulted, so a user who organizes
work into a group named `backend` cannot find that group by typing its name, and
cannot use the search field to pull up everything inside it.

## Goal

Typing a query that matches a tab group's displayed name surfaces that group and
every tab under it, regardless of whether the individual tab titles match.

## Scope

In scope: the vertical tabs panel search field only.

Out of scope: the Ctrl+Tab command palette tab switcher
(`app/src/search/command_palette/tabs/`). It is a flat MRU list of
`TabNavigationData` with no group concept; adding group awareness there would
require new item types and grouped result rendering, and is a separate project.

No data-model changes. `TabGroup` already carries `name`, `collapsed`, and
`pinned`; `TabData::group_id` already links members to their group.

## Design

### 1. Where the change lands

Entirely within `render_groups` (`vertical_tabs.rs:1748`), plus two extracted
helpers.

`render_groups` builds:

```rust
let visible_tabs: Vec<(usize, Option<Vec<PaneId>>)>
```

where the first element is the index into `workspace.tabs` and the second is
`None` (render all of the tab's pane rows) or `Some(ids)` (render only the pane
rows that matched). Everything downstream of line 1906 — group container runs,
drop targets, ghost slots, the empty state — consumes this vector and needs no
change. Group matching is therefore purely a matter of adding entries to it.

### 2. Matching rule

A new helper, shared with the group header renderer so the two cannot drift:

```rust
/// The group title as displayed in the panel header, including the
/// untitled fallback.
fn group_display_name(group: &TabGroup) -> String {
    group.name.clone().unwrap_or_else(|| "New Group".to_string())
}
```

`vertical_tabs.rs:2791`, which currently inlines the same
`unwrap_or_else(|| "New Group".to_string())`, switches to calling it.

A group matches when:

```rust
group_display_name(group).to_lowercase().contains(query_lower)
```

This is the same case-insensitive substring rule the existing tab filter uses,
so no new matching concept enters the code.

Matching the *displayed* name — fallback included — means an untitled group
matches the query "new group". This is deliberate: the user is searching for
what is on screen.

The rule applies in all three resolved modes (`Summary`, `FocusedSession`,
`Panes`), since group headers render in each.

### 3. Merging group hits with tab hits

A second helper, pure and unit-testable without an `AppContext`:

```rust
/// Force-includes every member of a name-matched group, preserving original
/// tab order. A tab that also matched on its own title is upgraded to the
/// group entry (`None` = all pane rows shown), never duplicated.
fn merge_group_name_matches(
    tab_group_ids: &[Option<TabGroupId>],
    matched_groups: &HashSet<TabGroupId>,
    own_matches: Vec<(usize, Option<Vec<PaneId>>)>,
) -> Vec<(usize, Option<Vec<PaneId>>)>
```

Two properties are load-bearing:

- **Ordering.** The output must stay sorted ascending by tab index. The `while`
  loop at `vertical_tabs.rs:1921` computes a group's member run with
  `take_while` over consecutive `visible_tabs` entries sharing a `group_id`.
  Injecting members out of order would split one group across several rendered
  containers.
- **Upgrade, not duplicate.** In `Panes`/`FocusedSession` mode a member tab may
  already appear as `Some(vec![pane_a])` from its own title match. Under a
  group-name match that entry becomes `None` so every pane row shows. "All the
  tabs under the group" means whole tabs, not pane-filtered slices of them.

Tabs outside any matched group keep their existing filter behavior unchanged.

### 4. Collapsed groups

`TabGroup::collapsed` is persisted state, and `render_grouped_tab_container`
hides members when it is set (`vertical_tabs.rs:3059`). The group value at the
call site (`vertical_tabs.rs:1927-1932`) is already a clone, so the override is:

```rust
// While a search is active, expand every rendered group so matches are
// visible without a click. The stored `collapsed` flag is untouched;
// clearing the query restores the real state on the next frame.
if !query.is_empty() {
    group.collapsed = false;
}
```

This expands *any* group surviving the filter, not only name-matched ones. A
collapsed group holding a title-matching tab would otherwise render as a bare
header with the match hidden inside it — worse than the alternative. Because the
override is applied at render time to a clone, there is no persistence, no undo
state, and no restore logic that can get out of sync.

Accepted side effect: `is_header_selected` (`vertical_tabs.rs:3005`) and the
collapsed-member icon collage (`vertical_tabs.rs:3012`) both key off
`is_collapsed`, so during a search a previously-collapsed group renders with a
chevron and expanded members rather than the collage. This is the intended
appearance.

## Testing

Unit tests in `app/src/workspace/view/vertical_tabs_tests.rs`, matching the
existing pure-function style in that file:

- `group_display_name` returns the name when set, and `"New Group"` when `None`.
- `merge_group_name_matches`:
  - a group-name match pulls in members whose own titles do not match
  - output stays ordered by tab index
  - a member already present as `Some(pane_ids)` is upgraded to `None`
  - a matched group with no members is a no-op
  - ungrouped tabs are unaffected
- A query matching neither any tab nor any group name still reaches the
  `"No tabs match your search."` empty state.

The collapsed override and the end-to-end filter are render-path behavior not
covered by this test file today. Verify those by running the app with the
`GroupedTabs` flag enabled rather than by adding a new integration test.

## Risks

- Group contiguity in `workspace.tabs` is an existing assumption of the render
  loop, not an invariant this design introduces. If tabs in one group are ever
  non-contiguous, the group already renders as multiple containers; group-name
  matching inherits that behavior without making it worse.
- The "New Group" fallback string is duplicated today. Extracting
  `group_display_name` removes the duplication; anyone adding a third call site
  should use the helper.
