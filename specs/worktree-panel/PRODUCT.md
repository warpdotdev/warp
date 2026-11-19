# Worktrees view

A view in the tools panel (next to Project explorer, Agent conversations, Global search, and Warp
Drive) that lists git repositories and their worktrees, so you can see where coding agents are
working and jump between worktrees with one click.

Gated behind the `WorktreePanel` feature flag (`worktree_panel` Cargo feature, on by default in
this build).

## Layout

- **Toolbelt button**: a branch icon in the tools panel header opens the Worktrees view.
- **Header**: "Worktrees" title with buttons to add a repository (+) and refresh.
- **Repositories**: one collapsible group per repository, named after its main working tree, with
  a worktree count and a + button that opens the New worktree dialog for that repository.
  Repositories with an open terminal start expanded; others start collapsed until toggled.
- **Worktrees**: listed under their repository (main worktree first), each showing its branch (or
  directory name when `HEAD` is detached), a status line, and a status dot.

## Agent status

Each worktree aggregates every terminal whose working directory is inside it. When worktrees are
nested (e.g. `<repo>/.worktrees/<branch>`), a terminal belongs to the most specific one.

| Dot     | Status       | Meaning                                                          |
|---------|--------------|------------------------------------------------------------------|
| Yellow  | Needs input  | An agent is blocked on a permission request or question.         |
| Magenta | Working      | An agent is running a turn.                                      |
| Blue    | Active       | An agent CLI is running but does not report detailed status.     |
| Green   | Idle         | An agent finished its turn and is waiting.                       |
| Gray    | Open         | A terminal is open in the worktree but no agent is running.      |

Supported agents include Warp Agent, cloud agents, and the third-party CLI agents Warp detects
(Claude Code, Codex, Copilot CLI, Gemini CLI, and others). When several agents share a worktree,
the most urgent status wins and the status line reads e.g. "2 agents · Needs input".

## Navigation

Clicking a worktree:

1. stays on the current tab if it already has a terminal in that worktree,
2. otherwise switches to the first tab with a terminal in that worktree,
3. otherwise opens a new tab in the worktree directory,

and then switches the tools panel to the Project explorer, which follows the active tab's terminal
and therefore lists the worktree's files. Click the branch icon to return to the Worktrees view.

## Opening, closing, and resizing

The Worktrees view shares the tools panel's behavior: close it with the panel's ×, resize it by
dragging the panel edge, and open it with the branch icon, the "Toggle worktree panel" action
(bindable in Settings > Keyboard shortcuts), or the Command Palette. Repository groups collapse
and expand individually. "Show worktrees in tools panel" in Settings > Worktrees removes or restores
the tab.

## New worktree dialog

Opened from a repository's + button in the panel (or the existing New worktree entry points):

- **Select repository** and **Base branch**: the branch the worktree starts from.
- **New branch name**: the branch to create for the worktree. Leave empty to autogenerate a name.
  Names may use letters, numbers, and `- _ . /` (e.g. `feature/login`); `/` creates nested
  worktree directories.
- **Fetch latest `<base>` from origin first** (on by default, remembered across uses and mirrored
  in Settings > Worktrees): runs `git fetch origin <base>` and creates the worktree from
  `origin/<base>`, so it starts from the latest remote main rather than a possibly stale local
  branch. The new branch is created with `--no-track`, so it does not track `<base>` and a plain
  `git push` never pushes to it. When unchecked, the worktree starts from the local `<base>`.

## Which repositories are listed

The panel lists repositories Warp already knows about (the same list the New worktree dialog
offers), repositories that open terminals are in, and any added with the + button. Worktrees are
re-listed every 10 seconds while the view is visible, and immediately on refresh. The merged pull
request cleanup keeps running while the view is hidden.
