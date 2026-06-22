# Touched-Workspace Discovery — Tech Spec
Part of the local-to-cloud Oz handoff feature ([REMOTE-1486](https://linear.app/warpdotdev/issue/REMOTE-1486)). Full feature behavior in `PRODUCT.md`; the orchestrator and UI that consume this module live in `TECH.md`.
## Context
The handoff flow needs two pieces of derived state from the local conversation that the existing cloud-agent infra doesn't already produce: a list of git repos and orphan files the local agent has touched (consumed by the snapshot pipeline in the sibling branch), and a repo-aware default env pick for the new cloud-mode pane's env selector. Both derivations are pure / async and have no UI of their own — they're a library that the parent stack branch wires up.
This branch contains only the library; nothing in-tree calls it yet, so the module is gated with `#![allow(dead_code)]` and the `sort_environments_by_recency` visibility bump is removed by the parent branch when the consumers land.
Relevant code:
- `app/src/ai/agent/conversation.rs` — `AIConversation::all_exchanges` and the per-exchange `working_directory` we walk.
- `app/src/ai/agent/mod.rs` — `AIAgentAction` and `AIAgentActionType` variants we filter for write actions.
- `app/src/ai/cloud_environments/mod.rs` — `CloudAmbientAgentEnvironment` and its `github_repos: Vec<GithubRepo>` field used for env-overlap matching.
- `app/src/ai/blocklist/agent_view/agent_input_footer/environment_selector.rs` — `sort_environments_by_recency`, the existing recency-sort helper we now share with the env-overlap pick.
## Proposed changes
All new code lives in `app/src/ai/blocklist/handoff/touched_repos.rs`.
### Path extraction
```rust path=null start=null
pub(crate) fn extract_paths_from_conversation(conversation: &AIConversation) -> Vec<PathBuf>;
```
Walks the conversation's exchanges newest-first, capped at `MAX_TOOL_CALLS_TO_SCAN = 500` action results. From each exchange we collect:
- Every file path the agent **wrote to** via `RequestFileEdits` or `UploadArtifact`.
- The per-exchange `working_directory`, so repos the agent only browsed via shell commands are still discovered.
Read-only actions (`ReadFiles`, `Grep`, `FileGlob*`, `SearchCodebase`, `InsertCodeReviewComments`) are intentionally **not** walked: the handoff snapshot uploads orphan-file contents verbatim, so including read-only paths would let the agent leak something like `~/.ssh/id_rsa` into the cloud sandbox. Limiting the walk to writes keeps the snapshot to files the user knowingly let the agent author.
Relative paths are resolved against the exchange's `working_directory`; paths with no resolvable cwd are dropped, as are empty entries.
### Workspace derivation
```rust path=null start=null
pub(crate) async fn derive_touched_workspace(paths: Vec<PathBuf>) -> TouchedWorkspace;
pub(crate) struct TouchedWorkspace {
    pub repos: Vec<TouchedRepo>,
    pub orphan_files: Vec<PathBuf>,
}
pub(crate) struct TouchedRepo {
    pub git_root: PathBuf,
    pub repo_id: Option<GithubRepo>,
}
```
Walks each input path up to the nearest `.git` directory: paths with a `.git` ancestor go into a deduped set of git roots; paths without one are kept as orphan files (filtered to ones that exist and are regular files). For each unique git root, `git remote get-url origin` runs via `command::r#async::Command` (no per-call OS thread) with a 5-second timeout. The trimmed remote URL is parsed by `parse_github_repo` into `<owner>/<repo>` for env-overlap matching; non-GitHub remotes leave `repo_id = None`.
Per-repo `branch` / `head_sha` metadata is **not** gathered here — the existing `repo_metadata` helper in the snapshot pipeline (sibling branch) does that during upload, keeping the rehydration prompt's plumbing unchanged.
### Env-overlap pick
```rust path=null start=null
pub(crate) fn pick_handoff_overlap_env(
    workspace: &TouchedWorkspace,
    envs: Vec<CloudAmbientAgentEnvironment>,
) -> Option<SyncId>;
```
Scores each env by the number of touched repos it contains (against the env's `github_repos`), picks the highest-scoring env, breaks ties by recency. Returns `None` when no env contains any touched repo so callers leave the existing env-selector default in place.
Sorts `envs` internally via `sort_environments_by_recency` (the same helper the env selector uses) so ties resolve to the most-recently-used env. That helper is bumped from `fn` to `pub(crate) fn` and re-exported from `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` so this module can call it.
## Testing and validation
- `touched_repos_tests.rs` covers `find_git_root` against a temporary directory layout (file inside a repo, directory inside a repo, path outside any repo). `find_git_root` is the only helper that walks the real filesystem; covering it directly avoids fixturing `git` subprocess behavior.
- `parse_github_repo` and `pick_handoff_overlap_env` are pure helpers exercised end-to-end by the handoff submit path on the parent stack branch — their correctness is enforced by their call sites there rather than by standalone tests in this branch.
