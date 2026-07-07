# Watch remote refs for updates — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/10090
Figma: none provided

## Summary
Warp should notice when Git remote-tracking refs stored on disk change for a repository that Warp is already watching. When the changed remote ref is the upstream ref tracked by a watched repository's current branch, Warp refreshes that repository's Git metadata so code review and Git operations UI reflects the new push/fetch state without requiring a manual reload or another unrelated filesystem change.

The first user-visible outcome is that the unpushed commit computation updates after a user pushes a branch. If a successful push updates `.git/refs/remotes/<remote>/<branch>`, Warp should recompute the current branch metadata and stop showing those commits as unpushed.

## Problem
Warp's repository watcher already reacts to local working-tree changes and selected Git metadata changes such as `HEAD`, local branch refs, and `index.lock`. Remote-tracking refs under `.git/refs/remotes/*` are ignored today. As a result, a push or fetch that updates a loose remote ref can leave Warp's cached code review metadata stale. The user may still see a Push action, an outdated unpushed commit list, or stale branch comparison metadata even though Git has already updated the remote-tracking ref on disk.

Users expect Warp's Git UI to track Git state changes that happen through Warp or through external commands in the same repository. Remote refs are part of that local Git state when they are stored as loose refs.

## Goals
- Detect loose remote-tracking ref changes under `.git/refs/remotes/<remote>/<branch>` for repositories already watched by Warp's repo watcher.
- Refresh Git metadata only for watched repositories whose current branch tracks the changed remote ref.
- Update code review and Git operations UI after a push so the unpushed commit list, primary action, and related metadata reflect the new upstream state.
- Preserve the existing handling for local commit-related refs (`HEAD`, `refs/heads/*`) and `index.lock`.
- Support normal repositories and linked worktrees whose remote refs live in the shared common `.git` directory.

## Non-goals
- Watching or parsing packed refs. Remote refs that only change in `.git/packed-refs` are intentionally out of scope for this implementation.
- Fetching from the network, comparing remote state directly, or polling Git. This feature reacts only to local filesystem watcher events.
- Refreshing repositories that do not track the changed remote ref.
- Refreshing metadata for non-current branches that happen to track the changed remote ref. Warp's current Git metadata is scoped to the repository's active branch.
- Changing push, publish, fetch, or PR creation behavior beyond making existing metadata refresh automatically when loose remote refs change.
- Adding new user-facing controls, preferences, telemetry, or feature flags.

## User experience
1. When a watched repository is on a branch with upstream `origin/feature` and Git updates `.git/refs/remotes/origin/feature`, Warp refreshes that repository's Git metadata after the existing watcher debounce and metadata throttling intervals.
2. After the refresh, the code review Git operations UI no longer shows commits as unpushed if `origin/feature..HEAD` is now empty.
3. If the branch still has commits ahead of its upstream after the remote ref update, Warp continues to show those commits as unpushed.
4. If the remote-tracking ref update creates the upstream ref for a newly published branch, Warp refreshes metadata for the repository that now tracks that ref and the primary action can advance from Publish/Push to the next appropriate state.
5. If a different remote ref changes, for example `.git/refs/remotes/origin/main` while the current branch tracks `origin/feature`, Warp does not refresh the feature branch's repository metadata solely because of that event.
6. If two watched worktrees share the same common `.git` directory and only one worktree's current branch tracks the changed remote ref, only that worktree's repository metadata refreshes. Other worktrees sharing the common Git directory are not invalidated unless their current branch tracks the same remote ref.
7. If multiple watched repositories or worktrees currently track the same changed remote ref, each of those repositories refreshes metadata.
8. If the repository has no upstream for its current branch, detached `HEAD`, an unreadable Git config, or a malformed upstream configuration, remote-tracking ref updates do not trigger a metadata refresh for that repository.
9. Existing local ref behavior is unchanged. Changes to `.git/HEAD`, `.git/refs/heads/*`, worktree-specific `HEAD`, and `index.lock` continue to produce the same metadata invalidations and locked-index behavior as before.
10. Remote refs stored only in packed refs are ignored. If a push or fetch updates only `.git/packed-refs`, Warp may remain stale until another existing refresh trigger occurs.
11. The behavior is transparent. Users should not see a new toast, spinner, setting, or prompt specifically for remote-ref watcher events.
12. Filesystem errors, missing refs, and racing Git updates are handled silently in the same style as existing watcher events. A failed attempt to classify a remote ref should not crash Warp or invalidate unrelated repositories.

## Success criteria
1. In a watched repository on a branch tracking `origin/<branch>`, pushing commits so that `.git/refs/remotes/origin/<branch>` updates causes Warp to refresh metadata and remove those commits from the unpushed commit list.
2. The primary Git action no longer remains stuck on Push after the current branch has no unpushed commits.
3. A remote-tracking ref update for an unrelated branch does not refresh metadata for the active repository.
4. A linked worktree whose current branch tracks the changed remote ref refreshes even though the remote ref is stored in the shared common `.git` directory outside the worktree root.
5. A linked worktree sharing the same common `.git` directory but tracking a different remote ref does not refresh.
6. Existing local branch, `HEAD`, and `index.lock` watcher behavior remains covered by regression tests and does not change.
7. Loose-ref behavior is documented and tested; packed-ref updates remain explicitly outside the scope of this feature.

## Validation
- Add unit tests for remote-ref path classification, including `.git/refs/remotes/origin/main`, remote branch names with slashes, worktree paths that must not be treated as shared remote refs, tags, local heads, and `packed-refs`.
- Add watcher routing tests that simulate remote ref changes and assert updates are delivered only to repositories whose current branch tracks the changed ref.
- Add linked-worktree routing tests for a common `.git/refs/remotes/*` update shared across multiple worktrees.
- Add code review metadata tests, or extend existing watcher subscriber tests, to assert that a remote-ref update sets the same metadata-refresh path used for recomputing unpushed commits.
- Manually validate with a real repository: open code review on a branch with unpushed commits, push the branch, and confirm the unpushed commits and primary action update without reopening the pane.
- Manually validate from an external terminal command that updates the same repository while Warp is running.

## Open questions
- None for product behavior. The scope is intentionally limited to loose remote-tracking refs and current-branch upstream metadata.
