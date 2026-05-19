# Product Spec: Project progress context for AI sessions (`.ai/progress.md`)

**Issue:** warpdotdev/warp#10342

## Problem

When a developer works across multiple Warp windows or panes, each AI session
starts completely blank. There is no shared knowledge of:

- Which task is actively being worked on
- What has already been completed
- Project-specific goals or constraints

This forces the user to manually re-explain context at the start of every
conversation — a recurring interruption that compounds with every context
switch.

## Goals

1. AI sessions in any Warp window automatically know the current task and
   what comes next, without the user having to type it in.
2. Task progress self-updates from git commit history; no manual bookkeeping.
3. Opt-in per project — zero impact on projects that don't use this feature.
4. Works alongside the existing `WARP.md` / `AGENTS.md` project rules.

## Non-goals

- Replacing or deprecating `WARP.md` / `AGENTS.md`.
- A GUI for editing `progress.md` (plain-text editor is sufficient for v1).
- Syncing progress state to Warp Drive or the server (all processing is local).

## User experience

### Setup (once per project)

```bash
mkdir -p .ai && cat > .ai/progress.md << 'EOF'
[1] Auth service    done
[2] Payment API     doing
[3] Email service   todo
[4] Unit tests      todo
EOF
```

### Every subsequent AI session (automatic)

When the user opens Warp AI in any window whose working directory is inside
the project, the AI automatically receives:

```
[CURRENT] [2] Payment API
[NEXT]    [3] Email service

[GOAL]
Build a resilient checkout backend.
```

No command needed. No copy-paste. Every window is in sync.

### Auto-advance on commit

```bash
git commit -m "[2] Payment API — charge endpoint complete"
# Next AI session automatically shows [3] Email service as CURRENT
```

### Optional context files

Additional markdown files in `.ai/ctx/` are included when present:

| File                  | Included as | Max lines |
|-----------------------|-------------|-----------|
| `00_goal.md`          | `[GOAL]`    | 15        |
| `05_api.md`           | `[API]`     | 10        |
| `04_constraint.md`    | `[CONSTRAINTS]` | 10   |
| `07_issue.md`         | `[RECENT ISSUES]` | last 3 |

A global fallback directory (`~/.ai/ctx/`) is consulted when a project-level
file is absent, enabling shared defaults across all projects.

## Behaviour invariants

1. If `.ai/progress.md` does not exist in the project, no extra context is
   injected. Existing behaviour is unchanged.
2. If `.ai/progress.md` exists, the assembled context block (≤ 100 lines) is
   injected into every AI session whose `pwd` is within that project tree.
3. Task statuses are derived from `git log --oneline -50` at query time:
   a. A commit whose message contains `[N]` or the task name marks that task
      done.
   b. The first non-done task becomes `doing`.
   c. Remaining non-done tasks remain `todo`.
4. If git is unavailable (non-git project, git not installed), statuses
   already written in `progress.md` are used as-is.
5. Context files in `.ai/ctx/` are read from the project directory first;
   `~/.ai/ctx/` is the fallback.
6. The progress context is always injected **after** `WARP.md`/`AGENTS.md`
   rules so project rules take precedence.
7. The assembled block never exceeds 100 lines to avoid degrading model focus.

## Edge cases

- **Empty progress.md:** No context is injected (no meaningful tasks parsed).
- **All tasks done:** `[CURRENT] (no task in progress)` is emitted; no
  `[NEXT]` line.
- **progress.md parse error on one line:** That line is skipped; valid tasks
  are still processed.
- **git command fails:** Git-derived status update is skipped silently; the
  statuses in `progress.md` are used.
- **ctx files contain only comments/blanks:** The corresponding section is
  omitted from the injected block.
- **Concurrent windows:** All windows read the same on-disk file; context is
  naturally consistent.
