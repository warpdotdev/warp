# REV-37: Preserve one team identity across asynchronous work

Jira: https://warp-dev-staging.atlassian.net/browse/REV-37

## Summary
Warp must capture one team identity for each logical team-scoped operation. The operation must use that identity for all asynchronous work, callbacks, retries, events, child work, and requests.

## Problem
The Rust client can read a team from a window, wait for asynchronous work, and then read the team again. The two reads can return different teams. The result can then be sent or applied under the wrong team.

The same risk exists in long-lived conversations and singleton models. These objects can outlive the window state that started their work.

## Behavior
1. A team-scoped operation captures its team once from an approved source.
   - A GUI operation captures the team from its source view or window.
   - A restored team conversation captures the team from its server owner metadata.
   - A child operation inherits the parent operation's team.
   - A CLI operation captures an explicitly selected team when more than one team is available.

2. The captured team is an immutable snapshot.
   - A later window team change does not modify the snapshot.
   - A later workspace change does not modify the snapshot.
   - Metadata refresh and window reconciliation do not modify the snapshot.

3. Every stage of one logical operation uses the same snapshot.
   - The background future uses the snapshot.
   - The completion callback receives the snapshot with the result.
   - A retry uses the snapshot from the first attempt.
   - An event or channel message carries the snapshot with its value.
   - A team-scoped server request uses the snapshot.

4. Code must not select a team again after the first capture.
   - It must not pass a `WindowId` or `ViewId` to select the team later.
   - It must not pass a raw team UID between normal operation layers.
   - It must not use a workspace index.
   - It must not use the first team.
   - It must not use the current workspace as a replacement for the captured team.

5. Two concurrent operations can use different teams.
   - Each operation keeps its own snapshot.
   - The completion of one operation cannot clear or replace state for the other operation.
   - A singleton model must keep the operation scope with each in-flight result.

6. Team-scoped success, error, and loading state carries the captured scope.
   - A result event includes the same team snapshot as the request.
   - Concurrent operations for the same team also use an operation identifier.
   - A view can use its local window identity for presentation only.
   - A view must match a completion by operation identity and captured team.

7. A new Agent Mode conversation captures its owner before its first request.
   - A team conversation owns one team snapshot.
   - A personal conversation has an explicit personal owner.
   - The conversation owner does not change during the conversation.

8. Agent Mode follow-ups, retries, streams, and local policy checks use the conversation owner.
   - A follow-up does not read `current_workspace()`.
   - A response retry does not read the current window team.
   - A stream event does not change the conversation owner.
   - The selected model and harness are snapshots for the request that uses them.

9. A restored cloud conversation uses its server owner.
   - `Owner::Team` resolves to the matching team snapshot.
   - A restored team conversation registers its new window with that team.
   - Restore does not use the first team or the current window team.
   - If team owner metadata is missing, the client must resolve it before new team-scoped work.
   - The client must not guess a team when owner resolution fails.

10. A remote child run inherits the parent conversation's team.
    - The client sends a concrete team identity.
    - The server does not choose a different team for that child.
    - A personal parent creates a personal child unless the user explicitly selects another supported owner.

11. Multi-team CLI commands require an explicit team selection for team-scoped work.
    - The CLI accepts `--team-id <uid>`.
    - The CLI validates that the selected team belongs to the user.
    - `--personal` remains an explicit personal selection.
    - A team-scoped command does not fall back to personal when several teams exist.
    - Existing single-team defaults can remain for backward compatibility.

12. `run-cloud` sends a concrete team identity for team work.
    - The request does not use a boolean that lets the server choose a team.
    - A remote child request uses the same concrete identity as its parent.
    - This behavior requires a matching public API contract.

13. Workspace policy remains workspace-global.
    - BYOK, BYOE, AWS, GEAP, and autonomy policy use the workspace that owns the captured team.
    - They do not use the workspace that is current when a later request starts.
    - A metadata refresh can update policy values for that same workspace.
    - It cannot move the operation to another workspace.

14. Model and harness catalogs keep their declared product scope.
    - Feature-model catalogs are per workspace.
    - Harness catalogs are per user.
    - A view reads the feature-model catalog for the workspace that owns its captured team.
    - A request keeps its selected model and harness after it starts.

