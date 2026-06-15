# WARPER-009: local Git, repo metadata, and file edit correctness

## Summary

Warper should port upstream fixes that make local Git chips, diff handling, file edit results, project explorer metadata, and repository watchers reliable at local scale without importing GitHub PR automation, remote SSH project explorer state, or hosted code indexing.

## Why this matters for Warper

Warper's local agent and terminal workflows depend on local repository state instead of hosted code indexing. If local diffs, worktrees, file reads, ignored paths, or project explorer metadata are wrong, the agent will reason from bad context and the user will see misleading local UI. These commits are relevant because they improve the local data plane WARPER-005 needs for file reads, edits, grep/glob/search, and follow-up context. They are not relevant where upstream solved GitHub PR creation, remote SSH project explorer state, or server-backed code intelligence.

## Source commits

| Commit | Resolution | Scope |
| --- | --- | --- |
| `802a881e` | Port | Fix diff handling for untracked directories and nested worktrees. |
| `5bee7a75` | Port | Avoid false diff-detection failure while repo detection is pending. |
| `89f61b63` | Port manually | Limit apply-diff results to changed ranges and local context. |
| `48331870` | Port manually | Cap `get_repo_contents` results and return explicit errors. |
| `5fa22831` | Port | Fix out-of-bounds `read_files` line ranges producing empty results. |
| `9f459842` | Port | Fix symlinked gitignored paths in code review and watchers. |
| `59e802ea` | Port manually | Fix linked-worktree branch checkout by routing users to the worktree path. |
| `2fe9d43c` | Port | Fix stale Git diff chip and code review button state. |
| `1175e82f` | Port | Fix branch/diff chip initialization race. |
| `ffe93a5e` | Port | Fix first-commit Git diff for commit message generation. |
| `1d2775ac` | Port manually | Keep local fork-point/diff-base logic only; exclude GitHub PR creation UI if not retained. |
| `e4695f21` | Port manually | Add Project Explorer hidden-file toggle through local settings only. |
| `43828a6d` | Port | Avoid cloning whole file tree on view update. |
| `03ad9ea9` | Port | Avoid eagerly expanding lazy-loaded repo subtrees. |
| `e8024b5a` | Port | Honor force-included paths in lazy repo metadata. |
| `0f97ef18` | Port manually | Allow partial repo metadata builds when repos exceed max limits. |
| `3497d184` | Port manually | Stop watching gitignored directories while force-including skill paths. |
| `21e70d56` | Port | Avoid panic when file watcher creation fails. |
| `cb4fe42a` | Port manually | Update filesystem watch filters after dependency/API review. |
| `5d8507e4` | Port | Avoid freshly cloned repos getting stuck in loading state. |
| `bd7202f3` | Port manually | Refresh affected file-tree roots instead of rebuilding all roots. |
| `a1b76c28` | Port | Preserve multiline partial-line suffixes during diff validation. |

## Goals / Non-goals

- Goal: make local Git metadata, diff chips, branch chips, code review buttons, commit-message generation, and local file edit results reliable.
- Goal: make local project explorer and repo metadata scale to large repos without excessive CPU, memory, or filesystem watching.
- Goal: preserve force-included local skill/rule paths even when repo metadata is lazily loaded.
- Non-goal: restore hosted code indexing, remote SSH project explorer loading state, cloud workspace state, or GitHub PR creation as a required baseline feature.
- Non-goal: upload repo metadata, diff stats, file tree state, or code review results.

## Behavior

1. Git diff handling reports untracked directories, nested worktrees, and first-commit repositories correctly.
2. Diff and branch chips do not show stale or false failure states while repo detection or metadata initialization is still in progress.
3. Linked-worktree branches are handled by directing the user to the existing worktree instead of running an invalid checkout command.
4. Local file edit results include the changed ranges and enough context for follow-up agent reasoning without dumping whole files unnecessarily.
5. `read_files` returns explicit empty segments or errors for out-of-bounds line ranges instead of silently dropping requested paths.
6. Repo contents APIs cap result size and communicate truncation or errors clearly.
7. Symlinked gitignored paths are attributed to the correct local repo and do not cause invalid Git commands.
8. Project Explorer can show or hide hidden files through a local-only setting.
9. Large file trees avoid expensive full-tree clones, eager subtree expansion, and whole-root rebuilds when a smaller update is enough.
10. Repo metadata can partially build when file limits are exceeded and continue lazily rather than collapsing useful local context.
11. Gitignored directories are not watched unnecessarily, but explicitly force-included local paths such as skill/rule providers remain visible.
12. File watcher creation failures degrade local repo metadata features gracefully and do not crash the terminal.
13. Remote SSH loading-state commits and remote-server watcher motivations are not ported unless Warper keeps a matching local path with the same bug.

## Validation

- Add Git fixtures for untracked directories, nested worktrees, linked worktrees, first commits, symlinked gitignored paths, and fork-point diff-base selection.
- Add UI/model tests for branch chip, diff chip, and code review button initialization and stale-state clearing.
- Add file edit validation tests for changed-range output, multiline partial-line suffix preservation, and out-of-bounds read ranges.
- Add Project Explorer tests for hidden-file filtering, lazy subtree updates, force-included paths, partial metadata builds, and freshly cloned repo loading.
- Add watcher tests for gitignore filtering, watcher creation failure, and selective root refresh.
- Run offline smoke tests to ensure no hosted code indexing, remote SSH, or GitHub PR network path is required.
