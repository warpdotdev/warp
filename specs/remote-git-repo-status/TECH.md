# TECH: Remote-capable git status + GitHub info
## Context
Tabs and prompt/footer chips read git status and GitHub info from two per-repo models vended by the `GitRepoModels` singleton factory:
- `GitRepoStatusModel` (`app/src/code_review/git_repo_model/`) — cheap, filesystem-watcher-driven status: `current_branch_name`, `main_branch_name`, `stats_against_head`. Event: `GitRepoStatusEvent::MetadataChanged`.
- `GitHubRepoModel` (`app/src/code_review/github_repo_model/`) — expensive, `gh`-CLI-driven `pr_info` (`gh pr view`) plus `repository_info` (`gh repo view`), refreshed on creation, on branch changes for PR info, and on a 60s timer. Event: `GitHubRepoEvent::{PrInfoChanged, RepositoryInfoChanged}`.
The local backend owns filesystem watchers and runs local `git`/`gh` commands. Remote sessions cannot run those commands locally, so the remote backend acts as a push receiver keyed by `(host_id, repo_path)` while the daemon runs the local models on the remote host.
Key code:
- `app/src/code_review/git_repo_model/` — unified `GitRepoStatusModel`, `LocalGitRepoStatusModel`, `RemoteGitRepoStatusModel`, factory cache keyed by `LocalOrRemotePath`.
- `app/src/code_review/github_repo_model/` — unified `GitHubRepoModel`, `LocalGitHubRepoModel`, `RemoteGitHubRepoModel`.
- `app/src/remote_server/server_model.rs` — daemon-side model cache, git-status and GitHub-info notification handlers, and broadcasts.
- `crates/remote_server/proto/diff_state.proto` — `GitStatusMetadata`, `GitStatusPush`, `UpdateGitStatus`, `GitHubPrInfoPush`, `GitHubRepositoryInfoPush`, `UpdateGitHubPrInfo`, `UpdateGitHubRepoInfo`.
- `crates/remote_server/proto/remote_server.proto` — `UpdateGitStatus`, `UpdateGitHubPrInfo`, and `UpdateGitHubRepoInfo` as `Notification` variants, and the `GitStatusPush` / `GitHubPrInfoPush` / `GitHubRepositoryInfoPush` `ServerMessage` push variants.
- `crates/remote_server/src/manager.rs` and `crates/remote_server/src/client/mod.rs` — manager events and the fire-and-forget git-status and GitHub-info notification helpers.
## Proposed changes
### 1. Unified local/remote models
Each model is a unified enum that forwards events and preserves the existing read API:
- `GitRepoStatusModel = enum { Local(LocalGitRepoStatusModel), Remote(RemoteGitRepoStatusModel) }` exposes `metadata()` and `refresh_metadata()`.
- `GitHubRepoModel = enum { Local(LocalGitHubRepoModel), Remote(RemoteGitHubRepoModel) }` exposes `pr_info()`, `repository_info()`, `is_refreshing_pr_info()`, `refresh_pr_info()`, and `refresh_repository_info()`.
`GitRepoModels` caches both model types by `LocalOrRemotePath`, so prompt chips, code review, tabs, and agent context all hold the same `ModelHandle<GitRepoStatusModel>` / `ModelHandle<GitHubRepoModel>` regardless of backend. Remote receivers preserve stale data across disconnects and update only from pushes matching their `(host_id, repo_path)`.
### 2. Transport shape: notifications in, pushes out
Both git status and GitHub info are shared per-repo model state, small enough to use notification-triggered push streams with no request/response:
- `UpdateGitStatus { repo_path }` is a `Notification`. It asks the daemon to subscribe/create the per-repo git-status model and push the current `GitStatusPush` snapshot when metadata is available. It has no request id, no response, and is best-effort.
- `RemoteGitRepoStatusModel` sends that notification on construction and again on `HostConnected`; live watcher changes arrive later as `GitStatusPush` messages filtered by `(host_id, repo_path)`.
- `UpdateGitHubPrInfo { repo_path }` and `UpdateGitHubRepoInfo { repo_path }` are `Notification`s. Each asks the daemon to subscribe/create the per-repo `GitHubRepoModel` and refresh the relevant info. The daemon model is the single source of truth: its `PrInfoChanged` / `RepositoryInfoChanged` events broadcast `GitHubPrInfoPush` / `GitHubRepositoryInfoPush` to all connections. There is no request/response, so the daemon never runs a separate per-request `gh` fetch that could diverge from the cached value it broadcasts.
- `RemoteGitHubRepoModel` sends both notifications on construction and again on `HostConnected`; results arrive as the push messages filtered by `(host_id, repo_path)`.
### 3. Why GitHub PR and repository refreshes are split
The cached model read surface remains unified because consumers want a single `GitHubRepoModel`, but the client-to-daemon refresh notifications and pushes are split because PR info is requested more often:
- PR refreshes happen after `gh`/`gt` commands, on branch changes, on reconnect, and from explicit code-review refreshes; those should only run `gh pr view`.
- Repository info is branch-independent and should only run `gh repo view` on initial activation, reconnect, explicit repository-info refresh, or the periodic local timer.
Splitting the notifications and push messages avoids turning frequent PR refreshes into unnecessary repository-info refreshes or combined payload updates. The daemon handler creates the `LocalGitHubRepoModel` if needed (whose creation kicks off the initial fetch) and otherwise forces a `refresh_pr_info` / `refresh_repository_info` on the existing model; either way the model's own lifecycle drives the shared broadcasts.
### 4. Server-side broadcast model
`ServerModel` keeps two daemon-side caches keyed by `StandardizedPath`:
- `git_status_models: HashMap<StandardizedPath, ModelHandle<GitRepoStatusModel>>`
- `github_repo_models: HashMap<StandardizedPath, ModelHandle<GitHubRepoModel>>`
On first subscription, `ServerModel` wires model events to `send_server_message(None, None, …)`, broadcasting pushes to all connections:
- `GitRepoStatusEvent::MetadataChanged` broadcasts `GitStatusPush { repo_path, metadata }`.
- `GitHubRepoEvent::PrInfoChanged` broadcasts `GitHubPrInfoPush { repo_path, pr_info }`.
- `GitHubRepoEvent::RepositoryInfoChanged` broadcasts `GitHubRepositoryInfoPush { repo_path, repository_info }`.
`UpdateGitHubPrInfo` / `UpdateGitHubRepoInfo` notifications trigger the daemon model to (create and) refresh; the resulting `PrInfoChanged` / `RepositoryInfoChanged` events drive the broadcasts above. There is no per-request response, so the broadcast push is the single source of truth for every connection.
Navigation still acts as an opportunistic git-status interest signal: after `NavigatedToDirectory` resolves a git root, the daemon subscribes to the git-status model and pushes the current snapshot if available. `RemoteGitRepoStatusModel` also sends `UpdateGitStatus` after subscribing and again on `HostConnected`, covering the race where navigation's opportunistic push arrives before the receiver exists.
### 5. Client-side receivers and matching
`RemoteServerClient` parses `GitStatusPush`, `GitHubPrInfoPush`, and `GitHubRepositoryInfoPush` into client events, and `RemoteServerManager` attaches the resolved `HostId` before emitting manager events. Remote models filter on both host and repo:
- `RemoteGitRepoStatusModel` accepts only `GitStatusPushReceived { host_id, repo_path, metadata }` matching its `RemotePath`.
- `RemoteGitHubRepoModel` accepts matching `GitHubPrInfoPushReceived` and `GitHubRepositoryInfoPushReceived` events, updating cached fields and emitting only when values move. It tracks no refresh state — `is_refreshing_pr_info()` is always `false` for remote repos.
This keeps git status and GitHub info fire-and-forget while preserving host/repo correctness; the push messages carry the durable shared state and every subsequent model update.
### 6. Wire protocol summary
`crates/remote_server/proto/diff_state.proto` owns the payloads:
- `GitStatusMetadata { current_branch_name, main_branch_name, DiffStats stats_against_head }`
- `GitStatusPush { repo_path, metadata }`
- `UpdateGitStatus { repo_path }`
- `GitHubPrInfoPush { repo_path, optional PrInfo pr_info }`
- `GitHubRepositoryInfoPush { repo_path, optional RepositoryInfo repository_info }`
- `UpdateGitHubPrInfo { repo_path }`
- `UpdateGitHubRepoInfo { repo_path }`
`crates/remote_server/proto/remote_server.proto` exposes `UpdateGitStatus`, `UpdateGitHubPrInfo`, and `UpdateGitHubRepoInfo` as `Notification` variants, and exposes `GitStatusPush`, `GitHubPrInfoPush`, and `GitHubRepositoryInfoPush` as `ServerMessage` push variants. GitHub info is push-only: there is no request/response and no combined `GetGitHubInfo` message in the protocol.
### 7. Consumer wiring
- `update_git_status_subscription` passes the current `LocalOrRemotePath` to `GitRepoModels::subscribe` and wires the unified status handle into prompt chips, tab metadata, code review refreshes, and the branch source needed by GitHub/agent-context subscriptions.
- `sync_pr_info_subscription` passes the current `LocalOrRemotePath` to `subscribe_github_repo` when prompt/footer chips or AI context need PR/repository info. It wires the unified GitHub handle into prompt PR chips and the AI context model.
- `CodeReviewView` subscribes to both per-repo models: git-status events refresh git-operation UI, while GitHub PR events drive PR-aware actions (`View PR`, `Create PR`, commit-and-create-PR eligibility) through the unified `GitHubRepoModel` instead of a local/remote split.
- After `gh`/`gt` commands and git-dialog completion, callers refresh PR info through the unified `GitHubRepoModel`; local backends run `gh pr view` directly, while remote backends send an `UpdateGitHubPrInfo` notification and await the resulting push.
- AI-context repository and PR gates use `current_repo_path` plus the unified GitHub handle so both local and remote repos can provide repository and PR context when data is available.
