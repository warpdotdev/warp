# Multi-team codebase indexing
## Problem
Codebase indices are global to the Warp client, but codebase-indexing policy is effective per team. A user can have several windows on teams with different policies. The client must honor the selected team without duplicating or assigning ownership to global index artifacts.

The current workspace-scoped policy cannot represent this. It can also let one policy decision reset or expose global index state for every window.
## Product model
Codebase indices remain global and unowned. Team policy controls what each window can do with those indices.

A window may use codebase indexing when its selected team's effective policy allows it. This decision is represented by one capability: `can_use_index(team_context)`.

An allowed window can:
- see existing local and remote indices;
- add a repository;
- resync or delete an index;
- expose indexed repositories to the agent;
- retrieve indexed codebase context.

A denied window cannot perform any of those actions. It does not show the index catalog or add controls.
## Automatic indexing
Automatic index creation is conservative across teams:
- If any known team explicitly disables codebase indexing, automatic indexing is disabled in every window.
- The user's auto-indexing preference is preserved while blocked.
- The auto-indexing setting remains visible but disabled, with a tooltip that names the blocking team and directs the user to add repositories explicitly from an allowed-team window.
- If the blocking policy is removed, the preserved preference becomes effective again.

Existing indices are not assigned to a team and are not classified by how they were created. They remain available to every allowed window.
## Background maintenance
Existing indices may continue syncing while at least one known team allows codebase indexing.

If every team denies indexing, the client stops indexing, watcher-driven updates, and retrieval. It keeps index data on disk so it can resume if a team later allows indexing.
## No-team behavior
Users with no teams retain the existing personal behavior. Global AI enablement and the user's codebase-context and auto-indexing preferences determine availability.

A captured team context that no longer resolves is not a no-team context. It must fail closed.
## Product scenarios
### Mixed policies
Team A allows indexing and team B denies it:
- Team A windows can use and manage global indices.
- Team B windows cannot see or use them.
- Automatic indexing is disabled in all windows.
- Team A windows can still add repositories explicitly.

### All teams allow indexing
All windows can use global indices. Automatic indexing follows the user's saved preference.

### All teams deny indexing
No window can see or use indices. Background work stops, but index data remains on disk.

### Policy changes
When a team changes from allowed to denied, its windows immediately lose access. Other allowed windows keep access. Automatic indexing becomes blocked globally.

When the last denying team becomes allowed, automatic indexing resumes if the saved preference is enabled.
## Non-goals
- Team ownership of an index.
- Separate copies of an index for each team.
- A per-index team UID or authorization list.
- Tracking whether an index was created automatically or explicitly.
- A team-scoped auto-indexing preference.
## Acceptance criteria
- Every foreground create, mutate, render, AI-context, and retrieval path checks `can_use_index` with the initiating team context.
- A denied window cannot use an index created by an allowed window.
- One denying team disables automatic index creation for the process.
- Existing indices remain usable by allowed windows.
- Background indexing stops only when no team can use indexing.
- Team removal or policy changes cannot retarget an in-flight operation to another team's policy.
