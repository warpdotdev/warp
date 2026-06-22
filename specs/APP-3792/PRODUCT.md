# APP-3792: Remote Codebase Indexing

Linear: [APP-3792](https://linear.app/warpdotdev/issue/APP-3792)

## Summary
Remote codebase indexing lets Warp agents in SSH-backed remote sessions use semantic codebase search against repositories that live on the remote host. Users should be able to enable indexing, understand whether it is enabled and healthy, and receive the same `SearchCodebase` quality they get for local repositories without extra setup.

## Figma
Figma: none provided. The user-visible surface is the existing codebase-indexing speedbump and settings page, extended to distinguish remote repositories and expose remote indexing status.

## Problem
Today, codebase indexing is local-only: the filesystem walk, tree build, chunking, sync, persistence, watcher, and retrieval state all assume files are on the client machine. In remote sessions, agents can read files and apply file edits after the remote-file-tooling work, but semantic codebase search is not available for the remote repository the user is actually working in.

## Goals
- Make `SearchCodebase` available in remote sessions once the remote repository has a ready index.
- Show users whether remote codebase indexing is enabled, in progress, ready, stale, failed, disabled, or unavailable.
- Reuse the local codebase-indexing product model where possible so local and remote repositories feel like one feature.
- Scope user decisions, status, and backend retrieval authorization per Warp user, remote host, and repository while allowing the machine-local serialized index cache to be reused when it contains no user-specific data.

## Non-goals
- Sharing user-specific enablement, status, decline/drop decisions, or backend retrieval authorization across different Warp users on the same host.
- Making remote indexing work without any daemon-to-Warp-backend egress. If that network path is blocked, the product should fail visibly and recoverably.
- Changing local codebase-indexing behavior.
- Exposing implementation identifiers such as root hashes in the UI.

## Behavior
### Enablement and discovery
1. When a user is in a connected remote session and navigates to a git repository on the remote host, Warp determines whether codebase indexing has been enabled for that `(Warp user, host, repo)` tuple and whether a reusable machine-local serialized index cache exists for the repo.

2. If the user has already enabled indexing for that tuple and a ready cached index exists, Warp treats the repo as index-enabled immediately. The user does not see a first-run speedbump and the agent can use `SearchCodebase` as soon as the client has received the ready status.

3. If no cached index exists and remote automatic indexing is enabled, Warp starts indexing the repo without interrupting the user, matching local automatic indexing behavior.

4. If no cached index exists and remote automatic indexing is not enabled, Warp shows the existing codebase-indexing speedbump in the remote session. The speedbump clearly indicates that the repository is remote, for example with a `Remote` tag, host label, or equivalent visual treatment.

5. Accepting the speedbump starts indexing for that remote repo. Declining dismisses indexing for that repo only. A global decline disables automatic remote indexing but does not change local automatic indexing.

6. Declining or dropping one remote repo does not affect other repos on the same host, the same repo path on a different host, or local repos.

7. If the remote-server connection is unavailable, not authenticated, or not running a build that supports remote indexing, Warp does not offer remote indexing for that session. Other remote agent tools continue to work normally.

### Status visibility
8. The codebase-indexing settings page lists remote repositories alongside local repositories. Each remote entry includes enough context to identify it: at minimum repo path and host; if multiple remote identities can point at the same host, the UI must still make the entries distinguishable.

9. Remote entries use the same overall visual language as local indexing entries, with an additional remote indicator. The minimum acceptable indicator is a visible `Remote` tag; showing host information is preferred when space allows.

10. Each remote repo exposes one current status:
    - **Not enabled** — indexing has not been accepted or started for this repo.
    - **Queued** — Warp accepted the indexing request but the daemon has not started the repo build yet.
    - **Indexing** — the daemon is building the tree, chunking files, embedding fragments, or syncing with the backend. Progress is shown when known.
    - **Ready** — indexing has completed and `SearchCodebase` can retrieve results for this repo.
    - **Stale** — a previous index is ready, but the remote filesystem has changed and a newer index is being synced. Search remains available against the last ready index.
    - **Failed** — indexing or sync failed. The UI shows a user-readable reason and a retry affordance.
    - **Disabled** — the user disabled indexing for this repo.
    - **Unavailable** — the repo has known status, but the remote host or daemon is currently disconnected.

11. In-progress states should communicate what Warp is doing when that is known, such as discovering files, syncing changed files, embedding fragments, or waiting to retry after a recoverable backend error.

12. Status updates should appear without requiring the user to refresh settings or reopen the tab. A user watching settings while indexing runs should see transitions from queued/indexing to ready or failed.

13. Failed states include retry. Retrying starts the remote indexing flow again for the same repo and updates the status as new progress arrives.

14. Dropping a remote repo from settings removes that user's cached indexing state for the repo and stops future syncing for that user until they re-enable indexing. The machine-local serialized index cache may remain available for other users or future reuse.

### Agent retrieval
15. In a remote session, `SearchCodebase` is advertised to the agent only when remote codebase indexing is enabled for the active repo and Warp has a ready searchable index.

16. When `SearchCodebase` runs for a ready remote repo, results refer to files and ranges on the remote host. The agent receives the same high-level result shape it receives for local search, including file paths and relevant fragments.

17. If the index is queued or indexing, `SearchCodebase` returns a clear "indexing is still in progress" failure rather than partial or silently empty results.

18. If the index failed, `SearchCodebase` returns the failure reason so the agent can explain the issue or fall back to tools like `Grep`, `FileGlob`, and `ReadFiles`.

19. If the repo is stale because a sync is in progress after filesystem changes, `SearchCodebase` continues using the last ready index until the new one becomes ready.

20. Remote `SearchCodebase` should feel comparable to local search. The remote architecture should avoid adding an SSH round trip to the main retrieval query when the client already has enough status to query the backend directly.

### Persistence, startup, and incremental changes
21. Once a remote repo has been indexed, per-user status metadata and the machine-local serialized index cache persist across SSH disconnects, tab closes, daemon grace-period survival, and daemon restarts when the daemon's on-disk cache remains available.

22. On startup or reconnect, Warp bootstraps known remote repo statuses from the remote side. Repos that the user already enabled and that have a valid machine-local cached index should become usable without rebuilding from scratch.

23. If the remote filesystem changed while disconnected, Warp detects that after reconnect and syncs incrementally. The status becomes stale or indexing while the sync runs, then ready when the new index is available.

24. If the daemon's on-disk cache is missing or corrupted, Warp rebuilds the index from scratch the next time indexing is enabled for that repo. The UI should make that look like a normal indexing run, not a permanent failure.

25. Remote indexing respects server-backed codebase-indexing configuration such as sync cadence, batch sizes, and embedding configuration. Users do not need to configure those values locally on the remote host.

### Per-user and security invariants
26. Remote indexing enablement, status, decline/drop decisions, and backend retrieval authorization are scoped to the authenticated Warp user that owns the daemon. Two Warp users connecting to the same OS account and repo path may reuse the same machine-local serialized Merkle/snapshot cache when OS permissions allow, but one user's choices or backend access do not enable search for another user.

27. Indexing respects the filesystem permissions of the OS user running the remote daemon. If the daemon cannot read a file, that file is not indexed.

28. The remote daemon uses its authenticated Warp credential only to call Warp services needed for indexing and sync. The credential is never displayed to the user, sent to the agent, or included in agent conversation context.
29. Any remote client <> remote server proto message that can cause the daemon to make auth-required outbound Warp service requests must include the client's current auth token or request-scoped bearer credential. The daemon must reject those requests when the token is missing or invalid instead of treating the daemon's stored token as sufficient, so a process writing directly to the proxy socket cannot bypass authentication.

30. Remote indexing does not change `ReadFiles`, `ApplyFileDiffs`, shell execution, or other remote agent tools. Those tools remain available regardless of whether remote indexing is enabled.

### Backend reachability and firewall behavior
31. The v1 product assumes the remote daemon can reach `app.warp.dev`; that assumption has been checked with the initial target enterprise environments.

32. If the remote daemon cannot reach `app.warp.dev`, remote indexing fails with a user-readable error such as "Warp could not reach the backend from this remote host." The user can retry after fixing network access.

33. A backend-unreachable repo is not searchable. Warp should not pretend the feature is enabled if sync cannot complete.

### Local behavior must not regress
34. Existing local codebase-indexing speedbumps, settings, indexing status, and retrieval behavior are unchanged.

35. Existing local settings continue to apply to local repos. Remote auto-indexing may have its own setting, but changing it does not unexpectedly toggle local indexing.

36. If the remote-indexing feature flag is disabled, remote sessions behave as they do today: no remote `SearchCodebase`, no remote indexing speedbump, and no user-visible errors from the disabled feature.