15. Workspace billing settings use a real workspace identity.
    - Usage-based pricing and add-on settings are workspace-admin operations.
    - A team selection resolves to its owning workspace.
    - The client does not convert a team UID string into a workspace UID.
    - Team-admin billing operations continue to use a team identity.

16. Billing and purchase completions return to the initiating scope.
    - A workspace-admin completion carries its workspace scope and operation identifier.
    - A team-admin completion carries its team scope and operation identifier.
    - Global success events do not clear loading state in an unrelated window.

17. User-scoped, resource-scoped, and cross-team operations keep their existing scope.
    - Authentication-only work does not require a team snapshot.
    - A request that is fully scoped by a server conversation, run, or task ID does not add a second team selector.
    - A cross-team listener does not pretend to belong to one team.
    - These operations must not call team-scoped APIs without an explicit team snapshot.

18. Missing or stale team membership fails safely.
    - A new team-scoped operation fails before it sends a request if the selected team is no longer available.
    - An in-flight operation can finish under its captured team while the server still authorizes that team.
    - A server authorization failure is returned to the same operation scope.
    - The client does not retry under another team.

19. Team selection remains new-window-only in the first rollout.
    - The team switcher opens a new team-scoped window.
    - The rollout does not add in-window team switching.
    - Tests still change the source window assignment during an await.
    - This test covers metadata reconciliation and future in-window switching.

20. The change is staged.
    - The boundary type and asynchronous helper land first.
    - Agent Mode and CLI migrations land after the boundary is stable.
    - Billing migrations land only after the workspace-versus-team server contract is confirmed.
    - Catalog and remaining policy migrations land after their server scope is confirmed.

## Decisions
1. Use both context-preserving spawn helpers and long-lived owner snapshots.
   - A helper is small and fits short future-and-callback work.
   - An owner snapshot fits conversations, runs, retries, and streams.
   - Either design alone leaves confirmed risks.

2. Use a sealed `TeamContext`.
   - Normal callers cannot construct it from a raw UID.
   - Normal callers cannot extract a raw UID.
   - The request edge has controlled extraction.
   - A thin newtype would improve naming but would not enforce the boundary.

3. Permit `TeamContext: Clone`.
   - Clones keep the same immutable team identity.
   - The safety property is stable identity, not one-time use.
   - A non-clone type would add ownership complexity without preventing a different raw UID from being selected.

4. Carry the team with the value as `TeamScoped<T>`.
   - This shape works across callbacks, model events, view events, and channels.
   - A tuple is smaller but is easier to split by accident.

5. Use concrete team identity for cloud run creation.
   - A boolean delegates selection to the server.
   - That delegation cannot preserve the parent's exact team in a multi-team account.

6. Key team-scoped results by team and operation identity.
   - Team-only state still collides when two operations run for one team.
   - Window-only state loses the server scope.

7. Keep feature-model catalogs per workspace and harness catalogs per user.
   - This matches the current server response ownership.
   - Per-team duplication has no confirmed product contract.

## Assumptions
- **Pending requester confirmation:** Usage-based pricing and add-on settings are workspace-admin operations. The client must resolve the real workspace UID.
- **Pending requester confirmation:** BYOK, BYOE, AWS, GEAP, and autonomy policies are workspace-global.
- **Pending requester confirmation:** Team selection remains new-window-only for this rollout.
- The server can add a concrete team UID to the `run-cloud` and remote-child contract.
- Server conversation owner metadata is authoritative for restore.
- A raw team UID is an identity value. It is not a secret. The boundary still hides it to prevent accidental reselection and substitution.

## Out of scope
- Add in-window team switching.
- Fix every confirmed wrong-team path in one implementation change.
- Add a global team value to `warpui_core`.
- Add a team field to every authenticated request.
- Change user-scoped or cross-team listeners into team-scoped listeners.
- Add `X-Warp-Team-Uid` to every request. The client adds it only when a server endpoint contract requires it.
- Change server authorization rules.
- Change billing product scope without product and server confirmation.
- Redesign team selection UI.
- Add new user-visible telemetry.
