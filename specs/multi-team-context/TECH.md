# Multi-team context

## Goals

- Replace views and windows as mutable proxies for operation scope with an explicit `TeamContext` captured from a `ViewContext`.
- Keep each logical operation scoped to the same team across asynchronous work and window team changes.
- Support several windows using different teams without process-wide “current” or “default” team state.
- Keep current-team UI reactive when a window changes teams.
- Distinguish a team inferred from the current window from an explicit team target or an existing resource.
- Preserve these guarantees through compiler-enforced API boundaries and focused code documentation.

## Constraints

- `TeamContext` is the owned operation scope for a team inferred from the current view and window. `UserWorkspaces` is its only minting authority, and the public owned-context minting API accepts a real `ViewContext`, not `WindowId`, `AppContext`, a view handle, or a raw team UID.
- `UserWorkspaces` may keep its private `WindowId`-to-team-UID registry. General application code cannot use that registry to mint an owned `TeamContext`.
- `TeamContext` is opaque, neither `Clone` nor `Copy`, and not generally wrapped in `Arc`. It has no public constructor and does not expose its raw UID to general application code. One logical operation moves the context between owners and borrows it for related work.
- A captured `TeamContext` is stable. It does not borrow its view and does not change or retarget when the window's selected team later changes.
- `TeamRenderContext<'a>` is the borrowed read scope for current-team rendering. `UserWorkspaces` resolves it from `WeakViewHandle` plus `AppContext` on each render. It cannot mint `TeamContext`, enter transport or mutation APIs, or outlive its borrow.
- A current-team view observes `UserWorkspaces` so `ctx.notify()` after a window-team change triggers another render. The next render resolves a new `TeamRenderContext`; the view does not cache an owned `TeamContext` merely for rendering.
- Work initiated from a view exchanges `ViewContext` for `TeamContext` before crossing into a model, command, event, future, or transport layer. Code does not resolve the operation's team again from a window or view handle after work starts.
- Current-team views and long-lived controllers do not store ambient `TeamContext`. A team-bound view or operation-scoped model may own one when its lifetime and behavior are intentionally bound to the captured team.
- Operations that explicitly identify a target team may continue to use a raw team UID. This includes discovery, joining, switching, and administration for a team named by the UI. Existing-resource operations prefer the resource identifier when the server can derive and authorize its team.
- A window either has a selected team or follows no-team behavior. This design does not add a broader pending or global scope enum.
- Global models and singletons cannot treat one window's selection as process-wide state. Team-derived state is request-local, keyed by team, or handled by an explicitly cross-team operation.
- Codebase-indexing permission is evaluated for the team scope that starts or uses the work. Shared local index artifacts may be reused across allowed scopes, but a denied scope cannot create, sync, retrieve from, or expose an index to AI.
- When every known team scope has effective codebase indexing disabled, process-global indexing work does not run. The client does not restore active indices, register index filesystem watchers, automatically discover work, start background sync, or expose indexed retrieval merely because persisted index data exists.
- Trusted transport may inspect `TeamContext` only to put the same selected team into the request header or body. The response must update that same team's state; adding the type to a signature is insufficient if transport ignores it or falls back to the first/default team. The server still authenticates membership and rejects inconsistent scope.
- Migration PRs may temporarily retain `team_uid_for_window`, `team_for_window`, `team_for_view`, and `team_for_view_handle`. The completed design removes those general resolvers and retains only `team_render_context_for_view_handle` for borrowed render scope.
- Public API documentation explains these capability, rendering, and ownership guarantees. Routine call sites remain self-documenting; comments do not narrate implementation steps, list callers, or describe migration history.

## Context types

`TeamContext` and `TeamRenderContext` represent different lifetimes and authorities. They are not interchangeable.

```rust
pub(crate) struct TeamContext {
    team_uid: ServerId,
}

pub(crate) struct TeamRenderContext<'a> {
    team: &'a Team,
}
```

### `TeamContext`

- It is an owned snapshot of the team selected for a view's window when an operation starts.
- Only `UserWorkspaces::team_context_for_view` constructs it for general application code.
- It moves through commands, events, models, futures, and transport without resolving the originating view again.
- Team-scoped mutation and request APIs accept `TeamContext` or `&TeamContext`.

### `TeamRenderContext`

- It borrows the current `Team` from `UserWorkspaces` and is resolved again whenever a view renders.
- Team-scoped presentation and policy-read APIs accept `&TeamRenderContext<'_>`.
- It exposes only the read access needed by rendering. It has no raw-UID conversion and no conversion to `TeamContext`.
- Its lifetime prevents it from entering normal `'static` futures or stored model state.
- The constructor is render-only by contract. `WeakViewHandle + AppContext` does not become an owned team-operation authority.

