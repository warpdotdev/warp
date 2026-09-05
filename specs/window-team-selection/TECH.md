# Window Team Selection
## Context
Team metadata belongs to `UserWorkspaces`, while the selected team is window-specific. The current draft stores that selection in an immutable `WindowTeam`, which prevents explicit and metadata-driven reassignment.

- [`app/src/root_view.rs (1697-1737) @ 7c734385`](https://github.com/warpdotdev/warp/blob/7c734385b6e920546eeb325b08847d273bfe6712/app/src/root_view.rs#L1697-L1737) initializes selection from new-window context.
- [`app/src/workspace/window_team.rs (1-60) @ 7c734385`](https://github.com/warpdotdev/warp/blob/7c734385b6e920546eeb325b08847d273bfe6712/app/src/workspace/window_team.rs#L1-L60) implements the current immutable state.
- [`app/src/search/command_palette/warp_drive/data_source.rs (29-144) @ 7c734385`](https://github.com/warpdotdev/warp/blob/7c734385b6e920546eeb325b08847d273bfe6712/app/src/search/command_palette/warp_drive/data_source.rs#L29-L144) maintains a per-window Drive search scope and index.

`Workspace` and its window-scoped children consume the selected team. Pre-Workspace flows only need an initial team UID.
## Proposed changes
- Make `WindowTeam` a non-singleton model owned by `Workspace`, storing `Option<ServerId>`.
- Carry the initial UID through `WorkspaceArgs`; create the model when constructing `Workspace`.
- Subscribe `WindowTeam` directly to `UserWorkspacesEvent::TeamsChanged`. Emit payload-free `WindowTeamEvent::Changed` whenever the UID changes or the selected team's metadata may have changed.
- Reconciliation uses `None` with no teams, selects the sole team, and in multi-team mode preserves a valid selection or falls back to `UserWorkspaces::default_team_uid()`.
- Keep the team switcher behavior: selecting another team opens a new window initialized to that team rather than reassigning the current window.
- Keep team metadata in `UserWorkspaces`, but remove its window-to-team map; resolve the model's selected UID to current metadata and effective team settings on demand.
- Let views resolve `WindowTeam` through their current RootView, which delegates to its current Workspace.
- Inject a cloned `ModelHandle<WindowTeam>` into window-scoped models such as Drive search. Consumers re-resolve derived state on `WindowTeamEvent::Changed` instead of separately observing team metadata.
- Multi-window models resolve the relevant `WindowTeam` per view, session, or operation rather than retaining one window's handle.
- Give inherited, restored, and detached windows independent models initialized from the source selection; never share one mutable model across windows.
- Use `UserWorkspaces::default_team_uid()` in pre-Workspace flows. Destroy the model with Workspace on logout and persist its current selection in window snapshots.
## Testing and validation
- Unit-test reconciliation and change-event emission, including metadata changes that preserve the selected UID.
- Cover new, team-selected, inherited, restored, detached, logged-out, and metadata-reassigned windows.
- Verify Drive search rebuilds after selection changes and remains independent across windows.
## Parallelization
Parallel implementation is not proposed because Workspace ownership, reconciliation, and Drive search subscriptions change one tightly coupled state flow.
