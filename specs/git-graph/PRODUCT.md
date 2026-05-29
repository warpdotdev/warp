# Git Graph Panel (read-only commit DAG visualization)

## Summary
Add a "Git Graph" tab to the left tools panel that mirrors the signature capability of [vscode-git-graph](https://github.com/mhutchie/vscode-git-graph): render the current repository's commit history as a colored directed acyclic graph (DAG) — branch lanes, commit nodes, merge/fork connectors, and branch / remote / tag / HEAD labels. Clicking a commit shows its details (full metadata + changed-file list + insertion/deletion counts).

**v1 scope: read-only browsing.** No operation that mutates repository state.

## Problem
Warp is a terminal, so users can of course run `git log --graph`, but:
- ASCII graphs become unreadable with many branches, and they are not interactive (can't click, can't inspect a commit's changes).
- The tools panel has Project Explorer / Global Search / Warp Drive / Agent Conversations, but no visual git entry point. "Looking at git history" — a high-frequency task — forces a switch to the terminal.

## Goals (v1)
- New "Git Graph" tab in the left panel, using the existing `Icon::GitBranch`.
- Render the commit DAG of the **repository of the currently active pane**:
  - Each row = one commit: lane graph on the left (colored vertical lines + node dot + connectors), then short hash + ref badges + subject.
  - Merge commits, forks, and multi-root repositories all connect correctly.
  - Ref labels distinguish 4 kinds: HEAD, local branch, remote branch, tag, each as a color-coded badge.
- Click a commit → detail area: full message, author (name + email), committer, full hash, and the changed-file list (path + insertions/deletions).
- Pagination: load ~200 commits initially, with a "Load more" row at the list bottom to fetch the next page.
- A manual refresh button in the header. (Auto-refresh on repository changes is deferred — see Non-goals.)
- The entire panel is gated by `FeatureFlag::GitGraph`, defaulting to Dev-only visibility.

## Non-goals (v1, deferred to a separate later plan)
- **Any write operation**: checkout / create branch / merge / rebase / cherry-pick / revert / reset / stash / tag / push / pull / context-menu actions — none.
- Multi-repository switcher UI (v1 follows the active repository only).
- A full diff viewer for commit contents (v1 stops at "changed-file list + insertion/deletion counts").
- Bezier-curve connectors (limited by the render layer, see TECH.md; replaced by orthogonal square-corner connectors — rounding is deferred polish).
- Auto-refresh on repository changes (new commit, branch switch): deferred — it needs repo-watcher plumbing + filtering + debounce; the manual refresh button covers v1.
- In-graph search / branch-filter dropdown.

## User experience

### Entry point and visibility
- When `FeatureFlag::GitGraph` is enabled and the build includes the `local_fs` feature, a "Git Graph" button appears in the left toolbelt; hover shows the tooltip "Git Graph" (with keybinding if bound).
- Clicking the button switches to the Git Graph view; clicking another tab switches away, consistent with existing tabs.

### Graph area
- Each commit occupies one fixed-height row. Lane width on the left adapts to the current maximum number of parallel branches.
- The node dot sits in the commit's lane column; a vertical line means a still-continuing branch; forks/merges are drawn as elbow connectors from the node to an adjacent column. Different lanes cycle through different colors.
- Ref labels sit just left of the subject as small color-coded badges, one per ref, distinguishing HEAD / local branch / remote branch / tag. (Colors come from a fixed palette today; sourcing from theme tokens is deferred polish.)

### Detail area
- No commit selected: detail area is empty.
- Commit selected: full message, author (name + email), committer (when it differs from the author), full hash, and the changed-file list (each row: path + `+ins / -del`). Formatted timestamps and per-file A/M/D/R status are deferred.
- In v1, clicking a file does **not** open it (avoids extra coupling); it is display-only. Opening files is a later enhancement.

### Empty / error states
- Active pane is not inside any git repository: show "Current directory is not a git repository."
- A `git` command fails: show an error message; the header refresh button lets the user retry. Never crash and never affect other panels.
- Repository has no commits: show "No commits yet."

## Roadmap by phase
- **v1 (this plan)**: the read-only browsing above.
- **v2 (separate later plan, same flag)**: write operations (checkout / branch / merge / rebase, etc.), branch filtering, full diff viewer, multi-repository switching, in-graph search.
