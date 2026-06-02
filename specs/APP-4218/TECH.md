# APP-4218: Git operations dialogs compare against the branch's actual parent — Tech Spec
Product spec: `specs/APP-4218/PRODUCT.md`
## Context
APP-4218 is about avoiding misleading Git Operations previews when a user creates a branch from another feature branch. The original implementation fell back to the detected default branch when a branch had no upstream, so the Push / Publish dialog could show inherited parent-branch commits as if they belonged to the current branch.
This PR implements the first, low-risk part of that behavior: the no-upstream Push / Publish commit list now falls back to a fork-point SHA instead of the default branch. The PR dialog, PR AI inputs, and `gh pr create --base` path still use the detected default branch in this checkout; they are listed as follow-ups below so the checked-in spec does not overstate what shipped.
Relevant code:
- `app/src/util/git.rs (199-249)` — `detect_fork_point`, which computes a SHA for the point where `HEAD` forked from other refs.
- `app/src/util/git.rs (391-428)` — `get_unpushed_commits`, which uses `<upstream>..HEAD` when an upstream exists and `<fork>..HEAD` only in the no-upstream fallback.
- `app/src/code_review/diff_state.rs (1360-1399)` — `load_metadata_for_repo`, which detects the current branch and upstream, then stores `unpushed_commits` in `DiffMetadata`.
- `app/src/code_review/code_review_view.rs (6770-6796)` — `primary_git_action_mode`, which turns `unpushed_commits` into Publish / Push button state.
- `app/src/util/git.rs (677-904)` — PR diff / AI / create helpers, which still compare against and target the detected default branch.
## Proposed changes
### 1. Add `detect_fork_point`
`detect_fork_point(repo_path, current_branch_name)` returns the SHA where `HEAD` forked from other local or remote refs. It accepts the current branch name so the current branch and `origin/<current>` can be excluded from the comparison set; otherwise the branch would subtract itself and report no unique commits.
The current implementation uses one reachability query plus a `rev-parse`:
```rust
pub async fn detect_fork_point(
    repo_path: &Path,
    current_branch_name: Option<&str>,
) -> Result<Option<String>>;
```
Algorithm:
1. Normalize `current_branch_name`; ignore empty names and detached `HEAD`.
2. Build `git rev-list HEAD --not --exclude=<current> --branches --exclude=origin/<current> --remotes`.
3. Treat the last non-empty line as the oldest commit unique to `HEAD`.
4. Return that commit's parent via `git rev-parse <oldest-unique>^`.
5. If there are no unique commits, resolve `HEAD` itself; if the git commands fail, return `Ok(None)`.
This differs from the earlier plan that introduced a separate `for-each-ref` step and a branch-name detector. The branch-name detector is not present in this implementation.
### 2. Use the fork point only for no-upstream unpushed commits
`get_unpushed_commits(repo_path, current_branch_name, upstream_ref)` keeps the upstream path unchanged:
- If `upstream_ref` exists, run `git log <upstream>..HEAD --format=COMMIT:%H\t%s --numstat`.
- If `upstream_ref` is missing, call `detect_fork_point(repo_path, current_branch_name)` and run `git log <fork>..HEAD ...`.
- If no fork point can be resolved, fall back to `git log HEAD ...`.
This is the behavior that fixes the Publish dialog for stacked no-upstream branches while preserving existing behavior for branches that already have an upstream.
### 3. Keep dialog and metadata wiring unchanged
`DiffStateModel::load_metadata_for_repo` already computes the current branch, upstream ref, and `unpushed_commits` during metadata refresh. The Git Operations button already derives its mode from `unpushed_commits`, `upstream_ref`, uncommitted stats, and PR info.
No new UI state is required. The Push / Publish dialog still receives a `Vec<Commit>` from `DiffStateModel` when opened, so switching the no-upstream fallback is enough to change the included commit list.
### 4. Leave PR helpers on the default branch for this PR
The current code still uses `detect_main_branch` for:
- `get_branch_diff_entries` — Create PR dialog file stats.
- `get_diff_for_pr` — AI PR title / body diff input.
- `get_branch_commit_messages` — AI PR title / body commit-message input.
- `create_pr` — `gh pr create --base <default-branch>`.
This means `PRODUCT.md` behavior around Create PR targeting the detected parent is not fully implemented by this PR. The tech spec should not claim `detect_pr_base_branch` exists or that these helpers use the fork-point SHA.
## End-to-end flow
1. Code review metadata refresh runs in `DiffStateModel::load_metadata_for_repo`.
2. The model resolves `current_branch_name` and optional `upstream_ref`.
3. `get_unpushed_commits` computes either `<upstream>..HEAD` or `<fork>..HEAD`.
4. `CodeReviewView::primary_git_action_mode` uses non-empty `unpushed_commits` plus upstream state to choose Publish or Push.
5. Opening the Push / Publish dialog passes those commits into `GitDialog::new_for_push`.
## Risks and mitigations
### Fork-point SHA is not a PR base branch name
The fork point is enough for a commit range, but `gh pr create --base` needs a branch name. This PR intentionally does not infer a parent branch name from the fork point. Create PR behavior remains default-branch based until that follow-up lands.
### Stale refs can affect fork-point detection
Because `rev-list` subtracts all branches and remotes except the current branch and `origin/<current>`, stale local or remote-tracking refs can make the fork point earlier than a user expects. This is conservative for Publish commit lists but can still be surprising. Pruning stale refs remains the mitigation.
### Remote name assumption
The current self-exclusion only excludes `origin/<current>`. Repos whose current branch is pushed to a differently named remote could still include that remote-tracking ref in the subtraction set. That can hide unique commits after a manual push to a non-origin remote. If that matters, resolve the actual remote-tracking branch before building excludes.
### Root commit / no other refs
If the oldest unique commit has no parent, `rev-parse <sha>^` fails and `detect_fork_point` returns `None`; `get_unpushed_commits` then falls back to logging `HEAD`. This is acceptable for orphan or brand-new repositories.
## Testing and validation
Manual validation for the current implementation:
- Create `feature-a` from main, add commits, then create `feature-b` from `feature-a` without setting an upstream. The Publish dialog on `feature-b` should show only commits unique to `feature-b`, not all of `feature-a`.
- Create a fresh no-upstream branch from main and add commits. The Publish dialog should still show commits since the branch forked from main.
- On a branch with an upstream, the Push dialog should still use `<upstream>..HEAD`.
- Reopen the dialog after rebasing the current branch; metadata refresh should recompute the fallback range from the new graph.
- Create PR dialog validation should expect current behavior for this PR: file stats, AI inputs, and `gh pr create --base` still use the detected default branch.
Recommended unit coverage in `app/src/util/git_tests.rs`:
- `detect_fork_point` returns the original fork commit after main advances beyond the branch point.
- No-upstream `get_unpushed_commits` excludes commits inherited from the parent feature branch.
- Upstream-backed `get_unpushed_commits` remains unchanged and uses `<upstream>..HEAD`.
- Detached `HEAD` does not try to exclude a branch named `HEAD`.
## Follow-ups
- Add parent branch-name detection for PR creation. A likely implementation is to find refs that contain or are closest to the fork point, then choose the best branch name with deterministic tiebreakers.
- Switch `get_branch_diff_entries`, `get_diff_for_pr`, and `get_branch_commit_messages` from default-branch ranges to the detected parent range once branch-name detection exists.
- Switch `create_pr` to pass `--base <detected-parent>` and strip `origin/` when the selected parent is a remote-tracking ref.
- Consider resolving the actual remote-tracking branch for current-branch self-exclusion instead of assuming `origin/<current>`.
