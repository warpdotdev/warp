# APP-4218: Git operations dialogs compare against the branch's actual parent — Tech Spec
Product spec: `specs/APP-4218/PRODUCT.md`
## Context
Today the Push / Publish dialog, the Create PR dialog, and their AI helpers hard-code the repo's main branch as the comparison base whenever the current branch has no upstream. On a branch-off-a-branch, every commit inherited from the parent branch shows up as "included" in the push / PR.
All of the offending code paths already route their base through one function call: `detect_main_branch`. The fix is to introduce a `detect_parent_branch` helper that returns the closest-ancestor branch (falling back to main), and swap the four callers that currently say `detect_main_branch` to say `detect_parent_branch`. No function signatures change.
Relevant code:
- `app/src/util/git.rs:316-359` — `get_unpushed_commits`: `git log @{u}..HEAD` with `main..HEAD` fallback.
- `app/src/util/git.rs:602-634` — `get_branch_diff_entries`: `main..<end>`.
- `app/src/util/git.rs:743-766` — `get_diff_for_pr`: `main..<end>`, feeds AI.
- `app/src/util/git.rs:775-784` — `get_branch_commit_messages`: `main..HEAD`, feeds AI.
- `app/src/util/git.rs:803-826` — `create_pr`: invokes `gh pr create` without `--base`.
- `app/src/code_review/git_dialog/{push,pr}.rs` — per-dialog state, unchanged in shape. The detected parent is used transparently through the four util helpers.
## Proposed changes
### 1. `detect_parent_branch`
```rust path=null start=null
// app/src/util/git.rs
/// Returns the closest-ancestor branch of `HEAD`, or the main branch when
/// no candidate qualifies. Ties prefer the detected main branch, then local
/// over `origin/*`, then alphabetical for determinism.
pub async fn detect_parent_branch(repo_path: &Path) -> Result<String>;
```
Implementation (all upfront queries run in parallel via `futures::join!`):
1. `git for-each-ref --merged HEAD --format='%(objectname) %(refname:short)' refs/heads refs/remotes` to list ancestor refs with their commit SHAs. `--merged HEAD` filters out non-ancestors at the git level, avoiding per-candidate subprocess spawns.
2. `git log HEAD --format=%H` to walk HEAD's history once. A `HashMap<&str, usize>` of `sha → position` gives each candidate's distance from HEAD in O(1) lookups.
3. Resolve the actual upstream via `git rev-parse --abbrev-ref --symbolic-full-name @{u}` and exclude it (plus the current branch name) from candidates. Handles non-`origin` upstream configurations.
4. Rank candidates by `(distance, !is_main, !is_local, name)`. Log the winner at debug.
5. If no candidate qualified, return `detect_main_branch(repo_path)`.
Return type is a plain `String` — either a local branch (`feature-a`) or a remote-tracking ref (`origin/feature-a`).
### 2. Swap `detect_main_branch` for `detect_parent_branch` inside the four helpers
No caller / signature changes. Each helper keeps its current shape; only the internal base-branch lookup changes:
- `get_unpushed_commits`: the no-upstream fallback branch becomes `detect_parent_branch` instead of `detect_main_branch`. The primary `@{u}..HEAD` path is unchanged (when an upstream exists, it's still the most accurate "what will be pushed"). When upstream is unset, the fallback now uses the closest ancestor.
- `get_branch_diff_entries`: `let base = detect_parent_branch(repo_path).await?;` in place of the current `detect_main_branch` call. The `{base}..{end_ref}` range logic is unchanged.
- `get_diff_for_pr`: same one-line swap.
- `get_branch_commit_messages`: same one-line swap.
### 3. `create_pr` passes `--base`
`create_pr` internally calls `detect_parent_branch`, strips any `origin/` prefix, and passes `--base <parent>` to `gh pr create`. Signature unchanged:
```rust path=null start=null
pub async fn create_pr(
    repo_path: &Path,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<PrInfo> {
    let base = detect_parent_branch(repo_path).await?;
    let base = base.strip_prefix("origin/").unwrap_or(&base).to_string();
    // ...existing gh pr create invocation, plus --base <base>...
}
```
If detection errors, propagate the error — the caller's existing `user_facing_git_error` path shows the generic failure toast.
### 4. Dialogs
No dialog-level plumbing. The four util helpers (`get_unpushed_commits`, `get_branch_diff_entries`, `get_diff_for_pr`, `get_branch_commit_messages`) already feed the Push and Create-PR dialogs; swapping them to `detect_parent_branch` internally is enough. The Commit dialog's `CommitAndCreatePr` chain inherits the fix via `create_pr` (§3).
Surfacing the detected parent in the dialog chrome (e.g. a "Based on" row) was evaluated and dropped for now — it added visible latency waiting on detection to resolve, with limited user value. Tracked as a follow-up.
### 5. Feature flag gating
All changes live under `FeatureFlag::GitOperationsInCodeReview` (already gating the dialogs); no new flag.
## Risks and mitigations
### Heuristic picks the wrong branch
Two branches pointing at the same commit, deleted historical parents, etc. The parent isn't visible in the UI right now, so bad detections only manifest as a wrong commit list / wrong PR base. A follow-up can surface the parent or add a per-branch override.
### Cost of repeated detection
`detect_parent_branch` runs inside each of the four helpers on every dialog open (and once more in `create_pr`). Each detection is 2–4 parallel subprocess calls (`for-each-ref --merged HEAD`, `log HEAD --format=%H`, `rev-parse @{u}`, `detect_main_branch`) regardless of branch count — typically sub-100ms even in repos with thousands of remote-tracking refs. For the common PR-create flow, that's ~4× the cost on top of the AI call, which is already the dominant latency. Acceptable. If large-repo latency becomes measurable, cache per-repo inside `detect_parent_branch` itself (follow-up).
### PR targets an unpushed base
If the parent is a local-only branch, `gh pr create --base <b>` fails. Surfaces via the generic "Git operation failed." toast; we do not silently retry without `--base`.
### Backwards compatibility
Fresh-feature-off-main shapes resolve to `main`, so today's behavior is preserved on the common path.
## Testing and validation
References below are to `specs/APP-4218/PRODUCT.md` success criteria.
### Manual validation
- `feature-a` (pushed), `git checkout -b feature-b feature-a`, 1 new commit, no push: Publish dialog shows 1 commit (SC 1). Create PR shows only those files; confirming runs `gh pr create --base feature-a` and the PR targets `feature-a` (SC 2).
- Fresh branch off main, no upstream: dialog shows `main..HEAD` (SC 3).
- Rebase `feature-b` onto `main`, reopen dialog: commits list reflects the rebased range (SC 4).
- Commit-and-create-PR on `feature-b`: PR targets `feature-a` (SC 6).
- Change the pane's diff-mode dropdown; reopen dialogs: previews are unchanged (SC 7).
### Integration / screenshot coverage
None added. The `git_dialog` module ships without an integration harness today (see `specs/APP-4125/TECH.md`).
## Follow-ups
- **Surface the detected parent in the dialog chrome** (a "Based on" row) once we have a way to populate it without visible latency — e.g. caching it on `DiffMetadata` so it's ready by the time the dialog opens.
- **Per-branch override** for mis-detected parents (stored in `.git/config` as `branch.<name>.warpParent`).
- **Per-repo caching** inside `detect_parent_branch` if repeated detection shows up in profiles.
