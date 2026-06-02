# Git Graph Panel (commit DAG visualization + right-click git operations)

## Summary
A "Git Graph" tab in the left tools panel that renders the active repository's
commit history as a colored directed acyclic graph (DAG) — branch lanes, commit
nodes, merge/fork connectors, and HEAD / local-branch / remote-branch / tag
badges. Selecting a commit shows its details and changed files; clicking a
changed file opens a read-only diff in the main area. Inspired by
[vscode-git-graph](https://github.com/mhutchie/vscode-git-graph).

The browsing surface is read-only. A separate, flag-gated layer
(`FeatureFlag::GitGraphWrite`) adds **right-click context-menu operations**
(checkout, branch/tag create/delete, merge, rebase, reset, cherry-pick, revert,
push/pull, archive). The read-only base and the write layer are gated
independently so the base can ship without the mutating layer.

## Problem
Warp is a terminal, so users can run `git log --graph`, but:
- ASCII graphs become unreadable with many branches and are not interactive — you
  can't click a commit to inspect what it changed.
- The tools panel (Project Explorer / Global Search / Warp Drive / Agent
  Conversations) has no visual git entry point, so "looking at git history" — a
  high-frequency task — forces a switch to the terminal or an external tool.

## Behavior invariants

### Entry point & visibility
1. When `FeatureFlag::GitGraph` is enabled, the build includes the `local_fs`
   feature, and the `Settings → Git` toggle `show_git_graph` is on, a "Git Graph"
   button (`Icon::GitBranch`, tooltip "Git Graph") appears in the left toolbelt.
2. Clicking the button opens the Git Graph view; clicking another tab switches
   away, consistent with the other left-panel tabs.
3. The panel follows the repository of the currently active pane's working
   directory; changing the active directory re-resolves the repository.

### Graph rendering
4. Each commit is one fixed-height row: a lane graph (colored vertical lines +
   node dot + orthogonal connectors) followed by short hash, ref badges, subject.
5. Merge commits, forks, and multi-root repositories all connect correctly; lane
   width adapts to the current maximum number of parallel branches.
6. Ref badges distinguish four kinds — HEAD, local branch, remote branch, tag —
   each as a color-coded badge.

### Commit detail & file diff
7. With no commit selected, the detail area is empty.
8. Selecting a commit shows: full message, author (name + email), committer (when
   it differs from the author), full hash, and the changed-file list (each row:
   path + `+insertions / -deletions`).
9. Clicking a changed file opens a read-only diff of that file's changes in the
   selected commit, in a pane in the main area; clicking another file reuses the
   same diff pane rather than opening a new one.

### Multi-repository & branch filtering
10. When the working directory hosts multiple repositories (within
    `git_graph_scan_depth`), a repository picker at the top lets the user switch
    which repository's history is shown.
11. A branch-filter overlay lets the user choose which branches' commits the graph
    shows (select-all / deselect-all / per-branch toggles); the graph updates to
    match the selection.

### Pagination & refresh
12. The graph loads an initial page (~200 commits); a "Load more" row at the
    bottom fetches the next page on demand.
13. A manual refresh button in the header re-fetches the current repository.

### Settings
14. `Settings → Git` exposes `show_git_graph` (panel visibility toggle) and
    `git_graph_scan_depth` (how many directory levels below the working directory
    to probe for repositories: 0 = the working directory's own repo only).

### States & edge cases
15. Active pane is not inside any git repository: show "Current directory is not a
    git repository."
16. A `git` command fails: show an error message; the header refresh button lets
    the user retry. Never crash and never affect other panels.
17. Repository has no commits: show "No commits yet."
18. With `FeatureFlag::GitGraphWrite` off, the panel is strictly read-only: no
    operation mutates repository state under any interaction. With it on, only
    the explicit operations in "Write operations" below mutate state — each
    behind a confirmation or an input/save dialog; browsing never does.

### Write operations (right-click context menus, gated by `FeatureFlag::GitGraphWrite`)
19. Right-clicking a commit row, the short hash, or a tag / local-branch /
    remote-branch badge, opens a context menu matching the target kind:
    - **Commit**: Add Tag…, Create Branch…, Checkout…, Cherry Pick…, Revert…,
      Drop…, Merge into current branch…, Rebase current branch on this Commit…,
      Reset current branch to this Commit…, Copy Commit Hash, Copy Commit Subject.
    - **Short hash** (the 7-char hash left of the subject): a focused menu with a
      single "Copy Short Hash to Clipboard" — the 7-char hash shown, distinct
      from the commit menu's "Copy Commit Hash" which copies the full hash.
    - **Tag**: View Details, Delete Tag…, Push Tag…, Create Archive, Copy Tag Name.
    - **Remote branch**: Checkout Branch…, Delete Remote Branch…, Merge into
      current branch…, Pull into current branch…, Create Archive, Unselect in
      Branches Dropdown, Copy Branch Name.
    - **Local branch**: Checkout Branch…, Rename Branch…, Delete Branch…, Merge
      into current branch…, Rebase current branch on Branch…, Push Branch…,
      Create Archive, Unselect in Branches Dropdown, Copy Branch Name. The HEAD
      badge opens this menu for the **current** branch and omits the operations
      that don't apply to itself (checkout / delete / merge-into-current /
      rebase-onto), leaving Rename, Push, Create Archive, Unselect, Copy.
20. The read-only items (Copy *, View Details, Unselect in Branches Dropdown) are
    always present; the mutating items appear only when `GitGraphWrite` is on.
    "Unselect in Branches Dropdown" reuses the branch filter (deselects the ref);
    "View Details" selects the tagged commit.
21. Every mutating operation is gated by explicit input before it runs:
    - name prompts (Add Tag / Create Branch / Rename Branch) — a text dialog;
    - Reset — a soft / mixed / hard mode dialog (hard is labelled as discarding
      uncommitted changes);
    - Create Archive — the OS save dialog (the chosen extension picks zip vs
      tar.gz);
    - everything else — a yes/no confirmation stating what will happen
      (history-rewriting and remote-mutating actions say so explicitly).
22. Operations phrased "…current branch" (merge / rebase / reset / pull /
    cherry-pick / revert / drop) are always offered when writing is enabled; on a
    detached HEAD git applies them to the detached HEAD or fails (surfaced in the
    error banner) rather than the menu silently hiding them.
23. A running operation blocks a second one and dims the panel with a centered
    "Working…" overlay; on success the graph reloads (a remote-branch deletion
    additionally fetches `--prune` so the pruned ref disappears); on failure a
    dismissable banner shows the git error and the repository is left as git left
    it (e.g. a conflicted cherry-pick/merge stops with its own message).

## Non-goals (deferred)
- **Stash operations**, and resolving in-app the **conflicts** a merge / rebase /
  cherry-pick may leave (the user finishes those in the terminal).
- **Per-branch push remote / upstream resolution**: Push Branch / Push Tag use
  `origin`; pushing to another remote is a terminal task.
- **Auto-refresh** on repository changes (new commit, branch switch): manual
  refresh covers this; auto-refresh needs repo-watcher plumbing + debounce.
- **Rounded (bezier) connectors**: the render layer has only rectangle
  primitives, so connectors are orthogonal square-corner polylines.
- **In-graph commit search.**
- **Per-file A/M/D/R status** and **formatted commit timestamps** in the detail
  area (only adds/dels counts and raw metadata are shown today).
- **Theme-token colors**: lane and badge colors come from a fixed palette today.