### Integration

Both constructors use the same private window-to-team registry, but produce different results:

```rust
impl UserWorkspaces {
    pub(crate) fn team_context_for_view<T>(
        &self,
        ctx: &ViewContext<T>,
    ) -> Option<TeamContext> {
        self.team_uid_for_window(ctx.window_id())
            .map(TeamContext::new)
    }

    pub(crate) fn team_render_context_for_view_handle<'a, T>(
        &'a self,
        view: &WeakViewHandle<T>,
        app: &AppContext,
    ) -> Option<TeamRenderContext<'a>> {
        let window_id = view.window_id(app)?;
        let team_uid = self.team_uid_for_window(window_id)?;
        let team = self.team_from_uid(team_uid)?;

        Some(TeamRenderContext { team })
    }
}
```

The first path captures stable operation scope. The second path borrows the window's current team for one synchronous render. Neither path converts into the other.

## Implementation

### PR 0: Context foundation

This PR is additive and is the only blocking client dependency for later migration PRs.

#### Changes

- Add an opaque `pub(crate) TeamContext` in `app/src/workspaces/user_workspaces.rs`. It stores the captured team UID privately and implements neither `Clone` nor `Copy`.
- Add `pub(crate) TeamRenderContext<'a>` in the same module. It privately borrows the current `Team` and provides no conversion to raw UID or owned `TeamContext`.
- Add `UserWorkspaces::team_context_for_view(&ViewContext<T>) -> Option<TeamContext>`. It snapshots the team currently assigned to that view's window.
- Add `UserWorkspaces::team_render_context_for_view_handle(&WeakViewHandle<T>, &AppContext) -> Option<TeamRenderContext<'_>>`. It resolves the team assigned to the view's current window for synchronous rendering.
- Add `UserWorkspaces::team_for_context(&TeamContext) -> Option<&Team>` for synchronous metadata reads.
- Document `TeamContext` as a stable selection snapshot and `TeamRenderContext` as a borrowed current-render scope. Neither is proof of current membership or server authorization.
- Keep raw UID extraction private. Add a narrowly documented transport boundary only when the first network API migration needs it.

#### Non-goals

- Do not migrate production call sites or change server API traits.
- Do not add `X-Warp-Team-Uid`.
- Do not remove the existing window/view resolvers yet.
- Do not add personal/global scope types, cancellation, invalidation, or membership leases.
- Do not add stored `TeamContext` fields to views or models.

#### Tests

- Two views in windows assigned to different teams mint contexts that resolve to their respective teams.
- A `TeamContext` captured for team A never resolves as team B after the window assignment changes to B.
- A newly resolved `TeamRenderContext` follows that same window from team A to team B.
- Both constructors return `None` for a view whose window has no team.
- A captured operation context for a removed team no longer resolves to team metadata, while the next render resolves the window's reconciled team; neither context is silently retargeted.

#### Validation

- Run `./script/format`.
- Run the focused `user_workspaces_tests`.
- Run `cargo check -p warp --lib`.
- Run `git diff --check`.

### Group 1: Current-window settings and policy verticals

These PRs establish the migration pattern for UI whose team is inferred from the current window. Each PR adds context-based `UserWorkspaces` accessors for its surface and migrates only that vertical. Compatibility accessors remain until later groups migrate their callers.

#### PR 1A: Warp Agent settings and AI policy

- Migrate the Warp Agent settings page and its team-derived BYO key, custom endpoint, model, host, Bedrock, Gemini Enterprise, remote-session, cloud-conversation, and overage policy reads.
- Mint `TeamContext` for settings changes whose team is inferred from the current window.
- Leave Agent Mode request ownership, global credential/model caches, and transport headers to later groups.

#### PR 1B: Privacy and security settings

- Migrate the privacy settings page and its team-derived telemetry, UGC collection, secret-redaction, and AI data-policy reads and changes.
- Keep explicitly targeted administration operations on raw team UIDs.
- Do not move team scope into process-global privacy state.

#### PR 1C: Code indexing settings

- Migrate the code-indexing settings page and its team-derived codebase-context and automatic-indexing policy reads and changes.
- Mint operation scope when a window requests indexing, resync, deletion, or indexed retrieval; do not resolve that operation's team from the window again.
- Present indexing as unavailable in a window whose current team disables it, even when another allowed team has created a shared local index for the same repository.
- Leave global or cross-window indexing cache redesign out of this PR.

#### PR 1D: Billing and usage presentation

