# Define team scoping for the built-in Factory MCP client — Tech Spec

See [`product.md`](./product.md) for the behavior contract. GitHub issue [#15608](https://github.com/warpdotdev/warp/issues/15608) remains the source of truth for the work item.

## Context
The Warp client currently builds one streamable-HTTP Factory MCP installation with a static `Authorization` header:
- [`app/src/ai/mcp/builtin.rs:64-104 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/ai/mcp/builtin.rs#L64-L104) constructs the installation.
- [`crates/mcp/src/runtime.rs:82-109 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/crates/mcp/src/runtime.rs#L82-L109) builds one `reqwest::Client` whose headers are static for the transport.
- [`app/src/ai/mcp/templatable_manager.rs:39-106 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/ai/mcp/templatable_manager.rs#L39-L106) stores active servers in a process-wide singleton keyed by installation UUID.
- [`app/src/ai/mcp/templatable_manager/native.rs:770-833 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/ai/mcp/templatable_manager/native.rs#L770-L833) attaches one built-in connection for interactive GUI and TUI clients.

Team selection has a different lifetime:
- [`app/src/workspaces/user_workspaces/mod.rs:344-394 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/workspaces/user_workspaces/mod.rs#L344-L394) stores one selected team per window.
- [`app/src/workspaces/user_workspaces/team_workspace_settings.rs:41-126 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/workspaces/user_workspaces/team_workspace_settings.rs#L41-L126) prevents callers from minting team scopes from bare IDs.
- [`app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs:89-155 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs#L89-L155) executes a tool for one terminal surface, but the `ReconnectingPeer` it selects uses the shared transport.
- [`app/src/server/team_scope.rs:4-25 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/server/team_scope.rs#L4-L25) shows the safe pattern for ordinary request-local team scope.

The headless driver does not use the interactive attach path:
- [`app/src/ai/agent_sdk/driver.rs:1808-1839 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/ai/agent_sdk/driver.rs#L1808-L1839) builds a run-scoped installation.
- [`app/src/ai/agent_sdk/driver.rs:2850-2923 @ b7ec0fc`](https://github.com/warpdotdev/warp/blob/b7ec0fc5572fb085ea1ac7837fc4c7ff1addb64b/app/src/ai/agent_sdk/driver.rs#L2850-L2923) injects that installation into Oz harness runs.
- Local `oz agent run` has no team scope argument. `oz agent run-cloud` uses `ObjectScope` while dispatching the cloud task, but the worker later attaches the built-in MCP from its run credential.

The hosted Factory MCP is intentionally stateless and its tools already own resource authorization:
- [`router/handlers/public_api/factory_mcp/server.go:1248-1308 @ f14defa`](https://github.com/warpdotdev/warp-server/blob/f14defad527d523d6a9650263182b5aa1751fbcf/router/handlers/public_api/factory_mcp/server.go#L1248-L1308) creates one stateless streamable-HTTP server.
- [`logic/factories.go:1947-2058 @ f14defa`](https://github.com/warpdotdev/warp-server/blob/f14defad527d523d6a9650263182b5aa1751fbcf/logic/factories.go#L1947-L2058) lists every accessible Factory and applies an explicit `team_uid` filter only when supplied.
- [`logic/factories.go:2096-2128 @ f14defa`](https://github.com/warpdotdev/warp-server/blob/f14defad527d523d6a9650263182b5aa1751fbcf/logic/factories.go#L2096-L2128) authorizes a `factory_uid` with object permissions.
- [`logic/team_scope.go:12-54 @ f14defa`](https://github.com/warpdotdev/warp-server/blob/f14defad527d523d6a9650263182b5aa1751fbcf/logic/team_scope.go#L12-L54) authorizes a `team_uid` for user and service-account principals.

The route currently installs `SetBillingMetadataAndTeam` without `SetActiveTeamFromHeader`:
- [`router/handlers/public_api/factory_mcp_routes.go:37-71 @ f14defa`](https://github.com/warpdotdev/warp-server/blob/f14defad527d523d6a9650263182b5aa1751fbcf/router/handlers/public_api/factory_mcp_routes.go#L37-L71) defines the middleware chain.
- [`router/middleware/team_selection.go:13-48 @ f14defa`](https://github.com/warpdotdev/warp-server/blob/f14defad527d523d6a9650263182b5aa1751fbcf/router/middleware/team_selection.go#L13-L48) validates the optional team header when a route opts into it.
- [`router/middleware/billing_metadata.go:14-48 @ f14defa`](https://github.com/warpdotdev/warp-server/blob/f14defad527d523d6a9650263182b5aa1751fbcf/router/middleware/billing_metadata.go#L14-L48) otherwise resolves the principal's fallback team.

## Decision
Keep the built-in Factory MCP connection cross-team and derive scope from each tool's explicit `factory_uid` or `team_uid`.

### Option A — request-scoped transport decoration
- Advantages:
  - Matches request-local team headers used by ordinary Warp APIs.
  - Can align connection admission metadata with an initiating window or run.
- Disadvantages:
  - The current transport stores default headers on one shared `reqwest::Client`.
  - The current peer and reconnection maps are keyed only by installation UUID.
  - Correct support requires a per-scope peer pool or a new dynamic HTTP transport API.
  - Every tool call, retry, reconnect, resource read, and tool-list refresh must carry the same captured scope.
  - A team-scoped connection conflicts with cross-team tools such as `list_factories` and `search_task`.
  - A header can disagree with a tool's `factory_uid` or `team_uid`, which creates two authorization and billing scopes for one operation.

### Option B — cross-team connection with tool-local scope
- Advantages:
  - Matches the hosted Factory MCP's cross-team discovery and explicit identifiers.
  - Reuses existing Factory and team authorization paths.
  - Keeps one interactive connection safe across concurrent windows.
  - Keeps local and cloud driver attachment behavior identical.
  - Avoids adding a second implicit scope that can disagree with tool input.
- Disadvantages:
  - The active Warp window does not implicitly filter Factory discovery.
  - The route's existing fallback team remains incidental middleware metadata.

Option B wins because Factory MCP is a control-plane surface, not an operation on the current terminal's team. Explicit tool scope is already the server's authority. A connection-level team would add ambiguity without removing any tool-level authorization.

## Proposed changes
### Warp client
1. Preserve the built-in installation API as bearer-token-only.
   - Keep `factory_mcp_installation` and `factory_mcp_installation_for_server_root` free of a team argument.
   - Update their documentation to state that omission of `X-Warp-Team-Uid` is the cross-team contract.
   - Do not read `UserWorkspaces`, `WindowId`, `TeamContext`, or `RequestTeamScope` from `builtin.rs`.

2. Preserve one interactive built-in installation.
   - `TemplatableMCPServerManager` continues to key it by `FACTORY_MCP_INSTALLATION_UUID`.
   - `UserWorkspacesEvent::WindowTeamChanged` must not restart or mutate the built-in server.
   - GUI and TUI use the same behavior because both use `sync_builtin_servers`.

3. Preserve run-scoped driver attachment.
   - `AgentDriver::builtin_factory_mcp_for_run` continues to accept credentials and server-name collisions only.
   - Do not infer a local run team from membership order or an interactive window.
   - Do not derive a cloud header from client workspace metadata. Service-account scope is enforced by the authenticated principal on the server.

4. Make the invariant executable in tests.
   - Parse the resolved built-in configuration and assert that its header set is exactly `Authorization`.
   - Assert that Firebase and API-key credentials produce the same no-team-header contract.
   - Assert that interactive synchronization does not respawn on per-window team changes.
   - Assert that driver injection uses the same installation contract.

### warp-server
1. Do not add `SetActiveTeamFromHeader` to `RegisterFactoryMCPRoutes` for this design.
   - The built-in client does not send the header.
   - Factory tool handlers must continue to authorize explicit Factory and team identifiers.
   - The current `SetBillingMetadataAndTeam` fallback is compatibility metadata. It must not become the authority for Factory access or spawned-run ownership.

2. Add route-level conformance tests in a separate `warpdotdev/warp-server` PR.
   - An absent `X-Warp-Team-Uid` preserves cross-team Factory discovery.
   - A valid or invalid team header does not override a tool's explicit `factory_uid` or `team_uid`.
   - A non-member cannot access a Factory or team through either a header or a tool argument.
   - A service account cannot access a Factory outside its pinned team.

3. Keep the shared header middleware tests.
   - `SetActiveTeamFromHeader` must continue to reject an unknown or non-member team with `403`.
   - Routes that do opt into header scoping must continue to run `SetActiveTeamFromHeader` before `SetBillingMetadataAndTeam`.
   - These tests are a guard for other endpoints. They do not opt Factory MCP into connection-level team scope.

### Required contingency if Option A is approved instead
Option A is not a small edit to `builtin.rs`. Before any client sends the header:
1. The client must capture a membership-aware `RequestTeamScope` from the initiating terminal surface or run.
2. The transport must carry that immutable scope through tool calls, resource reads, retries, and reconnects without mutating process-global state.
3. An ambiguous local run must omit the header.
4. A cloud user run must use a server-resolved run owner. A service-account run must use its pinned team.
5. The server route must install `SetActiveTeamFromHeader` before `SetBillingMetadataAndTeam`.
6. The server must reject header and tool-target disagreement.
7. Tests must cover two concurrent windows with different teams, header inclusion and omission, invalid membership, service-account pinning, and middleware ordering.

## Testing and validation
### Warp client
- `cargo nextest run -p warp -E 'test(factory_installation_resolves_to_a_preauthenticated_http_server) | test(builtin_factory_mcp)'`
  - Covers PRODUCT §2, §7, and §8.
  - Extend `factory_installation_resolves_to_a_preauthenticated_http_server` to assert the complete header set and explicit omission of `X-Warp-Team-Uid`.
  - Extend the driver tests to inspect the resolved transport headers, not only the installation UUID and name.
- Add focused manager tests for PRODUCT §3:
  - Register two windows with different teams.
  - Synchronize the built-in server once.
  - Switch one window's team.
  - Assert that the built-in installation count, UUID, and header set do not change.
- Run `./script/presubmit` after focused tests pass.

### warp-server
- `go test ./router/middleware -run 'TestSetActiveTeamFromHeader|TestTeamSelection'`
  - Preserves invalid-membership rejection and ordering for routes that use header scope.
- `go test ./router/handlers/public_api/... -run 'TestFactoryMCP.*(Team|Scope|Header)'`
  - Add tests for cross-team discovery, explicit tool authorization, ignored connection-header scope, and service-account pinning.
- `go test ./router/middleware ./router/handlers/public_api/...`
  - Final focused server validation.

No visual proof is required because the change has no rendered UI.

## Parallelization
Use two implementation workstreams after this spec is approved:
- **warp-client** — local worktree `../warp-gh-15608-client`, branch `factory/gh-15608-factory-mcp-team-scope`, owns `app/src/ai/mcp/builtin.rs`, interactive manager tests, and driver tests. It updates this spec PR.
- **warp-server-tests** — local worktree `../warp-server-gh-15608-scope-tests`, branch `factory/gh-15608-factory-mcp-scope-tests`, owns Factory MCP route conformance tests and any test seams needed to observe the middleware contract. It opens a separate draft PR because it is a different repository.

The two workstreams can run in parallel after approval. The client work must not add a team header. The server work must not opt the route into `SetActiveTeamFromHeader`. Each workstream runs its repository's focused tests. Final review verifies both PRs against PRODUCT §1–§12.

## Assumptions
- Factory MCP protocol requests do not themselves consume billable AI inference. The Factory or team selected by a tool determines ownership of work the tool creates.
- The selected team in a Warp window is a UI and request-policy scope. It is not a default Factory selection.
- The hosted Factory MCP remains stateless.
- Existing Factory and team logic remains the authorization source for tool inputs.

## Out of scope
- Implement a dynamic-header hook in `rmcp` or `reqwest`.
- Split the process-wide MCP manager into one manager per window.
- Change Factory MCP tool schemas.
- Remove or redesign `SetBillingMetadataAndTeam` for all public API routes.
- Change non-Factory MCP servers.

## Risks and mitigations
- **A future cleanup adds the header because sibling clients use it.** Keep a full-header assertion in `builtin_tests.rs` and document the deliberate omission next to the constructor.
- **A tool starts trusting middleware fallback instead of explicit scope.** Route conformance tests compare header values with explicit tool targets and require tool-level authorization.
- **Service-account access widens through cross-team discovery.** Test a service account against a Factory outside its pinned team.
- **The route fallback team is mistaken for billing authority.** Keep spawned-run ownership assertions tied to the resolved Factory, team, environment, or foreman identity.
