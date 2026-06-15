# WARPER-009: local Git, repo metadata, and file edit correctness

## Summary

Warper should port upstream fixes that make local diff handling, file edit results, project explorer metadata, and repository watchers reliable at local scale without importing Git chip polish, GitHub PR automation, remote SSH project explorer state, or hosted code indexing.

## Why this matters for Warper

Warper's local agent and terminal workflows depend on local repository state instead of hosted code indexing. If local diffs, worktrees, file reads, ignored paths, or project explorer metadata are wrong, the agent will reason from bad context and the user will see misleading local UI. These commits are relevant because they improve the local data plane WARPER-005 needs for file reads, edits, grep/glob/search, and follow-up context. They are not relevant where upstream solved GitHub PR creation, remote SSH project explorer state, or server-backed code intelligence.

## Source commits

| Commit | Resolution | Scope |
| --- | --- | --- |
| `802a881e` | Port | Fix diff handling for untracked directories and nested worktrees. |
| `89f61b63` | Port manually | Limit apply-diff results to changed ranges and local context. |
| `48331870` | Port manually | Cap `get_repo_contents` results and return explicit errors. |
| `5fa22831` | Port | Fix out-of-bounds `read_files` line ranges producing empty results. |
| `9f459842` | Port | Fix repo attribution and watcher behavior for symlinked gitignored paths. |
| `43828a6d` | Port | Avoid cloning whole file tree on view update. |
| `03ad9ea9` | Port | Avoid eagerly expanding lazy-loaded repo subtrees. |
| `e8024b5a` | Port manually | Honor force-included skill/rule paths in lazy repo metadata. |
| `0f97ef18` | Port manually | Allow partial repo metadata builds when repos exceed max limits. |
| `3497d184` | Port manually | Stop watching gitignored directories while force-including skill paths. |
| `21e70d56` | Port | Avoid panic when file watcher creation fails. |
| `5d8507e4` | Port | Avoid freshly cloned repos getting stuck in loading state. |
| `bd7202f3` | Port manually | Refresh affected file-tree roots instead of rebuilding all roots. |
| `a1b76c28` | Port | Preserve multiline partial-line suffixes during diff validation. |

## Deferred upstream commits

| Commit | Decision | Why not in this spec |
| --- | --- | --- |
| `5bee7a75` | Defer | Real code-review UI error-state fix, but not current OpenRouter file-tool correctness. |
| `59e802ea` | Defer | Linked-worktree branch checkout routing is branch-chip command correctness, not OpenRouter file-tool or repo metadata correctness. |
| `2fe9d43c` | Defer | Retained Git chip state correctness, but not current local data-plane pain. |
| `1175e82f` | Defer | Branch/diff chip initialization race is UI state, not WARPER-005 file-tool fidelity. |
| `ffe93a5e` | Defer | First-commit commit-message diff is outside current specs. |
| `1d2775ac` | Defer | PR diff/fork-point workflow needs a Warper-owned local design before porting. |
| `cb4fe42a` | Defer | Watch-filter API change needs dependency/API review before watcher churn. |

## Skipped upstream commits

| Commit | Decision | Why skipped |
| --- | --- | --- |
| `e4695f21` | Skip | Hidden-files toggle is a new Project Explorer preference, not a painkiller. |
| `54712e5d` | Skip | Pending SSH Project Explorer state is remote-session UX, not local Warper baseline. |

## Goals / Non-goals

- Goal: make local Git metadata, file edit results, OpenRouter file tools, and repo metadata reliable where they feed retained local agent and Project Explorer behavior.
- Goal: make local project explorer and repo metadata scale to large repos without excessive CPU, memory, or filesystem watching.
- Goal: preserve force-included local skill/rule paths even when repo metadata is lazily loaded.
- Non-goal: restore hosted code indexing, remote SSH project explorer loading state, cloud workspace state, GitHub PR creation, commit-message generation, hidden-file preferences, or Git chip polish as a required baseline feature.
- Non-goal: upload repo metadata, diff stats, file tree state, or code review results.

## Behavior

1. Local diff/repo metadata includes untracked directories and nested worktree state where agent context or retained local UI consumes that metadata.
2. Local file edit results include the changed ranges and enough context for follow-up agent reasoning without dumping whole files unnecessarily.
3. `read_files` returns explicit empty segments or errors for out-of-bounds line ranges instead of silently dropping requested paths.
4. `get_repo_contents` returns a bounded result and explicit truncation or error metadata instead of materializing unbounded repo entries.
5. Symlinked gitignored paths are attributed to the correct local repo and do not cause invalid Git commands.
6. Large file trees avoid expensive full-tree clones, eager subtree expansion, and whole-root rebuilds when a smaller update is enough.
7. Repo metadata can partially build when file limits are exceeded and continue lazily rather than collapsing useful local context.
8. Gitignored directories are not watched unnecessarily, but explicitly force-included local paths such as skill/rule providers remain visible.
9. File watcher creation failures degrade local repo metadata features gracefully and do not crash the terminal.
10. Remote SSH loading-state commits, hidden-file preferences, GitHub PR workflow fixes, branch-chip checkout routing, and remote-server watcher motivations are not ported unless a later Warper spec owns that product path.

## Validation

- Add Git fixtures for untracked directories, nested worktrees, and symlinked gitignored paths.
- Add file edit validation tests for changed-range output, multiline partial-line suffix preservation, and out-of-bounds read ranges.
- Add repo metadata/model tests for lazy subtree updates, force-included paths, partial metadata builds, and freshly cloned repo loading. Add UI tests only when implementation touches rendering.
- Add watcher tests for gitignore filtering, watcher creation failure, and selective root refresh.
- Run offline smoke tests to ensure no hosted code indexing, remote SSH, or GitHub PR network path is required.
