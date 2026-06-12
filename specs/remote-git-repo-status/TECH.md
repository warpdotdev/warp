# TECH: Remote-capable git status + GitHub info
## Context
Tabs and prompt/footer chips read git status and GitHub info from two per-repo models vended by the `GitRepoModels` singleton factory:
- `GitRepoStatusModel` (`app/src/code_review/git_repo_model/`) — cheap, filesystem-watcher-driven status: `current_branch_name`, `main_branch_name`, `stats_against_head`. Event: `GitRepoStatusEvent::MetadataChanged`.
- `GitHubRepoModel` (`app/src/code_review/github_repo_model/`) — expensive, `gh`-CLI-driven `pr_info` (`gh pr view`) plus `repository_info` (`gh repo view`), refreshed on creation, on branch changes for PR info, and on a 60s timer. Event: `GitHubRepoEvent::{PrInfoChanged, RepositoryInfoChanged}`.
The local backend owns filesystem watchers and runs local `git`/`gh` commands. Remote sessions cannot run those commands locally, so the remote backend acts as a push receiver keyed by `(host_id, repo_path)` while the daemon runs the local models on the remote host.
Key code:
- `app/src/code_review/git_repo_model/` — unified `GitRepoStatusModel`, `LocalGitRepoStatusModel`, `RemoteGitRepoStatusModel`, factory cache keyed by `LocalOrRemotePath`.
- `app/src/code_review/github_repo_model/` — unified `GitHubRepoModel`, `LocalGitHubRepoModel`, `RemoteGitHubRepoModel`.
- `app/src/remote_server/server_model.rs` — daemon-side model cache, git-status notification handler, GitHub host-scoped handlers, and broadcasts.
- `crates/remote_server/proto/diff_state.proto` — `GitStatusMetadata`, `GitStatusPush`, `UpdateGitStatus`, `GitHubPrInfoPush`, `GitHubRepositoryInfoPush`, `GetGitHubPrInfoRequest` / `GetGitHubPrInfoResponse`, `GetGitHubRepositoryInfoRequest` / `GetGitHubRepoInfoResponse`.
- `crates/remote_server/proto/remote_server.proto` — `UpdateGitStatus` as a `Notification`, GitHub refreshes as `HostScopedRequest` variants, and the matching push / response `ServerMessage` variants.
- `crates/remote_server/src/manager.rs` and `crates/remote_server/src/client/mod.rs` — manager events, the fire-and-forget git-status notification helper, and the host-scoped GitHub refresh helpers.
## Proposed changes
### 1. Unified local/remote models
Each model is a unified enum that forwards events and preserves the existing read API:
- `GitRepoStatusModel = enum { Local(LocalGitRepoStatusModel), Remote(RemoteGitRepoStatusModel) }` exposes `metadata()` and `refresh_metadata()`.
- `GitHubRepoModel = enum { Local(LocalGitHubRepoModel), Remote(RemoteGitHubRepoModel) }` exposes `pr_info()`, `repository_info()`, `is_refreshing_pr_info()`, `refresh_pr_info()`, and `refresh_repository_info()`.
`GitRepoModels` caches both model types by `LocalOrRemotePath`, so prompt chips, code review, tabs, and agent context all hold the same `ModelHandle<GitRepoStatusModel>` / `ModelHandle<GitHubRepoModel>` regardless of backend. Remote receivers preserve stale data across disconnects and update only from pushes or responses matching their `(host_id, repo_path)`.
### 2. Transport shape: git-status notification, GitHub host-scoped refreshes
Git status is shared per-repo model state and is small enough to remain a notification-triggered push stream:
- `UpdateGitStatus { repo_path }` is a `Notification`. It asks the daemon to subscribe/create the per-repo git-status model and push the current `GitStatusPush` snapshot when metadata is available. It has no request id, no response, and is best-effort.
- `RemoteGitRepoStatusModel` sends that notification on construction and again on `HostConnected`; live watcher changes arrive later as `GitStatusPush` messages filtered by `(host_id, repo_path)`.
GitHub info is also shared cached state, but explicit refreshes use host-scoped request/response so the client can track request completion and surface PR-refresh loading state:
- `GetGitHubPrInfoRequest { repo_path }` is a `HostScopedRequest`. The daemon validates the path, subscribes/creates the per-repo `GitHubRepoModel` (whose own lifecycle drives `GitHubPrInfoPush` broadcasts), runs `git::get_pr_for_branch` for the requesting client, and returns `GetGitHubPrInfoResponse`. It does not separately trigger `refresh_pr_info`, which would run a redundant `gh pr view`.
- `GetGitHubRepositoryInfoRequest { repo_path }` is a `HostScopedRequest`. The daemon validates the path, subscribes/creates the same `GitHubRepoModel`, runs `git::get_repository_info` for the requesting client, and returns `GetGitHubRepoInfoResponse`. It does not separately trigger `refresh_repository_info`.
- The daemon-side `GitHubRepoModel` broadcasts `GitHubPrInfoPush { repo_path, pr_info }` on PR changes and `GitHubRepositoryInfoPush { repo_path, repository_info }` on repository-info changes, so other consumers and later updates use shared push streams even though explicit refreshes also get direct responses.
### 3. Why GitHub PR and repository refreshes are split
The cached model read surface remains unified because consumers want a single `GitHubRepoModel`, but the client-to-daemon refreshes and pushes are split because PR info is requested more often:
- PR refreshes happen after `gh`/`gt` commands, on branch changes, on reconnect, and from explicit code-review refreshes; those should only run `gh pr view`.
- Repository info is branch-independent and should only run `gh repo view` on initial activation, reconnect, explicit repository-info refresh, or the periodic local timer.
Splitting the host-scoped requests and push messages avoids turning frequent PR refreshes into unnecessary repository-info refreshes or combined payload updates. Creating the daemon-side `LocalGitHubRepoModel` still kicks off its normal initial/periodic lifecycle, which drives the shared broadcasts; repeated PR requests run only `gh pr view` (via `git::get_pr_for_branch`) and return a PR-specific response to the requesting client.
### 4. Server-side broadcast model
`ServerModel` keeps two daemon-side caches keyed by `StandardizedPath`:
- `git_status_models: HashMap<StandardizedPath, ModelHandle<GitRepoStatusModel>>`
- `github_repo_models: HashMap<StandardizedPath, ModelHandle<GitHubRepoModel>>`
On first subscription, `ServerModel` wires model events to `send_server_message(None, None, …)`, broadcasting pushes to all connections:
- `GitRepoStatusEvent::MetadataChanged` broadcasts `GitStatusPush { repo_path, metadata }`.
- `GitHubRepoEvent::PrInfoChanged` broadcasts `GitHubPrInfoPush { repo_path, pr_info }`.
- `GitHubRepoEvent::RepositoryInfoChanged` broadcasts `GitHubRepositoryInfoPush { repo_path, repository_info }`.
Host-scoped GitHub refresh requests additionally return `GetGitHubPrInfoResponse` or `GetGitHubRepoInfoResponse` to the originating request id. The response path updates the requesting remote model's loading state; the push path keeps the shared cache and sibling consumers up to date.
Navigation still acts as an opportunistic git-status interest signal: after `NavigatedToDirectory` resolves a git root, the daemon subscribes to the git-status model and pushes the current snapshot if available. `RemoteGitRepoStatusModel` also sends `UpdateGitStatus` after subscribing and again on `HostConnected`, covering the race where navigation's opportunistic push arrives before the receiver exists.
### 5. Client-side receivers and matching
`RemoteServerClient` parses `GitStatusPush`, `GitHubPrInfoPush`, and `GitHubRepositoryInfoPush` into client events, and `RemoteServerManager` attaches the resolved `HostId` before emitting manager events. GitHub host-scoped helpers also emit response events keyed by `(host_id, repo_path)`. Remote models filter on both host and repo:
- `RemoteGitRepoStatusModel` accepts only `GitStatusPushReceived { host_id, repo_path, metadata }` matching its `RemotePath`.
- `RemoteGitHubRepoModel` accepts matching `GitHubPrInfoPushReceived`, `GitHubRepositoryInfoPushReceived`, `GetGitHubPrInfoResponse`, and `GetGitHubRepoInfoResponse` events. Pushes update cached fields and emit only when values move; while a field-specific request is in flight, an empty push value does not clear the requested field before the direct response arrives.
This keeps git-status notifications fire-and-forget while preserving host/repo correctness. GitHub refreshes use host-scoped request/response for explicit completion semantics, and the split GitHub push messages carry the durable shared state and subsequent model updates.
### 6. Wire protocol summary
`crates/remote_server/proto/diff_state.proto` owns the payloads:
- `GitStatusMetadata { current_branch_name, main_branch_name, DiffStats stats_against_head }`
- `GitStatusPush { repo_path, metadata }`
- `UpdateGitStatus { repo_path }`
- `GitHubPrInfoPush { repo_path, optional PrInfo pr_info }`
- `GitHubRepositoryInfoPush { repo_path, optional RepositoryInfo repository_info }`
- `GetGitHubPrInfoRequest { repo_path }`
- `GetGitHubPrInfoResponse { optional PrInfo pr_info, GitOpError error }`
- `GetGitHubRepositoryInfoRequest { repo_path }`
- `GetGitHubRepoInfoResponse { optional RepositoryInfo repository_info, GitOpError error }`
`crates/remote_server/proto/remote_server.proto` exposes `UpdateGitStatus` as a `Notification`, exposes `GetGitHubPrInfoRequest` and `GetGitHubRepositoryInfoRequest` as `HostScopedRequest` variants, and exposes `GitStatusPush`, `GitHubPrInfoPush`, `GitHubRepositoryInfoPush`, `GetGitHubPrInfoResponse`, and `GetGitHubRepoInfoResponse` as `ServerMessage` variants. There is no combined `GetGitHubInfo` request/response or push in the current protocol.
### 7. Consumer wiring
- `update_git_status_subscription` passes the current `LocalOrRemotePath` to `GitRepoModels::subscribe` and wires the unified status handle into prompt chips, tab metadata, code review refreshes, and the branch source needed by GitHub/agent-context subscriptions.
- `sync_pr_info_subscription` passes the current `LocalOrRemotePath` to `subscribe_github_repo` when prompt/footer chips or AI context need PR/repository info. It wires the unified GitHub handle into prompt PR chips and the AI context model.
- `CodeReviewView` subscribes to both per-repo models: git-status events refresh git-operation UI, while GitHub PR events drive PR-aware actions (`View PR`, `Create PR`, commit-and-create-PR eligibility) through the unified `GitHubRepoModel` instead of a local/remote split.
- After `gh`/`gt` commands and git-dialog completion, callers refresh PR info through the unified `GitHubRepoModel`; local backends run `gh pr view` directly, while remote backends send `GetGitHubPrInfoRequest`.
- AI-context repository and PR gates use `current_repo_path` plus the unified GitHub handle so both local and remote repos can provide repository and PR context when data is available.