- Migrate current-window overage, purchase-policy, add-on-credit, and usage presentation reads.
- Keep a billing action bound to an explicitly displayed team on that action's raw team UID.
- Defer `GetAiOveragesForWorkspace`, billing transport, and `X-Warp-Team-Uid` changes to PR 2A.

#### Shared acceptance criteria

- Rendering resolves `TeamRenderContext` through `team_render_context_for_view_handle` and does not store `TeamContext`.
- Each current-team view observes `UserWorkspaces` and rerenders after its window's team changes.
- A view action mints `TeamContext` only when its target is inferred from that view's current window.
- Models, futures, and callbacks in the vertical receive the operation context instead of resolving a window or view again.
- Explicit-team and existing-resource operations retain their raw team or resource identifiers.
- Focused tests cover team A, team B in another window, a window-team change, and no-team behavior.
- Each PR runs `./script/format`, its focused tests, `cargo check -p warp --lib`, and `git diff --check`.

PRs 1A–1D can begin after PR 0. They own disjoint UI surfaces but add methods in `user_workspaces.rs`, so they should use distinct accessors and rebase before merge rather than editing the same compatibility methods.

### Group 2: API migrations

[`specs/multi-team-api-context/TECH.md`](../multi-team-api-context/TECH.md) owns PRs 2A–2F. Those PRs add request-local team scope, preserve raw UIDs for explicitly named teams, keep existing-resource operations resource-scoped, and coordinate required server validation.

Group 2 can proceed in parallel with Group 1 once PR 0 lands and a call site's ownership path is ready. It does not own the global state redesigns in Group 3 or the resolver deletion in Group 4.

### Group 3: Architectural state migrations

These PRs require an explicit design review before implementation. They must not be assigned as mechanical signature or call-site migrations. Each owner must decide whether team-derived state is keyed by team or supplied by a view-owned operation, and must test two windows using different teams concurrently.

#### PR 3A: `LLMPreferences` model availability

- Replace one process-wide team-derived model catalog with per-team state or request-local model availability.
- Define cache invalidation when team policy or credentials change.
- Keep no-team model availability explicit without selecting a default team.

#### PR 3B: `ApiKeyManager` and GEAP credentials

- Replace one global GEAP policy and credential state with state keyed by the team or request that owns the mint.
- Carry team scope through refresh, retry, cancellation, and completion without borrowing a later window selection.
- Ensure one team's policy change cannot enable, disable, or overwrite another team's credential state.

#### PR 3C: `AIRequestUsageModel`

- Partition team-derived limits, overages, credits, and purchase policy, or compute them from the requesting operation's context.
- Prevent one window's usage refresh from changing another team's displayed or enforced state.
- Keep no-team behavior separate from any team's usage data.

#### PR 3D: `PrivacySettings`

- Remove process-wide storage of values derived from one team's privacy policy.
- Decide which values are user-global and which are team-derived before choosing cache keys.
- Ensure Group 1B views receive current-team values without making them global defaults.

#### PR 3E: Persisted workspace and codebase indexing

- Remove implicit current/default-team policy from persisted workspace restoration and codebase-indexing decisions.
- Keep local index artifacts shareable where safe, but track which team scopes may create, sync, and use each index.
- Carry the authorizing scope into background work so indexing can continue without consulting a later window selection.
- If every known team scope has effective codebase indexing disabled, do not restore active indices, register index filesystem watchers, automatically enqueue repositories, start background sync, or expose indexed retrieval. Persisted artifacts may remain inactive on disk.
- In a mixed-policy process, run shared background indexing work only when at least one allowed scope authorizes it, and reject creation, mutation, and retrieval from disabled scopes.
- Ensure background work cannot inherit whichever window most recently changed teams.

### Group 4: Enforcement cleanup

This group starts only after Groups 1–3 have migrated all consumers.

- Delete `team_for_window` and `team_for_view`.
- Delete `team_for_view_handle`; retain only `team_render_context_for_view_handle` for borrowed render-time current-team data.
- Make `team_uid_for_window` private to `UserWorkspaces` context minting or remove it if no longer needed.
- Delete raw team-UID variants of APIs whose team is inferred from the current window. Keep raw UIDs for the explicit-target operations defined in the constraints and Group 2.
- Remove compatibility accessors introduced during vertical migrations.
- Narrow temporary `TeamContext`-to-`ServerId` extraction to trusted transport and registry internals.
- Add focused grep or lint checks if they can reliably prevent the removed ambient patterns from returning.
- Run `./script/format`, focused multi-window tests, `cargo check -p warp --lib`, the `cargo clippy` command used by `./script/presubmit`, and `git diff --check`.
