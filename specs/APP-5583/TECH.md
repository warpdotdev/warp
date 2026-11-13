# APP-5583: Viewer-access-aware cloud-run links — technical design

Linear: [APP-5583](https://linear.app/warpdotdev/issue/APP-5583/cloudmode-view-in-oz-links-to-old-oz-ui-instead-of-platformwarpdev)

Product behavior is defined in [`PRODUCT.md`](./PRODUCT.md). The temporary removal is tracked in [APP-5602](https://linear.app/warpdotdev/issue/APP-5602/remove-app-5583-factory-access-routing-branch-after-platform-access).

## Context
At Warp commit [`378b74f3`](https://github.com/warpdotdev/warp/tree/378b74f3b8ee32d2abc0de21dbc230bc818b7762):

- [`crates/warp_core/src/channel/config.rs:68`](https://github.com/warpdotdev/warp/blob/378b74f3b8ee32d2abc0de21dbc230bc818b7762/crates/warp_core/src/channel/config.rs#L68) defines `OzConfig`. Its production origin is `https://oz.warp.dev`.
- [`crates/warp_core/src/channel/state.rs:270`](https://github.com/warpdotdev/warp/blob/378b74f3b8ee32d2abc0de21dbc230bc818b7762/crates/warp_core/src/channel/state.rs#L270) exposes only `oz_root_url()`.
- [`app/src/ai/conversation_details_panel.rs:764`](https://github.com/warpdotdev/warp/blob/378b74f3b8ee32d2abc0de21dbc230bc818b7762/app/src/ai/conversation_details_panel.rs#L764) renders the reported “View in Oz” button.
- [`app/src/ai/conversation_details_panel.rs:986`](https://github.com/warpdotdev/warp/blob/378b74f3b8ee32d2abc0de21dbc230bc818b7762/app/src/ai/conversation_details_panel.rs#L986) builds its run URL.
- [`app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs:514`](https://github.com/warpdotdev/warp/blob/378b74f3b8ee32d2abc0de21dbc230bc818b7762/app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs#L514) renders the pill-menu action.
- [`app/src/workspace/view/wasm_view.rs:25`](https://github.com/warpdotdev/warp/blob/378b74f3b8ee32d2abc0de21dbc230bc818b7762/app/src/workspace/view/wasm_view.rs#L25) builds “View all cloud runs.”
- [`app/src/ai/blocklist/block.rs:1123`](https://github.com/warpdotdev/warp/blob/378b74f3b8ee32d2abc0de21dbc230bc818b7762/app/src/ai/blocklist/block.rs#L1123) builds recording artifact links.
- [`app/src/ai/orchestration/remote_child.rs:375`](https://github.com/warpdotdev/warp/blob/378b74f3b8ee32d2abc0de21dbc230bc818b7762/app/src/ai/orchestration/remote_child.rs#L375) exposes a run URL to user-facing remote-child output.

The reported behavior is not caused by an incorrect run ID or route path. Every affected surface reads the same legacy origin.

At warp-server commit [`6f7fcb96`](https://github.com/warpdotdev/warp-server/tree/6f7fcb960786152eee279c88b973ef3dcfd37633):

- [`logic/factory_access.go:25`](https://github.com/warpdotdev/warp-server/blob/6f7fcb960786152eee279c88b973ef3dcfd37633/logic/factory_access.go#L25) computes access from the Factory feature flag, dogfood mode, user overrides, team overrides, and domain overrides. A user-level control assignment can deny access even when a team is allowlisted.
- [`client/packages/factory/src/api/access.ts:24`](https://github.com/warpdotdev/warp-server/blob/6f7fcb960786152eee279c88b973ef3dcfd37633/client/packages/factory/src/api/access.ts#L24) calls `GET /api/v1/factory/access`. A valid response is `{ "allowed": boolean }`. The desktop client reuses the endpoint contract and five-second request timeout, but it does not copy the web client's periodic cache refresh.
- [`client/packages/factory/src/FactoryApp.tsx:199`](https://github.com/warpdotdev/warp-server/blob/6f7fcb960786152eee279c88b973ef3dcfd37633/client/packages/factory/src/FactoryApp.tsx#L199) defines global `/runs` and `/runs/:runId` Platform routes.
- The same router has no global `/agents/:agentId`, `/skills/:skillId`, or `/memory/...` route. Its agent routes are factory-scoped and require a factory ID.

Task provenance cannot replace viewer access. The desktop task model has no reliable Factory source or factory ID. Factory-triggered tasks retain integration sources such as Slack or Linear. Provenance also cannot answer “View all cloud runs.”

## Proposed changes
### Authoritative access probe
Extend the existing authenticated Factory client in `app/src/server/server_api/factory.rs` with the REST access contract:

```rust
pub struct FactoryAccessResponse {
    pub allowed: bool,
}

async fn get_factory_access(&self) -> Result<FactoryAccessResponse>;
```

The exact error type may follow existing `ServerApi` conventions. The contract must distinguish a valid `allowed: false` response from transport, timeout, HTTP, and deserialization failures. Use a five-second request timeout to match the Platform web client.

Do not add `FACTORY_EARLY_ACCESS` to desktop experiment sync. The REST endpoint is the policy boundary. Reconstructing access from one experiment would omit the feature flag, dogfood behavior, domain overrides, and the user-control precedence rule.

### Application-scoped access state
Register one non-persisted application singleton, named for example `FactoryAccessModel`, after `ServerApiProvider` during app initialization. Compile it for every client target that renders an included link. Trigger its first request only after authentication completes.

The model owns:

- `Unknown`, `Allowed`, and `Denied` states.
- One startup request for the current authenticated session.

Lifecycle:

1. Start the first request immediately after `AuthManagerEvent::AuthComplete`.
2. Store a successful `Allowed` or `Denied` result for the rest of that authenticated session.
3. On timeout, transport error, non-success status, or malformed response, keep `Unknown` for the rest of the session. Do not retry.
4. Do not refresh on a timer, foreground transition, panel or menu open, render, or click.
5. Reset to `Unknown` and cancel old work on logout or authenticated-user change. The next successful authentication starts one new request.
6. Do not persist access across launches or accounts. A process restart starts a new request after authentication.
7. Channel and server origins are fixed for the lifetime of a normal client process. A process restart reconstructs the model from the new channel config. A test or development runtime override that changes either origin must also start a new authenticated session before evaluating access.

The model exposes a synchronous state read for render and click handlers. It notifies subscribed views when the startup result changes `Unknown` to `Allowed` or `Denied`, but link destinations are resolved again at click time.

### Central URL resolver
Add one resolver for the included cloud-run destinations. It accepts:

- The current access state.
- A route kind: run detail, run index, or run artifact.
- The run ID and artifact UID required by that route kind.
- The channel-configured Platform and Oz origins.

It returns Platform only for `Allowed`. It returns Oz for `Denied` and `Unknown`.

Keep `oz_root_url`. Add a separate channel-configured Platform origin, such as `platform_root_url`, because both origins are required during the transition. Do not repoint `oz_root_url` to Platform.

Call sites must resolve on activation rather than capturing a URL when the view is constructed. This lets an eager request that finishes while a menu or panel is open affect the next click.

### Channel configuration and staging dependency
The implementation spans two repositories:

1. In `warpdotdev/warp`, add the Platform origin to `OzConfig` or an equivalently scoped channel config and expose it through `ChannelState`.
2. In the private, reachable `warpdotdev/warp-channel-config` repository, emit:
   - `https://platform.staging.warp.dev` for dev builds.
   - `https://platform.warp.dev` for stable and preview builds.
   - `https://platform.staging.warp.dev` for local builds by default, plus a new `PLATFORM_ROOT_URL` override for developers running a local Platform server.
3. Merge the generator change first.
4. Update `script/install_channel_config` in the Warp implementation PR to pin the merged generator revision.
5. Generate dev and stable channel JSON and assert both Platform origins before UI verification.

The APP-5583 implementation owner authors both repository changes. The generator repository was verified reachable during spec research. The cross-repository merge and pin sequence is a blocking dependency for staging video verification, not a post-merge release task.

### Included call sites
Route these through the resolver and apply the PRODUCT copy:

- `app/src/ai/conversation_details_panel.rs`
  - Details button and tooltip.
  - Status chip and tooltip.
  - `OpenInOz` handling for task-backed run links.
  - Skill link copy only; keep its skill URL on Oz.
- `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs`
  - Menu copy, neutral icon, and run destination.
  - Keep the internal action and serialized telemetry value for compatibility.
- `app/src/workspace/view/wasm_view.rs`
  - Run-index destination.
- `app/src/ai/blocklist/block.rs`
  - Recording artifact run destination and query preservation.
- `app/src/ai/orchestration/remote_child.rs`
  - User-visible run destination. Change the helper interface as needed so it can read the resolver state instead of remaining a pure `run_id` formatter.

### Explicit exclusions
Leave these destinations unchanged:

- `app/src/ai/conversation_details_panel.rs` executor agent URL. Platform has no global agent route.
- `app/src/settings_view/billing_and_usage/billing_cycle_usage_rows.rs` service-account agent URL. It has the same missing factory ID and route.
- `app/src/ai/conversation_details_panel.rs` skill URL. Platform has no global skill route; only its copy changes.
- `app/src/terminal/view.rs` memory citation URL. Platform has no memory route.
- `app/src/ai/cloud_environments/mod.rs` environment URL and `app/src/ai/agent_management/cloud_setup_guide_view.rs`. The standalone Platform router does not expose a matching destination.
- `app/src/ai/agent_sdk/ambient.rs` human-readable CLI links. These short-lived commands can exit before the access probe completes and are not necessarily viewed by the authenticated desktop viewer.
- `app/src/ai/agent_sdk/driver/output.rs` JSON `run_url`. This is a machine-readable compatibility surface whose consumer's Platform access can differ from the process that emitted it.

## Decisions
### REST probe instead of experiment sync
- **Chosen:** Call `GET /api/v1/factory/access`.
  - Advantage: one authoritative answer that matches server enforcement.
  - Cost: one authenticated request per authenticated client session.
  - Latency: off the click path. Navigation never waits for it.
- **Rejected:** Sync only `FACTORY_EARLY_ACCESS` to the client.
  - Advantage: no new REST request.
  - Rejected because it duplicates incomplete policy and can disagree with the server.

### Once-per-session, memory-only state
- **Chosen:** Fetch once after authentication and hold the result for that session.
  - Advantage: no per-render requests, refresh timer, foreground hook, request coalescing, or background refresh state.
  - Cost: an eligibility change is not visible until the next launch or account login.
- **Rejected:** Refresh periodically.
  - Advantage: picks up eligibility changes during a long-running session.
  - Rejected because such changes are rare, the conditional is temporary, and the extra lifecycle machinery does not earn its cost.
- **Rejected:** Persist the result.
  - Advantage: no cold-start unknown state.
  - Rejected because an old account or revoked entitlement could incorrectly route to Platform.

### Fail to Oz without delaying a click
- **Chosen:** `Unknown` and all probe failures use Oz immediately.
  - Advantage: every run remains openable.
  - Cost: an enrolled viewer can see the exact legacy UI reported in APP-5583 during the first network round trip. A failed probe keeps that viewer on Oz for the session.
  - Rationale: the eager request makes this window uncommon in normal use, while routing an ineligible viewer to Platform produces a hard access dead end.
- **Chosen:** Do not retry a failed initial probe.
  - Rationale: authentication has already completed over the network before this request starts, so a startup connectivity race is less likely. One retry would add delayed task state for a code path scheduled for removal, while the safe Oz fallback still works.
- **Rejected:** Wait for the access response when the user clicks.
  - Rejected because navigation latency would depend on a request with a five-second timeout.
- **Rejected:** Treat unknown as allowed.
  - Rejected because waitlisted users would lose the usable run link during startup and outages.

### Viewer access instead of run provenance
- **Chosen:** Route all included links from viewer access.
- **Rejected:** Route Factory-originated tasks to Platform.
  - Rejected because no reliable provenance or factory ID is present, and the approach cannot handle the global run index.

### Narrow route scope
- **Chosen:** Change direct interactive run and run-index links whose Platform route is verified.
- **Rejected:** Change every use of `oz_root_url`.
  - Rejected because agent, skill, memory, environment, CLI, and SDK URLs have different route or compatibility contracts.

### Preserve telemetry continuity
- **Chosen:** Keep `PillBarActionKind::ViewInOz` and its `view_in_oz` serialized value.
- **Cost:** The internal analytics name no longer matches the UI label.
- **Removal:** APP-5602 requires an analytics review before renaming or retiring it.

## Assumptions
- Authenticated desktop and WASM clients can call the existing access endpoint through `ServerApi`.
- The endpoint remains the authoritative viewer-access contract until the waitlist is removed.
- Platform continues to support global `/runs` and `/runs/:runId`.
- Oz continues to serve equivalent run routes until APP-5602 removes the fallback.
- No new Figma design is required. Existing layout, sizing, and focus behavior remain unchanged.

## Testing and validation
### Automated access-model tests
Add deterministic model tests with a fake client and fake clock:

- `factory_access_fetches_eagerly_after_auth`
- `factory_access_holds_successful_result_for_session`
- `factory_access_does_not_refresh_on_time_or_foreground_changes`
- `factory_access_resets_on_logout_or_user_change`
- `factory_access_failure_remains_unknown_without_retry`
- `factory_access_maps_timeout_http_error_and_malformed_body_to_unknown`

### Automated URL tests
Add a resolver matrix that checks:

- Stable allowed → `https://platform.warp.dev/runs/<id>`.
- Dev allowed → `https://platform.staging.warp.dev/runs/<id>`.
- Denied and unknown → channel-matching Oz URLs.
- Timeout and malformed-response model states → unknown → Oz.
- `/runs` index selection in both access states.
- Recording artifact query encoding and preservation in both access states.
- A state change while a view is open affects the next activation.

Update `app/src/ai/conversation_details_panel_tests.rs` and add focused tests beside the new model/resolver. Run:

```sh
cargo test -p warp factory_access
cargo test -p warp cloud_run_web_url
cargo test -p warp conversation_details_panel
```

### UI and copy tests
Assert:

- The details button says “View cloud run.”
- The details tooltip says “View this cloud run in the web app.”
- The status tooltip says “View cloud run in the web app.”
- The pill menu says “View cloud run” and does not use the Oz icon.
- The run index still says “View all cloud runs.”
- The skill link says “Open in web app.”
- `PillBarActionKind::ViewInOz` still serializes to `view_in_oz`.

Run formatting and the repository-prescribed targeted presubmit for the changed Rust targets:

```sh
cargo fmt --check
```

### Channel-config verification
Before UI verification:

1. Run the pinned generator for dev and assert `platform_root_url` is `https://platform.staging.warp.dev`.
2. Run it for stable and assert `platform_root_url` is `https://platform.warp.dev`.
3. Build the dev client from the pinned generator revision. Do not substitute a hardcoded test URL.

### Required video
Use computer-use recording against the dev build. The proof must show:

1. The updated “View cloud run” copy in the reported details panel.
2. An allowed access response followed by a click that opens `https://platform.staging.warp.dev/runs/<run_id>`.
3. A denied or failed access response followed by a click that opens `https://oz.staging.warp.dev/runs/<run_id>` without delay.
4. “View all cloud runs” opening the matching selected host at `/runs`.

Use deterministic test accounts or a local access-endpoint fixture to exercise both states. The recording must show the browser host and must not expose credentials or tokens. Attach the video to the implementation PR and the originating Slack thread.

## Risks and mitigations
- **First-click legacy routing for an enrolled viewer.** Mitigation: fetch eagerly after authentication and resolve again on every activation after the startup result arrives.
- **Eligibility changes or a failed probe remain stale for the session.** Accepted because the Oz fallback remains usable, mid-session eligibility changes are rare, and APP-5602 removes this temporary branch at general availability.
- **Staging silently uses production Platform.** Mitigation: keep the origin in channel config, land the generator PR first, pin it, and verify generated dev/stable JSON before the video.
- **Access probe request fanout.** Mitigation: only the authentication-complete lifecycle starts the single session request. UI surfaces never fetch access.
- **Scope expands into broken Platform paths.** Mitigation: only run routes move. Every excluded destination is listed above.
- **Temporary code becomes permanent.** Mitigation: APP-5602 exists with a policy-based trigger and explicit deletion criteria.

## Parallelization
Parallel implementation is not recommended. The access model, central resolver, call-site changes, and tests share one small state contract and should evolve in one branch. The cross-repository generator change must land before the Warp pin update, so that work is sequential rather than parallel.
