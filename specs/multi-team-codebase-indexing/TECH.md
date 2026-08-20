# Multi-team codebase indexing technical design
Product requirements: `specs/multi-team-codebase-indexing/PRD.md`.
## Current state
`CodebaseIndexManager` owns one process-global set of local indices, filesystem watchers, snapshots, and retrieval state. `RemoteCodebaseIndexModel` mirrors global remote index state from connected daemons.

Team policy is still read through `UserWorkspaces::is_codebase_context_enabled`, which uses `WorkspaceSettings.codebase_context_settings`. That process-global decision controls startup restoration, automatic indexing, incremental sync, outline generation, `/index`, `/init`, and command availability.

Other paths do not check the legacy decision. AI request context can list every ready local and remote index, and local indexed retrieval can start whenever the global manager contains the requested path.

`PersistedWorkspace::maybe_enable_codebase_indexing` also treats policy as a binary process switch. Its disabled branch calls `reset_codebase_indexing`, which removes in-memory indices, watchers, snapshots, and persisted metadata. That behavior is not valid when one team denies indexing and another allows it.
## Policy API
Use one team-scoped capability for foreground behavior:

`UserWorkspaces::can_use_index(context: Option<&TeamContext>, app: &AppContext) -> bool`

The method combines:
- global AI availability;
- the captured team's effective `TeamSettings.codebase_context.value`;
- the global user codebase-context preference when the team respects it.

Its resolution rules are:
- `None` represents a genuine personal/no-team scope.
- A resolved `TeamContext` uses that team's current effective policy.
- A `TeamContext` whose team no longer exists returns `false`.

Rendering uses the same policy through a borrowed render-only adapter:

`UserWorkspaces::can_use_index_for_render(context: Option<&TeamRenderContext>, app: &AppContext) -> bool`

Do not infer operation scope from an active or default window. The view that starts an operation captures `TeamContext`, and async work retains that context.
## Aggregate process decisions
Add aggregate helpers derived from current team metadata:

- `can_auto_index(app)` is true when the auto-indexing preference is enabled, at least one scope can use indexing, and no known team has an explicit `Disable` policy.
- `can_maintain_indices(app)` is true when at least one known team can use indexing.

For users with no teams, both helpers use the personal/no-team policy.

These helpers are derived state. Do not persist a second team-policy map.
## Global index lifecycle
Indices remain keyed only by repository path. Do not add team identity or creation origin to persisted index metadata.

Replace the destructive policy reset with suspend/resume behavior:
- Suspending aborts active sync work and unregisters watchers.
- Suspending retains in-memory metadata, SQLite metadata, and snapshots.
- Resuming restores watcher and queue activity for existing indices.
- Logout and explicit deletion continue to use destructive removal.

The manager remains global. Foreground access is enforced before callers enter it, while process-level maintenance uses `can_maintain_indices`.
## Foreground scope propagation
Every path that starts or uses an index must carry the initiating team context:
- settings-page add, resync, delete, and remote-index actions;
- `/index`, `/init`, speedbumps, and other terminal index actions;
- local and remote `SearchCodebase` retrieval;
- AI request context assembly;
- automatic AI follow-ups and tool actions derived from an already-scoped request.

The AI request or conversation action state must retain the initiating context so delayed tool calls do not read whichever team the window selects later.

`CodebaseIndexManager` remains unaware of teams. Callers must pass `can_use_index` before create, mutation, status exposure, or retrieval. Remote client operations follow the same rule before sending a daemon request.
## Automatic indexing
Replace legacy workspace checks in local and remote automatic-indexing paths with `can_auto_index`.

This includes:
- startup automatic queueing;
- detected local repository events;
- remote navigation events;
- settings-change re-evaluation.

When one team disables indexing, stop creating new automatic indices. Do not remove or suspend existing indices if another team still allows indexing.
## Background sync
Use `can_maintain_indices` for:
- persisted index restoration;
- filesystem watcher registration and updates;
- conversation-triggered incremental sync;
- remote index status and maintenance activity.

When it becomes false, suspend global indexing without deleting persisted data. When it becomes true, resume existing indices and watchers.
## UI and commands
The code-indexing page uses `can_use_index_for_render` for its selected team:
- denied windows hide the index catalog and add controls;
- allowed windows show and manage the global catalog.

The auto-indexing row remains visible in every team window. When any team explicitly disables indexing, it is disabled and shows the blocking-team tooltip. The saved user preference remains unchanged.

Command Palette and slash-command availability must use the same effective team capability rather than raw `CodeSettings` values.
## Migration
1. Add `can_use_index`, `can_auto_index`, and `can_maintain_indices`.
2. Migrate foreground index creation, mutation, AI context, and retrieval.
3. Replace destructive policy reset with suspend/resume.
4. Migrate local and remote automatic/background paths.
5. Migrate UI and command context flags.
6. Remove `is_codebase_context_enabled`, `team_allows_codebase_context`, and `WorkspaceSettings.codebase_context_settings`.
7. Remove the legacy GraphQL selection and conversion after no production consumer remains.
## Testing
Add coverage for:
- two windows on allowed and denied teams using one global index;
- denied retrieval when an allowed window created the index;
- manual creation from allowed and denied windows;
- one denied team disabling automatic indexing in every window;
- preserving the auto-indexing preference while blocked;
- existing indices continuing to sync when another team allows indexing;
- all teams denying indexing suspending work without deleting persisted data;
- resuming after policy changes;
- no-team behavior;
- an unresolved captured team context failing closed;
- local and remote AI context and retrieval using the same capability.
## Risks
### Missing a foreground entrypoint
A direct manager call can bypass policy. Migrate every production caller and keep policy checks close to user or AI operation boundaries.

### Treating denial as deletion
The current reset path deletes persisted data. Introduce explicit suspend and destructive-delete operations so policy changes cannot erase another allowed window's index.

### Losing scope in asynchronous work
Looking up the active window after an operation starts can retarget authorization. Capture the team context at the initiating view and retain it through async callbacks and AI tool actions.

### Remote client and daemon drift
The client owns team policy. It must stop sending unauthorized requests and ignore index state for denied foreground scopes. Process-level suspend/resume must keep daemon work consistent without using destructive drop operations.
