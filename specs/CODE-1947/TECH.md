# CODE-1947: PR stacking UX in the code review pane — Tech Spec

Product spec: `specs/CODE-1947/PRODUCT.md`

Status: Draft. The product assumptions require requester approval before implementation.

The authoritative assumptions are listed near the top of `PRODUCT.md`. Changing V1 from read-only navigation to write operations, or changing layer navigation from a two-revision diff to branch checkout, changes this architecture.

## Context
The code review pane is a repository-scoped view in the right panel. It currently discovers one pull request for the checked-out branch, computes every diff from local Git state, and shares one in-memory comment batch per repository.

The implementation was researched at commit [`4f15a21bac92219f298d6cab94fe690437e61eb6`](https://github.com/warpdotdev/warp/tree/4f15a21bac92219f298d6cab94fe690437e61eb6).

Relevant code:
- [`app/src/util/git.rs (718-940) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/util/git.rs#L718-L940) defines `PrInfo`, runs `gh pr view`, and has no stack fields.
- [`app/src/code_review/github_repo_model/mod.rs (14-100) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/github_repo_model/mod.rs#L14-L100) exposes one current-branch `PrInfo` through unified local and remote backends.
- [`app/src/code_review/github_repo_model/local.rs (18-218) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/github_repo_model/local.rs#L18-L218) refreshes GitHub metadata on branch changes and a 60-second timer.
- [`app/src/code_review/diff_state/mod.rs (304-330) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/diff_state/mod.rs#L304-L330) models `Head`, `MainBranch`, and `OtherBranch`; every mode has an implicit current working tree or `HEAD` endpoint.
- [`app/src/code_review/diff_state/local.rs (510-579) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/diff_state/local.rs#L510-L579) changes diff modes and loads local Git diffs.
- [`app/src/code_review/diff_state/local.rs (1482-1621) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/diff_state/local.rs#L1482-L1621) resolves merge bases and dispatches mode-specific loaders.
- [`app/src/code_review/code_review_view.rs (368-690) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/code_review_view.rs#L368-L690) owns the pane, loaded files, active repository, active comment model, diff model, and GitHub model.
- [`app/src/code_review/code_review_view.rs (1486-1578) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/code_review_view.rs#L1486-L1578) builds the current branch-based diff selector and applies a new `DiffMode`.
- [`app/src/code_review/code_review_view.rs (6372-6560) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/code_review_view.rs#L6372-L6560) derives Git operations and the PR action from current-branch state.
- [`app/src/code_review/comments/batch.rs (14-204) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/code_review/comments/batch.rs#L14-L204) stores native and imported comments, with pending imported comments keyed only by `DiffMode`.
- [`app/src/pane_group/working_directories.rs (290-520) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/app/src/pane_group/working_directories.rs#L290-L520) shares one `ReviewCommentBatch` per repository.
- [`crates/persistence/migrations/2025-09-29-154015_add_code_review_pane/up.sql @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/crates/persistence/migrations/2025-09-29-154015_add_code_review_pane/up.sql) persists only `terminal_uuid` and `repo_path`.
- [`crates/remote_server/proto/diff_state.proto (165-205) @ 4f15a21`](https://github.com/warpdotdev/warp/blob/4f15a21bac92219f298d6cab94fe690437e61eb6/crates/remote_server/proto/diff_state.proto#L165-L205) mirrors `PrInfo` and diff metadata for remote repositories.

GitHub's Stacks REST response supplies ordered pull request numbers, state, draft status, merged timestamp, head ref, head SHA, and the trunk ref. It does not supply every display field or an explicit base SHA for every row. Warp therefore needs both stack topology and enriched pull request metadata before it can render exact, titled layer ranges.

## Proposed changes
### 1. Add an independent feature flag
Add `PrStackingInCodeReview` to the shared feature-flag declaration and app registration, following the existing `GitOperationsInCodeReview` pattern.

The flag gates:
- Stack API calls.
- The stack control and map.
- The pull request layer diff mode.
- Stack-specific telemetry.

The flag does not gate existing current-branch pull request lookup or working-tree review.

### 2. Add GitHub stack domain types and discovery
Keep current-branch `PrInfo` and stack topology separate. `PrInfo` remains useful when a pull request is not stacked. Add types with this intended shape:

```rust
pub struct PrStackInfo {
    pub number: u64,
    pub trunk_ref: String,
    pub layers: Vec<PrStackLayer>, // bottom to top
}

pub struct PrStackLayer {
    pub pr: PrInfo,
    pub title: String,
    pub head_ref: String,
    pub head_oid: String,
    pub base_ref: String,
    pub base_oid: String,
    pub merged_at: Option<DateTime<Utc>>,
}
```

The data contract must guarantee:
- `layers` is ordered bottom to top.
- Every layer has immutable base and head object IDs for the loaded snapshot.
- Each layer uses the base ref and object ID reported by that pull request. Do not derive the diff base only from adjacency because GitHub can retarget remaining layers after a partial merge.
- Stack membership with fewer than two pull requests normalizes to `None`.

Add `stack_info()` and `is_refreshing_stack_info()` to `GitHubRepoModel`, plus `GitHubRepoEvent::StackInfoChanged`.

The local refresh sequence is:
1. Resolve current-branch `PrInfo` with the existing `gh pr view`.
2. Resolve owner and repository with existing `RepositoryInfo`.
3. Run `gh api --method GET repos/{owner}/{repo}/stacks?pull_request={number}` with `X-GitHub-Api-Version: 2026-03-10`.
4. If one stack is returned, enrich all pull requests in one generated `gh api graphql` query. Query `title`, `url`, `state`, `isDraft`, `mergedAt`, `baseRefName`, `baseRefOid`, `headRefName`, and `headRefOid` by pull request number.
5. Preserve the REST order and validate that every listed number has enriched metadata and base/head object IDs. Do not reject a snapshot only because current pull request base refs no longer form the original branch chain; a partial stack merge can legitimately retarget the remaining pull requests.

A single GraphQL enrichment call avoids one `gh pr view` process and network round trip per layer. It also supplies exact base object IDs instead of deriving them from mutable local refs.

Do not depend on the optional `gh stack` extension in V1. `gh api` reuses the existing GitHub CLI authentication path and works for users who have not installed the extension.

Classify discovery results:
- `200` with one valid stack: `Available(PrStackInfo)`.
- `200` with an empty result: `NotStacked`.
- `404`: `Unavailable`; suppress the feature because the preview, repository, or permissions may not expose it.
- Missing `gh`, authentication failure, timeout, malformed response, or network failure: `Unavailable` with a diagnostic for logs and telemetry.

Use a branch/PR generation token for asynchronous refreshes. A result is applied only if it still matches the branch and pull request that started the request. Preserve a valid cached stack snapshot during a transient refresh failure. Clear it immediately on a branch change.

### 3. Add a two-revision pull request diff mode
Extend the diff model with an explicit commit-range mode:

```rust
DiffMode::PullRequestLayer {
    pr_number: u64,
    base_oid: String,
    head_oid: String,
}
```

This is intentionally different from `OtherBranch`. `OtherBranch` changes only the base and still uses the checked-out branch and working tree as the endpoint. A pull request layer must use two GitHub-supplied object IDs and exclude working-tree changes.

Add a loader that:
1. Validates object IDs as full hexadecimal Git object IDs before passing them to Git.
2. Ensures the head object exists locally. If it does not, fetch `refs/pull/{number}/head` into a Warp-owned ref under `refs/warp/code-review/pr/{number}/head`.
3. Ensures the base object exists. The head fetch normally includes an ancestor base. If it does not, fetch the same-repository base branch into `refs/warp/code-review/pr/{number}/base`.
4. Verifies both expected object IDs with `git cat-file -e <oid>^{commit}` after fetching.
5. Computes the file diff and base content from `base_oid...head_oid`, using the same size limits and editor construction as existing branch modes.

Fetching may update only Warp-owned refs and Git object storage. It must not update `HEAD`, the index, the working tree, user branches, or normal remote-tracking branches.

Pull request layer mode has no watcher-driven single-file invalidation. Local file changes are unrelated to an immutable remote commit range. It reloads only when the selected layer changes or refreshed GitHub metadata changes its base/head object IDs.

Although V1 hides stack UX for remote repositories, `DiffMode` crosses the local/remote model boundary. Update exhaustive conversions safely:
- Add a protocol representation for the new range so serialization remains total.
- Reject or avoid selecting the mode in `RemoteDiffStateModel` while `RemoteCodeReview` parity is out of scope.
- Do not add daemon-side stack discovery or remote Git fetch behavior in V1.

### 4. Enforce a read-only editor surface
Add an explicit read-only review-source property rather than inferring safety from branch equality.

When `DiffMode::PullRequestLayer` is active:
- Construct editors from Git object content, not the current global file buffer.
- Disable text mutation and save actions.
- Hide or disable discard actions.
- Do not send watcher updates into the historical editor.
- Keep selection, copy, find, file navigation, comment composition, comment submission, and diff-as-context.
- Label any action that opens a working-tree file as leaving the historical view; do not silently replace the historical editor with an editable buffer.

Centralize this check in a method such as `DiffMode::is_read_only()` so UI actions and editor construction use one invariant.

### 5. Add stack selection state and UI
Add a `stack_map` module under `app/src/code_review/` and expose it through `CodeReviewHeaderFields`.

`CodeReviewView` owns ephemeral state:
- Latest `PrStackInfo`.
- Stack discovery state.
- Optional selected pull request number.
- Per-review-context file expansion and scroll restoration.

The stack map receives immutable presentation rows and emits:
- `SelectLayer(pr_number)`.
- `OpenPullRequest(url)`.
- `Close`.

Selection resolves the layer from the latest stack snapshot and calls `set_diff_mode(PullRequestLayer { ... })`. Returning to `Head` clears the selected layer and restores the existing working-tree view state.

Derive header actions from a new review context:

```rust
enum ReviewContext {
    WorkingTree,
    PullRequestLayer { pr_number: u64 },
}
```

In `PullRequestLayer`, replace current-branch Git operations with "Open PR #N" for the selected layer and suppress mutation actions. Do not reinterpret the current branch's create/push state as state for the selected layer.

Keep the stack control outside the existing branch-base `DiffSelector`. The existing selector continues to own "Uncommitted changes," main, and arbitrary branch comparisons. The stack map owns pull request layer selection. Selecting a branch-based diff exits stack review.

### 6. Partition comments by review context
The current `WorkingDirectoriesModel.comment_models` key is only `LocalOrRemotePath`, which allows one repository-wide batch. Replace the value with a per-context collection or key the map by:

```rust
struct ReviewCommentContextKey {
    repo: LocalOrRemotePath,
    source: ReviewSourceKey,
}

enum ReviewSourceKey {
    WorkingTree,
    PullRequest { number: u64 },
}
```

Do not key native drafts only by base/head object IDs. A force-push changes object IDs but must not move the draft to another pull request. Store the base/head pair on each comment, as today, for relocation and outdated detection.

Update imported-comment insertion to require a pull request context when the source is a pull request. Pending imported comments remain keyed by diff mode inside that pull request's batch. Existing callers that import comments for the working-tree flow use `WorkingTree`.

When review context changes:
1. Unsubscribe from the old `ReviewCommentBatch`.
2. Swap `active_comment_model`.
3. Update the comment list and editor markers.
4. Reposition only the selected context's comments after its diff loads.

Extend `AgentReviewCommentBatch` or its surrounding submission metadata with the selected pull request number and URL. This preserves the existing "send comments to agent" workflow without asking the agent to infer which stack layer was reviewed. It does not authorize the agent to create or mutate stacks.

### 7. Keep pane persistence unchanged
Do not migrate `code_review_panes` in V1.

The persisted snapshot remains `terminal_uuid + repo_path`. Stack topology and pull request object IDs are remote, mutable data and must be rediscovered. Restored panes start in `Head` mode. In-memory comment batches and selected-layer state keep their current application-lifetime semantics.

This decision avoids restoring a deleted pull request or stale object IDs and keeps the rollout reversible behind the feature flag.

### 8. Refresh and invalidation
Reuse the GitHub metadata refresh triggers already owned by `LocalGitHubRepoModel`:
- Model creation.
- Current branch change.
- Existing 60-second refresh.
- Explicit pane refresh.
- Completion of relevant `gh` operations.

If the selected layer remains in the refreshed stack:
- Keep the loaded diff when base/head object IDs are unchanged.
- Perform a full layer reload and comment relocation when either object ID changes.

If the selected layer disappears:
- Switch to `Head`.
- Retain its comment batch in the context map.
- Emit a non-blocking notice.

### 9. Add telemetry
Add events for:
- Stack discovery result: stacked, not stacked, unavailable, invalid response.
- Stack map opened.
- Layer selected, including current-branch versus another layer and stack size.
- Layer diff load result and duration.
- Stack review exited to a branch-based diff.

Do not include repository names, branch names, pull request titles, URLs, or comment contents.

## End-to-end flow
1. `LocalGitHubRepoModel` resolves the current branch's pull request.
2. The model queries GitHub's Stacks REST endpoint and enriches the ordered pull requests.
3. `CodeReviewView` receives `StackInfoChanged` and renders the stack control.
4. The user selects a layer.
5. `CodeReviewView` switches the active comment context and applies `DiffMode::PullRequestLayer`.
6. `LocalDiffStateModel` ensures both commits exist locally and loads `base...head` without checkout.
7. `CodeReviewView` renders read-only editors and selected-layer actions.
8. A metadata refresh either preserves the snapshot, reloads changed object IDs, or returns the pane to `Head`.

## Decisions
### Use GitHub-native membership instead of inference
- **Chosen:** GitHub Stacks REST membership.
  - Advantages: authoritative order, explicit stack identity, no false positives, matches GitHub merge semantics.
  - Disadvantages: public-preview API dependency and no Graphite coverage.
- **Rejected for V1:** Infer from pull request base/head chains.
  - Advantages: works without the Stacks API and could cover Graphite.
  - Disadvantages: ambiguous chains, forks and reused branches, no authoritative stack identity, and behavior can disagree with GitHub.

### Use read-only two-revision diffs instead of checkout
- **Chosen:** Fetch Git objects and render `base...head`.
  - Advantages: safe with dirty working trees, fast layer switching after fetch, preserves the user's branch.
  - Disadvantages: requires a new diff source and read-only editor path.
- **Rejected:** Check out each selected branch.
  - Advantages: reuses current `HEAD`-based diff code.
  - Disadvantages: mutates user state, can fail on local changes, changes terminal context, and creates recovery obligations.
- **Rejected:** Use GitHub's files API as the primary renderer.
  - Advantages: no local fetch.
  - Disadvantages: patch truncation, binary and large-file limits, missing complete base content, and a separate renderer from local review.

### Use `gh api` without requiring `gh stack`
- **Chosen:** REST for topology plus one GraphQL enrichment query through `gh api`.
  - Advantages: reuses existing auth, no extension install, exact commit identities, one enrichment round trip.
  - Disadvantages: custom parsing and preview-version handling.
- **Rejected for V1:** Shell out to `gh stack view --json`.
  - Advantages: stack-aware CLI output and local workflow metadata.
  - Disadvantages: optional extension, evolving output, local tracking semantics, and an unnecessary dependency for read-only discovery.

### Keep stack selection ephemeral
- **Chosen:** Rediscover after restore and start in `Head`.
  - Advantages: no migration, no stale remote identity, clean flag rollback.
  - Disadvantages: users must reselect a layer after restart.
- **Deferred:** Persist stack number and selected pull request.
  - Reconsider only if usage shows restore-to-layer is important.


## Risks and mitigations
### GitHub public-preview API changes
Pin the API version in one helper, isolate response types, and gate the entire path. Treat unknown fields as ignorable and missing required fields as an unavailable snapshot.

### Fetching objects from deleted or rewritten branches
Prefer GitHub's pull request head ref, verify expected object IDs after fetch, and fall back to the base branch only when the base object is missing. Never diff an unverified local ref in place of the API object ID.

### Stale async results
Tag GitHub lookups and diff loads with branch, pull request, and generation identity. Drop callbacks that no longer match current selection.

### Comment leakage
Make review context part of the comment model key. Add tests that use the same file and line in two layers and prove only the selected layer's comment is visible.

### Large stacks and repeated metadata calls
Use one REST topology call and one GraphQL enrichment call. Do not spawn one `gh` process per layer. Cache a valid snapshot until a normal refresh trigger invalidates it.

### Historical editors accidentally mutating files
Enforce read-only at both action dispatch and editor construction. Tests must attempt save, discard, and text input while a pull request layer is active.

## Testing and validation
### Automated tests
- `app/src/util/git_tests.rs`
  - Parse valid two-layer and multi-layer REST responses.
  - Enrich and validate bottom-to-top chains.
  - Normalize empty and one-layer results to not stacked.
  - Classify `404`, auth, timeout, malformed JSON, and missing fields.
  - Reject mismatched base/head chains.
- `app/src/code_review/github_repo_model/local_tests.rs`
  - Drop stale results after a branch change.
  - Preserve a valid snapshot on transient refresh failure.
  - Clear stack state when the current branch changes.
- `app/src/code_review/diff_state/local_tests.rs`
  - Load a `base...head` range that excludes working-tree changes.
  - Fetch missing pull request refs into `refs/warp/code-review`.
  - Prove `HEAD`, index, working tree, local branches, and remote-tracking branches are unchanged.
  - Reload only when the selected object IDs change.
- `app/src/code_review/code_review_view_tests.rs`
  - Show and hide the stack control for all discovery states.
  - Render trunk and pull request rows in the required order and state.
  - Ensure rapid selections apply only the last result.
  - Hide mutation actions and expose the selected PR action in layer mode.
  - Return to the preserved working-tree state.
  - Swap comment batches by pull request and never leak same-file comments across layers.
  - Exit safely when a selected layer disappears.
- Stack map view tests
  - Keyboard traversal, activation, close behavior, focus return, long labels, and non-color status indicators.

Run:
- `cargo fmt --check`
- `cargo test -p warp util::git::tests`
- `cargo test -p warp code_review::github_repo_model::local::tests`
- `cargo test -p warp code_review::diff_state::local::tests`
- `cargo test -p warp code_review::code_review_view::tests`
- `cargo check -p warp --lib`

### Manual and visual verification
Use a local test repository with a three-layer GitHub-native stack and the feature flag enabled.

Record a computer-use video that proves:
1. The pane opens in "Uncommitted changes" and later shows the stack control without blocking the first diff.
2. The map shows trunk, three ordered pull requests, state, selected layer, and current branch.
3. Selecting each layer shows only its parent-to-layer files.
4. `git status --porcelain=v2 --branch` and `git rev-parse HEAD` are unchanged after navigation.
5. Text editing, save, discard, commit, push, and create-PR actions are unavailable in layer mode.
6. Comments on the same file in two layers swap independently and return intact.
7. Returning to "Uncommitted changes" restores its diff and comments.
8. A simulated API error preserves normal review and does not show a broken stack surface.
9. Narrow and wide pane layouts remain usable.
10. Keyboard-only navigation completes the map-open, layer-select, and return-to-working-tree flow.

Attach the video to the implementation PR description. A static screenshot is insufficient because the core proof is interaction and lack of working-tree mutation.

## Parallelization
Implementation can use two parallel foundation agents, followed by one integration agent. The lead integrates all work into the existing CODE-1947 spec PR branch and produces one implementation PR.

- **stack-data** — Local agent in `/workspace/warp-worktrees/code-1947-stack-data`, branch `factory/code-1947-stack-data`. Owns `app/src/util/git.rs`, `app/src/code_review/github_repo_model/`, parsing, refresh races, and tests. Returns a local branch for cherry-pick.
- **stack-diff** — Local agent in `/workspace/warp-worktrees/code-1947-stack-diff`, branch `factory/code-1947-stack-diff`. Owns `app/src/code_review/diff_state/`, the commit-range loader, read-only source plumbing below the view, protocol exhaustiveness, and tests. Returns a local branch for cherry-pick.
- **stack-ui-comments** — Starts after the two foundation branches are integrated. Local agent in `/workspace/warp-worktrees/code-1947-stack-ui-comments`, branch `factory/code-1947-stack-ui-comments`. Owns the stack map, `CodeReviewView` integration, contextual comment batches, actions, accessibility, telemetry, and view tests.

The lead resolves shared-type conflicts, runs the complete validation set, and performs computer-use verification after all branches land.

## Follow-ups
- Remote daemon discovery, protocol execution, and SSH UI parity.
- Stack creation and append flows, likely through GitHub REST write endpoints or an explicitly installed `gh stack`.
- Cascading sync/rebase with conflict recovery.
- Asynchronous stack merge and merge-queue integration.
- Cumulative stack-to-trunk diff.
- Graphite or inferred stack adapters behind a provider abstraction.
- Agent stack creation and management coordinated with REMOTE-330.
