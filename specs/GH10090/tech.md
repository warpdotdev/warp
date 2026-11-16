# Watch remote refs for updates — Tech Spec
Product spec: `specs/GH10090/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/10090

## Problem
`repo_metadata` already watches repository roots and selected Git internals, but remote-tracking refs under `.git/refs/remotes/*` are filtered out. Code review metadata computes unpushed commits from the current branch's upstream ref, so a push or fetch that updates a loose remote-tracking ref can leave `DiffStateModel` and the Git operations UI stale until another invalidation happens.

The implementation needs to allow loose remote-ref watcher events through, keep each watched `Repository` aware of the loose remote ref tracked by its active branch, refresh that cached tracking state when `HEAD` or Git config changes can alter it, and expose remote-ref invalidations explicitly on `RepositoryUpdate`.

## Relevant code
- `crates/repo_metadata/src/entry.rs (361-484)` — Git internal path helpers: `git_suffix_components`, `extract_worktree_git_dir`, `is_shared_git_ref`, `is_commit_related_git_file`, `is_index_lock_file`, and `should_ignore_git_path`.
- `crates/repo_metadata/src/entry_test.rs (222-395)` — current allowlist and shared-ref tests; remote refs and `.git/config` are currently asserted as ignored.
- `crates/repo_metadata/src/watcher.rs (120-194)` — `DirectoryWatcher::find_repos_for_git_event`, which routes worktree-specific, shared local branch ref, and repo-specific Git events.
- `crates/repo_metadata/src/watcher.rs (293-335)` — `start_watching_directory`, which registers watched paths with the `should_ignore_git_path` filter.
- `crates/repo_metadata/src/watcher.rs (391-529)` — filesystem event handling that converts Git internal events into `RepositoryUpdate`.
- `crates/repo_metadata/src/watcher.rs (588-626)` — `RepositoryUpdate`, especially `commit_updated` and `index_lock_detected`.
- `crates/repo_metadata/src/repository.rs (54-151)` — `Repository` stores `root_dir`, optional per-worktree `external_git_directory`, and optional shared `common_git_directory`.
- `crates/repo_metadata/src/repository.rs (162-229)` — `Repository::start_watching`, which registers the worktree root, per-worktree gitdir, and shared `refs/heads` for linked worktrees.
- `app/src/code_review/diff_state.rs (1116-1247)` — code review repository subscriber maps repository updates to metadata invalidation and throttled metadata refresh.
- `app/src/code_review/diff_state.rs (1347-1393)` — `load_metadata_for_repo` reads `@{u}` and recomputes `unpushed_commits`.
- `app/src/code_review/git_status_update.rs (168-283)` — Git status metadata watcher refreshes when repository metadata flags indicate Git state changed.
- `app/src/util/git.rs (391-468)` — `get_unpushed_commits` computes `<upstream>..HEAD`.

## Current state
`should_ignore_git_path` uses an allowlist for `.git` internals. Only commit-related files (`HEAD` and `refs/heads/*`) and `index.lock` are allowed through; `.git/refs/remotes/origin/main` and `.git/config` are explicitly ignored in `entry_test.rs`.

For normal repositories, the repository root watcher is recursive, so `.git/refs/remotes/*`, `.git/config`, and `.git/HEAD` would be observable if the filter allowed them. For linked worktrees, remote refs and shared config are stored in the shared common `.git` directory, outside the worktree checkout and outside the per-worktree gitdir. `Repository::start_watching` currently adds `common_git_dir/refs/heads` for shared local branch refs, but it does not add `common_git_dir/refs/remotes` or common Git config.

Once a Git internal path reaches `DirectoryWatcher::handle_watcher_event`, it is classified with `is_commit_related_git_file` or `is_index_lock_file`. A `commit_updated` update is enough to trigger the existing code review and git-status metadata refresh paths, but it does not distinguish a local commit/branch update from an upstream remote-ref update. The missing pieces are path classification, cached tracked-upstream state on `Repository`, watcher registration for linked worktrees, scope-aware routing to repositories tracking the changed remote ref, and an explicit `RepositoryUpdate` field for remote-ref changes.

## Proposed changes

### 1. Add remote-tracking ref and tracking-config path helpers
Extend `crates/repo_metadata/src/entry.rs` with helpers for loose remote-tracking refs and Git files that can change a repository's tracked remote ref:

- `is_remote_tracking_ref(path: &Path) -> bool`
  - true for paths under `.git/refs/remotes/<remote>/<branch...>`.
  - false for paths under `.git/worktrees/<name>/...`, `.git/refs/heads/*`, `.git/refs/tags/*`, `.git/packed-refs`, and non-Git paths.
  - requires at least a remote component and one branch component after `refs/remotes`.
- `remote_tracking_ref_path_under_common_git_dir(path: &Path) -> Option<PathBuf>`
  - canonicalization-friendly helper for routing. It returns the full loose ref path only for shared remote refs, not worktree-local files.
- `is_tracking_state_git_file(path: &Path) -> bool`
  - true for files that can change the active branch's tracked remote ref: repository-specific `HEAD`, common `.git/config`, and per-worktree `config.worktree` when present.
  - false for local branch ref files, tags, `packed-refs`, objects, logs, and hooks.

Update `should_ignore_git_path` so loose remote-tracking refs and tracking-state files are allowlisted. Keep `packed-refs` ignored. Do not add broad `refs/*` matching; tags and other Git internals remain filtered out.

Keep `is_commit_related_git_file` focused on `.git/HEAD` and `.git/refs/heads/*`. Remote refs should not be folded into that helper because `RepositoryUpdate` will expose them separately.

### 2. Store the tracked remote ref on `Repository`
Add a cached tracked-upstream field to `crates/repo_metadata/src/repository.rs`:

- `tracked_remote_ref: Option<TrackedRemoteRef>`

Add a small internal type:

- `TrackedRemoteRef`
  - `full_ref_name: String`

`full_ref_name` should be the symbolic full upstream ref returned by Git, for example `refs/remotes/origin/feature`. Store the ref name rather than remote/branch components so Git owns branch config parsing, quoted branch names, worktree config, includes, and other config edge cases.

`Repository` should initialize this field in `Repository::new` or during the first watcher registration, and expose narrow helpers:

- `pub(crate) fn tracked_remote_ref(&self) -> Option<&TrackedRemoteRef>`
- `pub(crate) fn tracks_remote_ref_path(&self, remote_ref_path: &Path) -> bool`
- `pub(crate) fn refresh_tracked_remote_ref(&mut self) -> bool`
- `pub(crate) fn tracked_remote_ref_path(&self) -> Option<PathBuf>`

`refresh_tracked_remote_ref` should run Git in the repository worktree context and update the cached value:

1. Run `git -C <repo_root> rev-parse --symbolic-full-name @{u}`.
2. If Git exits non-zero, cache `None`. This covers detached `HEAD`, no upstream, malformed config, unreadable config, and racing branch changes.
3. Trim stdout to one line.
4. If the ref name does not start with `refs/remotes/`, cache `None`. This excludes local upstreams such as `remote = .` that resolve to `refs/heads/<branch>`.
5. Validate the ref name is relative and does not contain path traversal components.
6. Cache `TrackedRemoteRef { full_ref_name }`.

Implementation details:

- Prefer using an existing Git command helper in the app/repo metadata layer if one is available; otherwise add a narrow helper dedicated to resolving the current upstream ref.
- Do not run Git for every remote-ref event. Run it only on repository construction/startup and allowlisted tracking-state events (`HEAD`, common `.git/config`, and optional `config.worktree`). Remote-ref routing should use the cached value.
- Use the existing repository task queue for Git-backed upstream refreshes so filesystem watcher routing does not block on process execution. Add a task such as `Task::RefreshTrackedRemoteRef { repository: WeakModelHandle<Repository> }`. The task should run the Git command off the watcher event path, then update the repository cache on completion. If the cached value changed, enqueue `RepositoryUpdate { remote_ref_updated: true, ..Default::default() }` for that repository's subscribers. Do not broaden remote-ref routing to every repository as a shortcut.
- `tracked_remote_ref_path` should compute `self.common_git_dir().join(full_ref_name)` from the cached `TrackedRemoteRef`; do not store the path separately as duplicate state.
- `tracks_remote_ref_path` should compare the changed path to the computed tracked remote ref path, normalizing/canonicalizing existing parent paths where possible.

This cached field is the scope boundary that satisfies the product requirement that only repositories tracking the changed remote ref refresh metadata.

### 3. Refresh cached tracking state when it can change
The tracked remote ref for a watched repository can change when any of these local files change:

- `self.git_dir()/HEAD`: the active branch changes, `HEAD` becomes detached, or `HEAD` is reattached to a branch with a different upstream.
- `self.common_git_dir()/config`: branch upstream config changes, including `git branch --set-upstream-to`, `git branch --unset-upstream`, `git push -u`, `git remote rename`, and manual edits to `branch.<name>.remote` or `branch.<name>.merge`.
- `self.git_dir()/config.worktree`: worktree-specific upstream config changes, if worktree-specific config is enabled and this repo supports reading it.

Update watcher handling so events for `is_tracking_state_git_file(path)` enqueue a tracked-remote-ref refresh task for each affected repository instead of running Git inline. When the queued task completes, it should compare the resolved upstream ref to the cached value. If `refresh_tracked_remote_ref` changed the cache, enqueue `RepositoryUpdate { remote_ref_updated: true, ..Default::default() }` for that repository.

Routing for tracking-state files should be scoped by where the file lives:

1. Worktree-specific `HEAD` and `config.worktree` under `.git/worktrees/<name>/...` route only to that linked worktree.
2. Normal repo `.git/HEAD` routes only to the repository whose working tree owns that `.git` directory.
3. Common `.git/config` routes to all watched repositories whose `common_git_dir()` matches that `.git` directory, then each repository decides whether its cached `tracked_remote_ref` changed.

A common config edit may affect multiple watched worktrees if Git resolves a different upstream for their active branches. It may also affect none of them. The config event may enqueue refresh tasks for every watched repository sharing that common Git directory, but only repositories whose resolved upstream changed should receive `remote_ref_updated`. Do not broaden remote-ref refreshes to every watched worktree as a substitute for the per-repository cache check.

### 4. Register shared Git paths for linked worktrees
Update `Repository::start_watching` in `crates/repo_metadata/src/repository.rs` so linked worktrees watch shared Git paths from the common Git directory in a way that includes remote-tracking refs and shared config.

Preferred registration for linked worktrees:

- the worktree root, as today.
- the per-worktree gitdir, as today, for `HEAD`, `index.lock`, and optional `config.worktree`.
- `common_git_dir/refs`, when it exists, so shared local branch refs and remote-tracking refs are visible.
- `common_git_dir/config`, if the watcher can register a file path; otherwise `common_git_dir` with the existing allowlist filter.

This lets a linked worktree observe both existing remote refs and first-time creation under `refs/remotes` without having to create Git directories from Warp. It also lets upstream tracking additions/removals in common config update the cached `tracked_remote_ref`.

Update `Repository::stop_watching` to unregister the same shared paths when the last subscriber is removed. Keep start/stop symmetric, preferably by sharing a helper that computes the optional watch paths.

For normal repositories, no extra watch path is required because the root watcher already recursively covers `.git/refs/remotes`, `.git/config`, and `.git/HEAD` once the filter allows those paths.

### 5. Add remote-ref routing in `DirectoryWatcher`
Update `DirectoryWatcher::find_repos_for_git_event` in `crates/repo_metadata/src/watcher.rs` with a routing tier for shared remote-tracking refs:

1. Worktree-specific paths under `.git/worktrees/<name>/...` keep the existing highest-priority route.
2. Remote-tracking refs under `.git/refs/remotes/*` route to watched repositories whose `common_git_dir()` contains the event path and whose `Repository::tracks_remote_ref_path(event_path)` returns true based on the cached full upstream ref name.
3. Shared local branch refs under `.git/refs/heads/*` keep the existing broadcast-to-shared-common-git-dir route.
4. Common `.git/config` routes to all watched repositories sharing that common Git directory so each can refresh its cached tracked remote ref.
5. Repo-specific paths keep the existing fallback route.

The remote-ref tier should deduplicate repository handles just like the existing tiers. It should log the routing tier and affected repo count with the existing `[GIT_EVENT_ROUTING]` pattern.

Remote-ref filesystem events should produce `RepositoryUpdate { remote_ref_updated: true, ... }` synchronously after cached-path routing. Tracking-state events should not produce an immediate subscriber update; they enqueue `Task::RefreshTrackedRemoteRef`, and task completion produces `remote_ref_updated = true` only for repositories whose cached `tracked_remote_ref` changed. No Git internal path should be added to `added`, `modified`, `deleted`, or `moved`, matching the current treatment for local Git metadata files.

### 6. Add a queued tracked-ref refresh task
Extend `TaskQueue` in `crates/repo_metadata/src/watcher.rs` with a task dedicated to refreshing the cached upstream ref:

- `Task::RefreshTrackedRemoteRef { repository: WeakModelHandle<Repository> }`

The task should:

1. Upgrade the repository handle.
2. Run `refresh_tracked_remote_ref` for that repository, using Git to resolve `@{u}` outside the watcher event path.
3. If the cached value changed, collect the repository's current subscriber IDs and enqueue `Task::Update` with `RepositoryUpdate { remote_ref_updated: true, ..Default::default() }` for each subscriber.
4. If the repository was dropped, Git fails, or the resolved upstream is unchanged, complete without delivering a subscriber update unless the cache changed to or from `None`.

This preserves ordering enough for correctness: if `git push -u` updates both `.git/config` and a remote ref and the remote-ref event arrives before the cache refresh completes, that remote-ref event may not match the old cache. The queued config refresh still emits `remote_ref_updated` when the tracked upstream changes, so code review metadata refreshes without requiring broad remote-ref routing.

### 7. Add `RepositoryUpdate.remote_ref_updated`
Extend `crates/repo_metadata/src/watcher.rs`:

- `pub remote_ref_updated: bool`
  - true when the repository's tracked upstream state changed, or when the loose remote-tracking ref currently tracked by the repository changed.

Update all `RepositoryUpdate` plumbing:

- `RepositoryUpdate::is_empty` should include `!self.remote_ref_updated`.
- `merge_repository_updates` should OR `remote_ref_updated`, like `commit_updated` and `index_lock_detected`.
- Destructuring call sites, tests, and default builders should include the new field.
- Logs should distinguish `commit_updated`, `remote_ref_updated`, and `index_lock_detected`.

Keep `commit_updated` for local commit/branch state (`HEAD` and `refs/heads/*`). Use `remote_ref_updated` for upstream state so consumers can reason about refresh causes without conflating local and remote ref changes.

### 8. Preserve code review and git status behavior
Consumers should refresh metadata when either `commit_updated` or `remote_ref_updated` is true.

Existing flow after the watcher update:

1. `Repository` caches that the active branch tracks `.git/refs/remotes/origin/feature`.
2. Watcher sees `.git/refs/remotes/origin/feature` change.
3. `find_repos_for_git_event` routes only to repositories whose cached full upstream ref name computes to that tracked remote ref path.
4. `handle_watcher_event` enqueues `RepositoryUpdate { remote_ref_updated: true, ... }`.
5. `DiffStateModel::handle_file_update` treats `remote_ref_updated` like `commit_updated` for metadata invalidation and schedules the throttled metadata refresh.
6. `load_metadata_for_repo` re-reads `@{u}` and recomputes `get_unpushed_commits`.
7. `CodeReviewView` observes the updated metadata and recalculates the primary Git action.

Tracking-add/remove flow:

1. Git updates `.git/config` after `git push -u`, `git branch --set-upstream-to`, or `git branch --unset-upstream`.
2. The watcher routes the config event to repositories sharing that common Git directory.
3. Each repository calls `refresh_tracked_remote_ref`.
4. Repositories whose cached tracked remote ref changed receive `RepositoryUpdate { remote_ref_updated: true, ... }`.
5. Code review and Git status metadata refresh from the same path used for remote-ref content updates.

`GitRepoStatusModel::should_refresh_metadata` should also return true for `remote_ref_updated`, so branch/status metadata remains consistent.

### 9. Tests
Add and update unit tests in `crates/repo_metadata`:

- `entry_test.rs`
  - `should_ignore_git_path` does not ignore `.git/refs/remotes/origin/main`.
  - `should_ignore_git_path` does not ignore `.git/config` or `.git/worktrees/<name>/config.worktree`.
  - remote branch names with slashes are recognized.
  - `.git/refs/remotes/origin` without a branch is not recognized.
  - `.git/packed-refs`, `.git/refs/tags/*`, `.git/refs/heads/*`, and worktree-local paths are not remote-tracking refs.
  - existing local branch and index-lock assertions remain unchanged.
- `repository.rs` tests or a new `repository_tests.rs`
  - `tracked_remote_ref` initializes from a mocked or fixture-backed `git rev-parse --symbolic-full-name @{u}` result such as `refs/remotes/origin/feature`.
  - `tracked_remote_ref_path` computes `common_git_dir/refs/remotes/origin/feature` from cached full ref name state.
  - `tracks_remote_ref_path` returns true for the computed loose remote ref path.
  - slash-containing branch names are preserved in the full ref name returned by Git.
  - `refresh_tracked_remote_ref` returns true when Git reports tracking was added, removed, or changed to a different upstream ref, and false when the resolved upstream ref is unchanged.
  - returns/caches `None` for detached `HEAD`, no upstream, Git command failure, local upstreams such as `refs/heads/main`, malformed output, absolute paths, and path traversal components.
  - runs Git in the worktree root so linked worktrees resolve their own active branch and worktree-specific config correctly.
- `watcher_tests.rs`
  - a matching remote-ref event is delivered to a subscriber as `remote_ref_updated = true`, `commit_updated = false`, and contains no file-list changes.
  - an unrelated remote-ref event is not delivered to a repository that tracks a different upstream.
  - two repositories/worktrees tracking the same remote ref both receive the update.
  - a `.git/config` change that adds, removes, or changes the active branch upstream enqueues `Task::RefreshTrackedRemoteRef`, and task completion emits `remote_ref_updated = true`.
  - a `.git/config` change unrelated to the active branch may enqueue `Task::RefreshTrackedRemoteRef`, but task completion does not emit `remote_ref_updated` when the resolved upstream is unchanged.
  - linked worktrees register/unregister shared refs and shared config in a way that covers `refs/remotes`, including first-time creation under `refs/remotes` when the common `refs` directory exists.
  - local `refs/heads/*`, worktree-specific `HEAD`, and `index.lock` routing continue to pass existing regression tests.

Update app-level tests only if repo-metadata tests cannot prove the end-to-end invalidation contract. The key app-level assertion is that `DiffStateModel::handle_file_update` treats `remote_ref_updated` as full metadata invalidation, matching `commit_updated`.

### 10. Manual validation
1. Open Warp code review on a branch tracking `origin/<branch>` with one or more unpushed commits.
2. Push the branch from Warp or an external terminal.
3. Confirm the loose ref `.git/refs/remotes/origin/<branch>` updates.
4. Confirm the unpushed commit list clears and the primary Git action updates without reopening code review.
5. Run `git branch --unset-upstream`, then `git branch --set-upstream-to=origin/<branch>`, and confirm metadata refreshes when tracked remote ref state is removed and restored.
6. Repeat with another branch's remote ref update and confirm the active branch does not refresh.
7. Repeat in a linked worktree whose common `.git` directory is outside the worktree checkout.

## Risks and mitigations

### Risk: over-invalidating every worktree sharing a common `.git`
Shared refs and shared config are visible to every linked worktree. Broadcasting remote-ref changes to all watched worktrees would satisfy freshness but violate the product requirement and create unnecessary metadata work.

Mitigation: cache only Git's resolved full upstream ref name on each `Repository` and compute the loose remote ref path from `common_git_dir()` when routing. For common config changes, deliver updates only when `refresh_tracked_remote_ref` changes the cached value. Tests must include two worktrees sharing the same common `.git` directory but tracking different upstream refs.

### Risk: cached tracked remote ref becomes stale
If the watcher misses a `HEAD` or config event, remote-ref routing could use an outdated cached upstream ref name.

Mitigation: initialize the cache in `Repository::new`, refresh it on all allowlisted tracking-state events, and refresh it during initial scan/start-watching if needed. Keep existing manual and metadata refresh paths as backstops. Prefer conservative false negatives over routing unrelated remote refs to every repository.

### Risk: running Git from watcher-triggered state refreshes
Resolving the upstream with Git is more correct than parsing config, but it introduces process execution when `HEAD` or config changes.

Mitigation: run Git only on repository construction/startup and tracking-state events, not on every remote-ref event. Use `Task::RefreshTrackedRemoteRef` in the existing repository task queue so watcher routing never blocks on Git. Cache `None` on Git failures and let existing manual/other refresh paths handle the repository.

### Risk: over-broad shared Git watching
Watching `common_git_dir/refs` or `common_git_dir` for linked worktrees is broader than the current `refs/heads` registration.

Mitigation: keep `should_ignore_git_path` as the allowlist boundary. Only local branch refs, remote-tracking refs, tracking-state files, and index/HEAD files should pass through; tags and other refs remain ignored. Add tests proving tag changes and unrelated Git internals under the broader watched directory do not produce repository updates.

### Risk: stale reads while Git is updating refs or config
A filesystem event can fire while Git is still writing a ref or replacing config.

Mitigation: route based on path and cached tracking state, refresh cached tracking state after config/HEAD events by asking Git for the current upstream, then reuse the existing debounced watcher and throttled metadata refresh. If metadata load races and observes stale state, later Git file events or manual refresh paths should correct it. Avoid adding bespoke retry loops unless tests prove they are necessary.

## Follow-ups
- Support `.git/packed-refs` if product later needs packed remote refs to trigger metadata refresh.
- Consider a shared utility for resolving the current branch's upstream ref if other Git UI features need the same information.
- Consider adding telemetry around metadata refresh causes if remote-ref invalidations become performance-sensitive.
