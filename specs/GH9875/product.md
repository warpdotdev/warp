# Product Spec: Automatic tab grouping by project

**Issue:** [warpdotdev/warp#9875](https://github.com/warpdotdev/warp/issues/9875)
**Figma:** none provided

## Summary

An opt-in mode that keeps every tab in a tab group named after the project it is in — all
worktrees of one repository in one group — while manual grouping keeps working exactly as it
does today. The feature is a membership resolver plus one persisted key; every visible
affordance is inherited from the tab groups that already shipped in v0.2026.07.01.

## Problem

Warp shipped manual tab groups in v0.2026.07.01: create, rename, color, collapse, pin, drag
within and across windows, and restore across restarts. Filling those groups is entirely
manual.

Developers who keep several repositories open at once — increasingly with multiple
`git worktree` checkouts per repository, one per parallel coding-agent task — get one flat
list where sessions from unrelated projects interleave, and where worktrees of the same
repository look unrelated because they sit at different paths. Multiple commenters on #9875
report moving to other tools over it.

Two implementations were attempted. [#9876](https://github.com/warpdotdev/warp/pull/9876)
drew its own project section headers in the vertical tabs panel and was closed pending
designer mocks; the tab-group model it would now reuse (`app/src/workspace/tab_group.rs`)
landed three weeks after it was written. [#14306](https://github.com/warpdotdev/warp/pull/14306)
proposes a much larger projects-and-tasks layout and is stalled on the same `needs-mocks`
gate.

## Goals

- Tabs from one repository — including every worktree of it — collect into one group.
- Manual grouping continues to work unchanged, in the same tab list, at the same time.
- Every affordance (name, color, collapse, pin, drag, restore) is the existing group's.
- A user's manual placement always wins and is never undone by automation.

## Non-goals

- Restricting drags to a tab's own group. It removes an ability users have today.
- Dragging a whole group to another window — separate feature, tracked in
  [#14152](https://github.com/warpdotdev/warp/issues/14152).
- A projects-and-tasks layout ([#14306](https://github.com/warpdotdev/warp/pull/14306)) or
  manually created workspace pills ([#13311](https://github.com/warpdotdev/warp/issues/13311)).
- Cross-window grouping. Grouping is per window, as manual groups already are; the same
  project open in two windows produces one group in each.
- Submodules, remote and SSH working directories, and per-project settings.
- A right-click menu on empty horizontal tab-bar space (the per-tab menu already serves
  both bars).

## Behavior invariants

Each invariant is numbered and testable. Invariant *N* corresponds to requirement R*N* in
the implementation plan.

### Mode

1. Auto-grouping is a mode the user turns on and off, off by default, available only where
   tab groups themselves are available.
2. At the moment the mode is enabled, every ungrouped, unpinned tab that has a project key
   is placed into the group matching it; a tab whose key is not resolved yet is placed the
   first time its key resolves.
3. Disabling the mode leaves every existing group in place as an ordinary manual group;
   nothing dissolves, renames, or reorders.

### Project identity

4. A tab's project key is the shared git directory of the repository containing its
   directory, so every worktree of one repository resolves to the same key.
5. Worktree unification holds for bare repositories, where no ancestor directory is named
   `.git`.
6. A tab whose directory is not inside a git repository keys on the directory, and joins an
   existing non-git group whose path is a prefix of that directory. Automation never creates
   a non-git group for the user's home directory or the filesystem root; a tab sitting there
   stays ungrouped.
7. A tab with no directory at all — settings, notebook, or an editor with no terminal
   session — stays outside every group.
8. A group's name is the repository name for a git group, or the directory basename for a
   non-git group; when two groups in one window would take the same name, both are qualified
   with their parent directory segment.

### Membership lifecycle

9. A tab's project key is fixed when the tab is created and tracks only that tab's own
   directory; changing which pane has focus never changes it.
10. A tab under automation moves into the group matching its project key whenever that key
    changes.
11. A group disappears when its last member leaves it.
12. When the only member of a group changes project and no group exists for the new key, the
    group is re-keyed and renamed in place instead of being destroyed and recreated.

### Manual override

13. Placing a tab where automation would not have put it detaches that tab: it stays where
    the user put it and stops following its directory.
14. Placing a detached tab into the group matching its project key re-attaches it to
    automation; creating a new group from a detached tab does the same, and the new group
    adopts that tab's project key.
15. Ungrouping a group leaves its former members ungrouped and detached; while the mode is
    on they stay ungrouped until the user places them somewhere.
16. Automation never overwrites a manual edit to a group; a group the user renamed or
    recolored keeps those values, including through a re-key under invariant 12.

### Surfaces and persistence

17. The mode applies wherever tab groups render — both the vertical tabs panel and the
    horizontal tab bar.
18. A group created by automation survives a restart carrying its project key, name, color,
    collapse state and pin state.
19. The mode is toggled from the tabs settings and from the tab bar's context menu.

### Coexistence and edge cases

20. When more than one non-git group's path is a prefix of a tab's directory, the longest
    prefix wins; a directory that sits above every existing non-git group's path yields no
    key and falls under invariant 21 rather than producing a parent group.
21. A tab whose project identity cannot be resolved stays where it is rather than being
    re-keyed, and is not read as manually placed.
22. A tab that is the only member of an automation-keyed group keeps its own drag
    affordance, so it can still be dragged to another window; a manual group holding one tab
    is unaffected.
23. The mode never groups a pinned tab and never clears a pin. Unpinning a tab reconciles it
    as if newly created; pinning a tab removes it from its automation group without marking
    it detached.
24. Joining a group never changes that group's collapse state.
25. A tab arriving from another window is treated as newly created rather than as manually
    ungrouped.
26. Reopening a closed tab is treated as newly created only when its stored group no longer
    exists; otherwise its restored placement stands and the ordinary manual-override rule
    applies.
27. Automation never opens the group rename editor.

## Key flows

### Tab follows its project

**Trigger:** the directory of a tab under automation changes to a different project.
The tab's project key is recomputed; the group for the new key is found or created; the tab
moves into it; the group it left is removed if it is now empty, unless the sole-member re-key
in invariant 12 applied instead.
Covers invariants 9, 10, 11, 12, 21.

### Manual placement detaches a tab

**Trigger:** the user drags a tab, or moves it through the tab menu, into a group other than
the one matching its project key — including out of every group. The tab lands where the user
put it and is no longer under automation; later directory changes do not move it.
Covers invariants 13, 15.

### Turning the mode on

**Trigger:** the user enables the mode. Every ungrouped, unpinned tab is placed into the
group for its project key, creating groups as needed; tabs already in manual groups are left
alone. From this point ungrouped means detached.
Covers invariants 2, 15, 16, 23.

## Acceptance examples

1. **Worktrees unify, including bare** (invariants 4, 5) — Given three worktrees of one
   repository at unrelated paths, one of them from a bare checkout, all three tabs sit in one
   group named after the repository.
2. **Descending a non-git directory does not rebuild the list** (invariant 6) — Given a tab
   grouped under a non-git directory, when it moves into a subdirectory, it stays in the same
   group and no group is created or removed.
3. **A sole member changing project re-keys its group** (invariants 11, 12) — Given a group
   holding exactly one tab and no group for the tab's new project, when that tab's project
   changes the group is re-keyed and renamed in place. But when a group for the new project
   already exists, the tab moves into it and the emptied group disappears.
4. **A renamed group keeps its name through a re-key** (invariants 12, 16) — Given a group
   the user renamed, holding exactly one tab, when that tab's project changes the group's key
   moves and the user's name stays.
5. **Ungrouping is not undone** (invariant 15) — Given a group holding several tabs with the
   mode on, when the user ungroups it and then changes the directory of one of those tabs,
   all of them remain ungrouped.
6. **Re-attaching restores automation** (invariants 13, 14) — Given a tab the user dragged
   into another project's group, when the user places it back into the group matching its own
   project it follows its directory again.
7. **A sole member can still leave the window** (invariant 22) — Given the mode on and a tab
   that is the only member of its project's group, dragging that tab out of the window
   detaches it into a new window and its emptied group disappears.
8. **Enabling the mode leaves pinned tabs alone** (invariants 2, 23) — Given two pinned tabs
   and three unpinned tabs across two projects, when the mode is enabled the pinned tabs stay
   pinned and ungrouped and only the unpinned tabs are grouped.
9. **A collapsed group stays collapsed** (invariant 24) — Given a collapsed group for a
   project and a tab under automation whose directory changes into that project, the tab
   joins the group and the group stays collapsed.
10. **Detachment survives a restart** (invariants 13, 18) — Given a tab the user dragged into
    another project's group and a restart, the tab is still in that group and still does not
    follow its directory.

## Open questions

- **Feedback when the toggle reorganizes a window the user did not act in.** The setting is
  global and cloud-synced, so enabling the mode rearranges tabs in every other open window —
  and on every other signed-in machine — with nothing on screen explaining why. A silent
  large-scale reorganization reads as a bug rather than a feature turning on. Options range
  from a one-time notice in windows that did not originate the flip, to an animated
  transition, to accepting the silence. This needs a product call.
- **Whether the anchor pane's identity is persisted.** A tab's project follows the terminal
  pane it was created with, and restore re-derives that anchor as the first terminal pane in
  the restored pane group. Splitting left or up inserts the new pane *before* the existing
  one, so such a tab restores with a different anchor and can silently change project.
  Persisting the anchor's identity fixes it at the cost of a third per-tab field; accepting
  the re-anchor as an ordinary key change is cheaper and arguably correct.

## Known limitation

Repository detection never removes a `Local` root, so a repository deleted and recreated at
the same path keeps resolving to the old identity until restart. Invariant 21 does **not**
bound this — a stale root resolves successfully, just to outdated data. The risk is accepted
unmitigated here and should be fixed separately in `crates/repo_metadata`.
