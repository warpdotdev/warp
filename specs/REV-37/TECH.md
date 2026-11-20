# REV-37: Technical design for stable team identity

Jira: https://warp-dev-staging.atlassian.net/browse/REV-37
Companion: `specs/REV-37/PRODUCT.md`
Source commit: `e4857bd60cdd1bc0333e273495d3ae243a64aea9`

## Context
The client stores window team assignments in the `UserWorkspaces` singleton. It also has fallbacks that read the current workspace and its first team.

- [`app/src/workspaces/user_workspaces.rs:109-116`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/workspaces/user_workspaces.rs#L109-L116) stores `current_workspace_uid`, `workspaces`, and `window_team_uids`.
- [`app/src/workspaces/user_workspaces.rs:295-340`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/workspaces/user_workspaces.rs#L295-L340) selects teams for windows and views. `inherited_or_default_team_uid` falls back to `teams.first()`.
- [`crates/warpui_core/src/core/view/context.rs:558-637`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/crates/warpui_core/src/core/view/context.rs#L558-L637) runs a future on the background executor and passes only its output to the view callback.
- [`crates/warpui_core/src/core/model/context.rs:314-474`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/crates/warpui_core/src/core/model/context.rs#L314-L474) has the same output boundary. Its retry helper re-enters `spawn`.
- [`app/src/ai/agent/conversation.rs:253-393`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/ai/agent/conversation.rs#L253-L393) stores conversation and server metadata. It has no immutable local team owner.
- [`app/src/ai/agent/api.rs:122-165`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/ai/agent/api.rs#L122-L165) defines `RequestParams` without team or workspace scope.
- [`app/src/ai/agent/api.rs:310-380`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/ai/agent/api.rs#L310-L380) rebuilds request policy from `UserWorkspaces` and `current_workspace()`-based helpers.
- [`app/src/ai/orchestration/remote_child.rs:283-312`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/ai/orchestration/remote_child.rs#L283-L312) sends `team: None` for a remote child.
- [`app/src/root_view.rs:1628-1728`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/root_view.rs#L1628-L1728) does not bind `FromCloudConversationId` to the conversation owner. It uses the inherited-or-default selector.
- [`app/src/ai/agent_sdk/common.rs:103-130`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/ai/agent_sdk/common.rs#L103-L130) resolves CLI owners with `sole_team_uid()` and can fall back to personal.
- [`app/src/server/server_api/ai.rs:208-224`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/server/server_api/ai.rs#L208-L224) defines `SpawnAgentRequest.team` as `Option<bool>`.
- [`app/src/server/server_api/workspace.rs:92-116`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/server/server_api/workspace.rs#L92-L116) writes a team UID string into the GraphQL `workspace_uid` field.
- [`app/src/server/server_api/workspace.rs:137-161`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/server/server_api/workspace.rs#L137-L161) reads AI overages from the first returned workspace.
- [`app/src/workspaces/user_workspaces.rs:1586-1727`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/workspaces/user_workspaces.rs#L1586-L1727) emits global billing success events and writes overages to all teams in the current workspace.
- [`app/src/server/server_api/ai.rs:1879-1895`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/app/src/server/server_api/ai.rs#L1879-L1895) removes workspace index zero for feature models.
- [`crates/http_client/src/lib.rs:283-362`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/crates/http_client/src/lib.rs#L283-L362) adds global client and platform headers. It has no team header.
- [`crates/warp_server_client/src/base_client.rs:46-90`](https://github.com/warpdotdev/warp/blob/e4857bd60cdd1bc0333e273495d3ae243a64aea9/crates/warp_server_client/src/base_client.rs#L46-L90) shows the request-local policy pattern for ambient headers.

The Jira investigation compiled prototypes for the proposed ownership shapes on stable Rust. The helper needs no higher-ranked trait bound. It needs no boxed future beyond the existing `spawn` implementation.

## Design principles
1. Capture identity at an approved selection boundary.
2. Carry identity with work and results.
3. Keep raw identity out of normal operation code.
4. Use owner snapshots for long-lived work.
5. Keep user, resource, and cross-team scopes explicit.
6. Do not add app types to `warpui_core`.
7. Migrate call sites in stages.

## Proposed changes
### 1. Add `TeamContext` in the app crate
Add `app/src/workspaces/team_context.rs`. Re-export the public opaque type from `app/src/workspaces/mod.rs`.

The type has this shape:

```rust path=null start=null
#[derive(Clone, Eq, PartialEq)]
pub struct TeamContext {
    team_uid: ServerId,
}
```

Rules:
- Keep `team_uid` private.
- Do not implement `From<ServerId>` or `Into<ServerId>`.
- Do not expose `as_server_id`.
- Permit `Clone`. Every clone has the same immutable identity.
- Use a custom `Debug` implementation that does not expose the raw UID.
- Keep `TeamContext::new` private to the `workspaces` module.
- Keep test-only identity assertions behind `#[cfg(test)]`.

`UserWorkspaces` is the construction authority. Add these approved capture paths:

```rust path=null start=null
impl UserWorkspaces {
    pub fn team_context_for_window(
        &self,
        window_id: WindowId,
    ) -> Option<TeamContext>;

    pub fn team_context_for_view<T: Entity>(
        &self,
        ctx: &ViewContext<T>,
    ) -> Option<TeamContext>;

    pub fn team_context_for_owner(
        &self,
        owner: &Owner,
    ) -> anyhow::Result<ConversationOwnerContext>;

    pub fn resolve_cli_team_context(
        &self,
        requested_team_id: &str,
    ) -> anyhow::Result<TeamContext>;
}
```

`resolve_cli_team_context` parses the ID and verifies current membership. It does not create a context for an arbitrary server ID.

Add helpers that accept `&TeamContext`:
- `team_for_context`
- `workspace_for_context`
- Team policy queries
- Workspace policy queries

These helpers can read the private UID because they stay in the `workspaces` module. They must not return the UID.

### 2. Seal raw UID extraction at request edges
Add `app/src/server/server_api/team_context_edge.rs`.

Define a zero-sized `TeamContextEdgeToken`. Keep its constructor private to the request-edge module. The opaque type can expose one crate-private extraction method that requires this token:

```rust path=null start=null
impl TeamContext {
    pub(crate) fn team_uid_for_request(
        &self,
        _edge: &TeamContextEdgeToken,
    ) -> ServerId;
}
```

Normal app modules can name the method. They cannot call it because they cannot construct the token. Request-edge functions construct the token and immediately build a GraphQL variable, REST body, or request-local header.

Rules:
- Team-scoped client traits accept `&TeamContext`.
- Request builders extract the UID only while they build wire data.
- They do not return the raw UID.
- They do not cache the raw UID in a singleton.
- They do not set a team on the shared `BaseClient`.

The first implementation does not add `X-Warp-Team-Uid`. Current contracts use body fields.

If a server endpoint adopts `X-Warp-Team-Uid`, add it as request-local data. Follow the `AmbientHeaderPolicy` pattern. The request builder must require `&TeamContext`. Never derive this header from current window state.

### 3. Add `TeamScoped<T>`
Add this type beside `TeamContext`:

```rust path=null start=null
pub struct TeamScoped<T> {
    context: TeamContext,
    value: T,
}

impl<T> TeamScoped<T> {
    pub fn context(&self) -> &TeamContext;
    pub fn value(&self) -> &T;
    pub fn into_parts(self) -> (TeamContext, T);
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> TeamScoped<U>;
}
```

Use `TeamScoped<T>` for values that cross:
- A future and completion callback.
- A model event.
- A view event.
- A channel.
- Stored request state.

Do not use it for user-scoped or cross-team values.

### 4. Add app extension traits for view and model spawn
Add `app/src/workspaces/team_context_spawn.rs`.

Do not change `warpui_core`. The app crate can implement a local extension trait for foreign `ViewContext` and `ModelContext` types.

Use this view shape:

```rust path=null start=null
pub trait ViewContextTeamExt<T: Entity> {
    fn spawn_with_team_context<W, S, F, U>(
        &mut self,
        team_context: TeamContext,
        work: W,
        on_resolve: F,
    ) -> SpawnedFutureHandle
    where
        W: 'static + FnOnce(TeamContext) -> S,
        S: Spawnable,
        S::Output: SpawnableOutput,
        F: 'static
            + FnOnce(
                &mut T,
                TeamScoped<S::Output>,
                &mut ViewContext<T>,
            ) -> U,
        U: 'static;
}
```

Add the same shape as `ModelContextTeamExt<T>`.

The helper behavior is fixed:
1. It owns the captured `TeamContext`.
2. It clones that context for the work closure.
3. It wraps the future output with the original context.
4. It passes `TeamScoped<Output>` to the main-thread callback.
5. It does not read `UserWorkspaces` in the callback.

This uses the existing `Spawnable` and `SpawnableOutput` bounds. It needs no higher-ranked trait bound. It needs no new `BoxFuture`.

For model retries, add `spawn_with_team_context_retry_on_error` to the model extension trait. Capture the context before the first attempt. Pass a clone to every retry attempt. Return `TeamScoped<RequestState<M>>` to every callback.

Plain `ctx.spawn` remains valid for user-scoped work and for work whose callback does not need a team. Review guidance must reject plain `spawn` when a team-scoped future or callback reselects a team.

### 5. Add an immutable Agent Mode owner
Add:

```rust path=null start=null
pub enum ConversationOwnerContext {
    Personal,
    Team(TeamContext),
}
```

Add `owner_context: ConversationOwnerContext` to `AIConversation`.

Rules:
- Every production conversation constructor receives the owner.
- A new GUI conversation captures the owner from its source view before the first send.
- A CLI conversation captures the owner from explicit CLI selection.
- A restored conversation resolves `server_metadata.permissions.space`.
- An `Owner::Team` restore verifies current membership and creates `TeamContext`.
- A personal owner stays explicit.
- The owner field has no setter.

Legacy restore:
- Fetch server metadata when a persisted team owner is not available.
- Permit a resource-scoped restore to display while metadata loads.
- Do not start a new team-scoped request, child, or mutation until owner resolution succeeds.
- Do not use `inherited_or_default_team_uid`.

Update `NewWorkspaceSource::FromCloudConversationId` so the restore flow obtains owner metadata before final window registration. Register the window with the restored team. A personal conversation registers no team override.

### 6. Make `RequestParams` owner-aware
Add `owner_context: ConversationOwnerContext` to `ConversationData` and `RequestParams`.

Change `RequestParams::new` to receive the conversation owner. Use it for all team or workspace policy resolution.

For the recommended workspace-global policy decision:
- Resolve the owning workspace from `TeamContext`.
- Read BYOK, BYOE, AWS, GEAP, and autonomy policy from that workspace.
- Capture the resulting request settings in `RequestParams`.
- Retries clone `RequestParams`.
- A live retry check must use the same owner context. It must not use `current_workspace()`.

Personal conversations use the existing personal and no-workspace policy path.

### 7. Send concrete team identity for cloud work
This stage requires a server contract change.

Replace `SpawnAgentRequest.team: Option<bool>` with an explicit owner payload. Prefer:

```rust path=null start=null
pub enum SpawnOwner {
    Personal,
    Team { team_uid: ServerId },
}
```

The app-level builder accepts `&ConversationOwnerContext`. The request edge performs the sealed extraction.

Update:
- `warp agent run-cloud`.
- `prepare_remote_child_launch`.
- Local-to-cloud handoff.
- Any retry that rebuilds `SpawnAgentRequest`.

A remote child receives its parent owner by default. A caller cannot pass `None` and let the server choose.

### 8. Make CLI multi-team selection explicit
Replace boolean-only team selection for team-scoped commands.

Rules:
- Add `--team-id <uid>`.
- Keep `--personal`.
- Reject conflicting selectors.
- With one team, keep the current team default.
- With several teams, require `--team-id` or `--personal`.
- Do not fall back to personal after `--team` intent.
- Resolve the result to `ConversationOwnerContext` or `TeamContext` before async work starts.

Migrate secret, schedule, environment, runner, provider, `agent run`, and `agent run-cloud` creation paths that call `resolve_owner`.

### 9. Correlate result state
Replace global team-scoped success events with scoped payloads.

Use:

```rust path=null start=null
pub struct TeamOperationResult<T> {
    pub operation_id: TeamOperationId,
    pub scoped: TeamScoped<T>,
}
```

For workspace-admin operations, use the equivalent opaque `WorkspaceContext`. Create it only from the workspace that owns a validated `TeamContext`.

Views store the operation ID that started their loading state. A completion clears loading only when the operation ID and scope match.

### 10. Correct billing identity after contract confirmation
The recommended product decision classifies usage-based pricing and add-on settings as workspace-admin operations.

Under that decision:
- Add opaque `WorkspaceContext`.
- Resolve it from the captured `TeamContext` through `UserWorkspaces`.
- Change `WorkspaceClient` methods to accept `&WorkspaceContext`.
- Extract the real `WorkspaceUid` at the GraphQL edge.
- Delete `workspace_uid: team_uid.to_string()`.
- Key overage reads and writes by the returned workspace.
- Do not copy one workspace's overages onto every unrelated team object.
- Add scope and operation IDs to success and error events.

If the server owner confirms these are team-admin operations, do not use `WorkspaceContext`. Change the GraphQL contract to accept a real team UID and keep `TeamContext`.

This contract decision blocks the billing migration. It does not block the boundary type or helper.

### 11. Scope model and harness catalogs
Keep feature-model catalogs per workspace. Change the server query so the client can identify which workspace owns each catalog. Store catalogs by opaque workspace scope, not by vector index.

Keep harness availability per user. Do not duplicate it by team without a server product contract.

At request creation:
- Resolve the feature-model catalog through the conversation owner.
- Snapshot the selected model and harness in `RequestParams`.
- Do not change an in-flight request after a catalog refresh.

### 12. Keep long-lived cross-team listeners explicit
Do not add one `TeamContext` to:
- Cloud object listeners that process all authorized spaces.
- Team metadata polling.
- Update management.
- Agent event streams that are scoped by run ID.

These components must carry resource owner data when they emit a team-scoped result. They must not select a team from the active window.

## End-to-end flow
1. A view calls `team_context_for_view`.
2. The operation calls `spawn_with_team_context`.
3. The work closure receives a clone of that context.
4. The future sends a request that accepts `&TeamContext`.
5. The request edge extracts the UID and builds wire data.
6. The helper wraps the output as `TeamScoped<Output>`.
7. The callback handles that scoped output.
8. A later request uses `scoped.context()`.
9. No stage reads the current window team.

For Agent Mode:
1. Conversation creation captures `ConversationOwnerContext`.
2. `AIConversation` stores it.
3. `RequestParams::new` receives it.
4. Policy resolution uses the owning workspace.
5. Follow-ups and retries reuse it.
6. Remote children inherit it.
7. Restore reconstructs it from server owner metadata.

## Staged implementation
### Stage 0: Confirm server and product contracts
Confirm:
- Billing settings are workspace-admin or team-admin.
- Workspace policy stays workspace-global.
- New-window-only team switching remains the rollout behavior.
- The public run API can accept a concrete owner.
- Feature-model responses can identify workspace ownership.

This stage does not change production code.

### Stage 1: Land the boundary and helper
Add:
- `TeamContext`.
- Sealed construction.
- Request-edge extraction token.
- `TeamScoped<T>`.
- View and model extension traits.
- Retry helper.
- Unit tests.

Do not migrate high-risk production call sites in this stage.

This stage fixes no existing call site by itself. It prevents new untyped team-scoped APIs and creates the required migration boundary. It begins the H12 transport-boundary fix without inventing a global header.

### Stage 2: Migrate Agent Mode, CLI, and cloud spawn
Migrate:
- `AIConversation`.
- `ConversationData`.
- `RequestParams`.
- Conversation restore and window registration.
- Agent Mode follow-ups and retries.
- Remote child spawn.
- CLI owner resolution.
- `run-cloud`.

This stage fixes:
- H3: CLI multi-team personal fallback.
- H4: Boolean or absent cloud team selection.
- The Agent Mode portion of H6: live workspace policy reads.
- H10: conversations without an immutable team owner.

This stage needs the concrete cloud owner server contract.

### Stage 3: Migrate billing and team mutations
Migrate:
- Usage-based pricing settings.
- Add-on settings and purchase flows.
- AI overage refresh.
- Team ownership transfer.
- Purchase gating in `AIRequestUsageModel`.
- Success, error, and loading correlation.

This stage fixes:
- H1: team UID written as workspace UID.
- H2: first-workspace overage read and cross-team write.
- H7: global billing success events.
- H8: team ownership transfer without a team identity.
- H9: purchase gating through `current_workspace()`.

This stage is blocked by the billing contract decision.

### Stage 4: Migrate catalogs and remaining policy surfaces
Migrate:
- Feature-model catalog storage and lookup.
- Remaining BYOK, BYOE, AWS, GEAP, and autonomy policy consumers.
- Any settings or banners that still use `current_workspace()` for a team-bound view.

This stage fixes:
- H5: first-workspace and global model catalog behavior.
- The remaining H6 policy paths.

This stage needs the catalog server contract.

### Stage 5: Audit intentional non-team scopes
Audit user-scoped, resource-scoped, and cross-team listeners.

This stage resolves:
- H11 as intentional resource or cross-team behavior where confirmed.
- H12 per endpoint. Add a request-local header only when the server contract requires it.

Do not force one team onto a cross-team listener.

## Confirmed risk map
- H1 → Stage 3.
- H2 → Stage 3.
- H3 → Stage 2.
- H4 → Stage 2.
- H5 → Stage 4.
- H6 → Stages 2 and 4.
- H7 → Stage 3.
- H8 → Stage 3.
- H9 → Stage 3.
- H10 → Stage 2.
- H11 → Stage 5 classification and scoped emissions.
- H12 → Stage 1 boundary and endpoint migrations in Stages 2 through 5.

## Decisions and trade-offs
### Use both the helper and owner snapshot
- Helper only:
  - Advantage: small API.
  - Disadvantage: does not protect long-lived conversation state.
- Owner snapshot only:
  - Advantage: fits Agent Mode.
  - Disadvantage: does not protect short singleton and view operations.
- Both:
  - Advantage: each operation uses the ownership pattern that matches its lifetime.
  - Disadvantage: two APIs must be taught and reviewed.
- Decision: use both.

### Seal construction and extraction
- Thin newtype:
  - Advantage: small migration.
  - Disadvantage: callers can still construct or extract arbitrary IDs.
- Sealed type with edge token:
  - Advantage: enforces approved capture and edge extraction.
  - Disadvantage: adds a small capability type.
- Decision: use the sealed type.

### Use `TeamScoped<T>`
- Tuple:
  - Advantage: minimal code.
  - Disadvantage: callers can split it without a name.
- Wrapper:
  - Advantage: expresses that context and value travel together.
  - Disadvantage: adds mapping methods.
- Decision: use the wrapper.

### Keep clones
- Non-clone context:
  - Advantage: reduces casual copies.
  - Disadvantage: requires private sharing or complex ownership.
- Cloneable immutable context:
  - Advantage: fits existing spawn and retry APIs.
  - Disadvantage: does not prevent a caller from holding several valid contexts.
- Decision: permit clones. Construction sealing prevents arbitrary identity.

### Do not add a global team header
- Global header:
  - Advantage: low call-site work.
  - Disadvantage: races across concurrent windows and requests.
- Request-local body or header:
  - Advantage: preserves operation scope.
  - Disadvantage: each endpoint must adopt it.
- Decision: use request-local scope only.

## Assumptions
- **Pending requester confirmation:** Billing settings are workspace-admin operations.
- **Pending requester confirmation:** BYOK, BYOE, AWS, GEAP, and autonomy policy are workspace-global.
- **Pending requester confirmation:** Team switching stays new-window-only.
- A team belongs to one workspace at one time.
- Server owner metadata is authoritative on conversation restore.
- The public cloud-run API can add a concrete owner without keeping boolean team selection as the source of truth.
- Feature-model APIs can return workspace identity with each catalog.

## Testing and validation
### Stage 1 unit tests
Add tests beside the new modules.

- `team_context_cannot_be_constructed_through_public_conversions`
  - Compile-fail documentation or API review confirms no `From<ServerId>`, `Into<ServerId>`, or public getter.
- `view_spawn_returns_original_team_after_window_assignment_changes`
  - Capture team A.
  - Start work.
  - Reassign the source window to team B before completion.
  - Assert the callback receives team A.
- `model_spawn_returns_original_team_after_window_assignment_changes`
  - Repeat the same sequence for `ModelContext`.
- `retry_uses_first_team_for_every_attempt`
  - Change the window team after the first failure.
  - Assert every attempt and the final callback use team A.
- `team_scoped_map_preserves_context`
  - Transform the value.
  - Assert the context is unchanged.
- `request_edge_builds_body_from_context`
  - Build a mock team request.
  - Assert the body contains team A.
  - Assert no normal API returns the raw ID.

Run:

```bash path=null start=null
cargo nextest run -p warp -E 'test(team_context)'
./script/format
```

### Stage 2 tests
- New team conversation keeps team A after its window changes to team B.
- Follow-up `RequestParams` uses team A policy.
- A retry uses the original owner.
- `Owner::Team` restore registers the window with that team.
- Missing owner metadata does not use the first team.
- Remote child request contains the parent team.
- Multi-team CLI team work errors without `--team-id`.
- `--personal` remains explicit.
- Single-team CLI compatibility remains.

Add a mocked public API serialization test for the concrete owner payload.

### Stage 3 tests
- Billing workspace input uses the real workspace UID.
- A team UID that differs from its workspace UID cannot enter `workspace_uid`.
- Overages update only the returned workspace.
- Two windows start billing work for different scopes.
- Completing one operation clears only its matching loading state.
- Team ownership transfer contains the initiating team.
- Purchase gating uses the initiating scope.

### Stage 4 tests
- Two workspaces can hold different feature-model catalogs.
- A team-bound view reads its owning workspace catalog.
- A model refresh does not change an in-flight request model.
- User-level harness availability remains shared.
- Workspace policy helpers never read `current_workspace()` when they receive `TeamContext`.

### Full validation
Run before each implementation PR:

```bash path=null start=null
cargo nextest run -p warp
cargo clippy -p warp --all-targets --all-features --tests -- -D warnings
./script/format
```

Run `./script/presubmit` before the final migration PR.

No visual proof is required for Stages 1 and 2 because they do not change the team-selection UI. If Stage 3 changes loading or success presentation, record the concurrent two-window flow with computer use.

## Parallelization
Stage 1 is sequential. One owner must establish the boundary, helper, and review rules first.

After Stage 1 lands, three implementation streams can run in parallel:
- `agent-owner` owns Agent Mode conversation, request, restore, and remote-child files. Use a separate worktree and branch `factory/rev-37-agent-owner`.
- `cli-cloud-owner` owns CLI selection and public run request serialization. Use a separate worktree and branch `factory/rev-37-cli-cloud-owner`.
- `billing-scope` owns billing, overage, and result correlation. Start only after the billing contract is confirmed. Use a separate worktree and branch `factory/rev-37-billing-scope`.

The catalog and remaining policy migration starts after the owner helpers are stable. Use branch `factory/rev-37-catalog-policy`.

Each stream opens its own PR. The integration owner checks the shared `TeamContext` API and rejects duplicate constructors or raw UID getters.

## Risks and mitigations
- A crate-private getter can weaken the boundary.
  - Require the unconstructable edge token.
  - Reject public conversions in review.
- Legacy conversations can lack owner metadata.
  - Fetch metadata before new team-scoped work.
  - Fail without a fallback when team scope cannot be resolved.
- A global header can reintroduce the race.
  - Keep team decoration request-local.
- Server and client contracts can roll out at different times.
  - Add server fields first.
  - Keep client parsing backward-compatible during rollout.
  - Do not let the old boolean choose the team after a concrete owner is present.
- A broad migration can hide scope mistakes.
  - Use the staged risk map.
  - Keep each PR limited to one operation family.
- `TeamContext: Clone` can be misunderstood as a new selection.
  - Document that clone preserves identity.
  - Keep construction sealed.

## Out of scope for the first implementation change
- Production call-site migrations.
- Agent Mode owner persistence.
- CLI flag changes.
- Public API changes.
- Billing fixes.
- Model or harness cache changes.
- Policy behavior changes.
- New `X-Warp-Team-Uid` behavior.
- In-window team switching.
- Changes to `warpui_core`.

The first implementation change contains only the boundary type, sealed extraction, scoped result wrapper, extension traits, retry helper, and unit tests.
