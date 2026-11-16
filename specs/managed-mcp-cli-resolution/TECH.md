# Managed MCP Resolution for CLI Agent Runs - Tech Spec

## Context

This spec covers the Warp client work needed for CLI-based agent runs to consume managed MCP servers. The authoritative server-side behavior is documented in `../warp-server/specs/oauth-managed-mcp`; this client spec focuses on how `agent run --mcp` resolves a `warp_id` that may refer to a managed MCP installation and turns it into runtime MCP config without persisting resolved secrets, proxy headers, or rendered command config. There is intentionally no sibling `PRODUCT.md` for this spec.

References were inspected at:

- `warp` commit `1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10`
- `warp-server` commit `b2149361735cc756d54f26522287e79200f37f07`

Relevant current client flow:

- [`crates/warp_cli/src/mcp.rs:14-24 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/crates/warp_cli/src/mcp.rs#L14-L24) defines `MCPSpec` as either `Uuid` or raw JSON, and [`crates/warp_cli/src/mcp.rs:51-70 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/crates/warp_cli/src/mcp.rs#L51-L70) parses UUID-looking `--mcp` values before falling back to file/inline JSON.
- [`app/src/ai/agent_sdk/mcp_config.rs:7-60 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/ai/agent_sdk/mcp_config.rs#L7-L60) builds the `mcp_servers` map sent to the ambient-agent API. UUID specs become `{"warp_id": "<uuid>"}`; JSON specs are unpacked and validated.
- [`app/src/ai/agent_sdk/config_file.rs:99-136 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/ai/agent_sdk/config_file.rs#L99-L136) converts persisted `mcp_servers` back into runtime `MCPSpec`s. Any entry with `warp_id` becomes `MCPSpec::Uuid`, so the driver sees the same UUID shape whether the input came from CLI, config file, or server task metadata.
- [`app/src/ai/agent_sdk/driver.rs:971-1045 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/ai/agent_sdk/driver.rs#L971-L1045) resolves runtime MCP specs. UUIDs are treated only as local templatable MCP installations today; JSON specs become ephemeral installations.
- [`app/src/ai/agent_sdk/driver.rs:1310-1387 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/ai/agent_sdk/driver.rs#L1310-L1387) starts local installed MCPs separately from ephemeral MCPs.
- [`app/src/ai/agent_sdk/driver.rs:1897-1942 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/ai/agent_sdk/driver.rs#L1897-L1942) wires the Oz harness startup path through that split.
- [`app/src/ai/agent_sdk/driver.rs:2440-2526 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/ai/agent_sdk/driver.rs#L2440-L2526) resolves MCP specs for third-party harnesses into `JSONMCPServer` values after applying local secrets.
- [`app/src/ai/mcp/parsing.rs:256-374 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/ai/mcp/parsing.rs#L256-L374) parses user JSON into templatable MCP installations. It only derives install variable values from `env` and `headers`; placeholders elsewhere in the config become template variables without values and currently produce `MCPMissingVariables`.
- [`app/src/ai/mcp/templatable_installation.rs:100-155 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/ai/mcp/templatable_installation.rs#L100-L155) applies local raw managed secrets to MCP install variables, including explicit `{{secret_name}}` references.
- [`app/src/server/server_api.rs:1630-1680 @ 1cdb4794`](https://github.com/warpdotdev/warp/blob/1cdb4794e6d30f9c60d76167ec9b4ee823bc6a10/app/src/server/server_api.rs#L1630-L1680) exposes domain-specific GraphQL-backed clients from `ServerApiProvider`; managed MCP should follow this pattern rather than passing raw `ServerApi` around new call sites.

Relevant server contract:

- [`graphql/v2/mutations/managed_mcp.graphqls:69-77 @ b2149361`](https://github.com/warpdotdev/warp-server/blob/b2149361735cc756d54f26522287e79200f37f07/graphql/v2/mutations/managed_mcp.graphqls#L69-L77) defines `CreateManagedMcpClientConfigOutput` with `transportKind`, `mcpConfigJson`, proxy fields, and expiry.
- [`graphql/v2/mutations/managed_mcp.graphqls:119-122 @ b2149361`](https://github.com/warpdotdev/warp-server/blob/b2149361735cc756d54f26522287e79200f37f07/graphql/v2/mutations/managed_mcp.graphqls#L119-L122) exposes the `createManagedMcpClientConfig` mutation.
- [`logic/ai/ambient_agents/managed_mcp/client_config.go:26-30 @ b2149361`](https://github.com/warpdotdev/warp-server/blob/b2149361735cc756d54f26522287e79200f37f07/logic/ai/ambient_agents/managed_mcp/client_config.go#L26-L30) states the use-time contract: URL-backed rows use proxy sessions, command-backed rows raw-render command config while preserving managed-secret placeholders for local client resolution.
- [`logic/ai/ambient_agents/managed_mcp/client_config.go:51-74 @ b2149361`](https://github.com/warpdotdev/warp-server/blob/b2149361735cc756d54f26522287e79200f37f07/logic/ai/ambient_agents/managed_mcp/client_config.go#L51-L74) implements URL vs command behavior.
- [`logic/ai/ambient_agents/managed_mcp/client_config.go:92-110 @ b2149361`](https://github.com/warpdotdev/warp-server/blob/b2149361735cc756d54f26522287e79200f37f07/logic/ai/ambient_agents/managed_mcp/client_config.go#L92-L110) returns canonical portable MCP JSON in the `{"mcpServers": {...}}` shape.

The core client bug today is that a managed MCP `warp_id` reaches `AgentDriver` as a UUID, then fails as `MCPServerNotFound` unless a local templatable installation with the same UUID exists. The client needs a runtime fallback that asks the server to resolve that UUID as a managed MCP installation.

## Proposed Changes

### GraphQL schema and client API

1. Sync `crates/warp_graphql_schema/api/schema.graphql` with the server schema needed for `createManagedMcpClientConfig`.
2. Add `crates/graphql/src/api/mutations/create_managed_mcp_client_config.rs` with Cynic types for:
   - `CreateManagedMcpClientConfigVariables`
   - `CreateManagedMcpClientConfigInput { uid: cynic::Id }`
   - `CreateManagedMcpClientConfig`
   - `CreateManagedMcpClientConfigOutput`
   - `CreateManagedMcpClientConfigResult`
   - `ManagedMcpTransportKind`
3. Register the mutation module in `crates/graphql/src/api/mutations/mod.rs`.
4. Add `app/src/server/server_api/managed_mcp.rs` with a `ManagedMcpClient` trait:
   - `async fn create_managed_mcp_client_config(&self, uid: uuid::Uuid) -> anyhow::Result<ManagedMcpClientConfigOutput>`
   - map `UserFacingError` through `get_user_facing_error_message`
   - return an explicit error for `Unknown` so schema drift is visible.
5. Add `pub mod managed_mcp;` and `ServerApiProvider::get_managed_mcp_client()` returning `Arc<dyn ManagedMcpClient>`.

### Runtime resolution model

Replace `AgentDriver::resolve_mcp_specs` and `resolve_mcp_specs_to_json` with a managed-aware resolver. Keep the current local/ephemeral split, but make it capable of async GraphQL calls.

Recommended domain shape:

```rust
struct ResolvedMcpSpecs {
    local_uuids: Vec<Uuid>,
    ephemeral_installations: Vec<TemplatableMCPServerInstallation>,
}
```

Resolution rules:

1. Parse `MCPSpec::Json` exactly as today and append resulting ephemeral installations.
2. For each `MCPSpec::Uuid`:
   - If `TemplatableMCPServerManager::get_installed_server(&uuid).is_some()`, append it to `local_uuids`.
   - Otherwise call `ManagedMcpClient::create_managed_mcp_client_config(uuid)`.
   - Parse returned `mcpConfigJson` as portable MCP config and append it as ephemeral installation(s).
3. Preserve local-first behavior. This avoids changing existing offline/local MCP flows and keeps backwards compatibility with UUIDs that refer to local templatable MCP installs.
4. Do not write the returned `mcpConfigJson`, proxy token, authorization header, or rendered command/env values back to `AgentConfigSnapshot`, task config, config files, or logs.
5. Treat GraphQL user-facing failures as fatal setup errors for that managed UUID. Add:

```rust
#[error("Failed to resolve managed MCP server {uid}: {message}")]
ManagedMcpResolutionFailed { uid: Uuid, message: String }
```

to `AgentDriverError`.

### Managed returned config parsing

The server returns `mcpConfigJson` in the same portable wrapper shape the client already accepts for user JSON. That should be parsed through the same MCP templating machinery where possible, but managed config requires one additional helper because command-backed managed installs may preserve placeholders outside `env` and `headers`.

Add a helper near the existing MCP parsing/resolution code:

```rust
fn installations_from_managed_client_config_json(
    json: &str,
) -> Result<Vec<TemplatableMCPServerInstallation>, AgentDriverError>
```

Behavior:

1. Normalize and parse the JSON using `ParsedTemplatableMCPServerResult::from_user_json`.
2. If `templatable_mcp_server_installation` is present, use it.
3. If it is missing because the template has variables that were not captured from env/headers, preserve any captured env/header variable values and synthesize only missing variable values as `VariableValue { variable_type: Text, value: format!("{{{{{key}}}}}") }`.
4. Construct a `TemplatableMCPServerInstallation` from the parsed template, preserved captured values, and synthesized missing values.
5. Let the existing `apply_secrets` step resolve `{{secret_name}}` placeholders against local/task raw managed secrets before `resolve_json`.

This keeps hardcoded JSON and managed JSON on the same launch path while filling the parser gap for managed command args such as `["--token={{API_TOKEN}}"]`.

### Oz harness path

Update the Oz setup section to resolve MCP specs asynchronously before startup:

1. Retrieve `Arc<dyn ManagedMcpClient>` from `ServerApiProvider`.
2. Spawn the resolver with access to:
   - raw `task.mcp_specs`
   - `TemplatableMCPServerManager`
   - managed MCP client
3. Start `ResolvedMcpSpecs.local_uuids` through `start_mcp_servers`.
4. Start `ResolvedMcpSpecs.ephemeral_installations` through `start_ephemeral_mcp_servers`.
5. Preserve existing strict/degraded MCP startup behavior after resolution. Resolution failures are not degraded startup; they are invalid setup because the requested MCP could not be materialized.

### Third-party harness path

Update `prepare_harness` so the third-party path uses the same managed-aware resolver:

1. Resolve MCP specs into `ResolvedMcpSpecs`.
2. For `local_uuids`, load each local installation from `TemplatableMCPServerManager`, apply secrets, render JSON, deserialize to `JSONMCPServer`, and extend the harness config map.
3. For managed/inline ephemeral installations, apply secrets, render JSON, deserialize to `JSONMCPServer`, and extend the harness config map.
4. Duplicate names should continue to behave like existing map extension semantics unless the implementation finds an existing helper that can produce better diagnostics without changing current behavior.

### Preserved invariants

- `agent run --mcp <uuid>` still stores and sends `{"warp_id":"<uuid>"}` in agent/task config.
- Managed MCP resolution is use-time only and run-scoped.
- Existing direct JSON `command` and `url` configs behave unchanged.
- Existing local templatable MCP UUIDs behave unchanged.
- Server-side task launches that pass stored `warp_id` values through CLI automatically get the same resolution path because `mcp_specs_from_mcp_servers` already converts `warp_id` entries into `MCPSpec::Uuid`.

## Testing and Validation

Add targeted unit tests around the new helpers and driver resolution path:

1. `build_mcp_servers_from_specs` and `mcp_specs_from_mcp_servers` continue to serialize/deserialize UUID specs as `warp_id`; update existing tests only if schema names move.
2. Managed resolver with a locally installed UUID returns that UUID in `local_uuids` and does not call `ManagedMcpClient`.
3. Managed resolver with a non-local UUID calls `create_managed_mcp_client_config` and turns returned command config into an ephemeral installation.
4. Managed command config with env placeholder:
   - input `{"mcpServers":{"GitHub MCP":{"command":"npx","env":{"API_TOKEN":"{{API_TOKEN}}"}}}}`
   - with secret `API_TOKEN=real`
   - rendered JSON contains `API_TOKEN=real`.
5. Managed command config with arg placeholder:
   - input `{"mcpServers":{"GitHub MCP":{"command":"npx","args":["--token={{API_TOKEN}}"]}}}`
   - with secret `API_TOKEN=real`
   - rendered JSON contains `--token=real`.
6. Managed URL config returned from server:
   - input includes proxy `url` and `Authorization` header
   - third-party harness JSON contains the proxy URL and header value unchanged.
7. Missing managed secret leaves the placeholder in the rendered command config, matching existing `apply_secrets` behavior.
8. GraphQL `UserFacingError` maps to `AgentDriverError::ManagedMcpResolutionFailed` with the UID and message.
9. `Unknown` GraphQL response also fails with a clear managed MCP resolution error, not `MCPServerNotFound`.

Run targeted validation:

- `cargo test -p warp_graphql`
- `cargo test -p warp mcp_config_tests`
- `cargo test -p warp driver_tests`
- `cargo check -p warp` if Cynic/schema changes touch generated schema usage broadly.

Manual smoke validation once implementation exists:

1. Use `agent run --mcp <managed-command-uid>` with a command-backed managed MCP whose env/args reference a managed secret. Confirm the MCP starts and the secret is substituted locally.
2. Use `agent run --mcp <managed-url-uid>` with a URL-backed managed MCP. Confirm the server mints proxy config and the local run can see the MCP tools.
3. Use a local templatable MCP UUID. Confirm no managed GraphQL resolution is attempted and the MCP starts as before.
4. Start from an existing server-side task whose stored config has `mcp_servers.<name>.warp_id`. Confirm the CLI worker path resolves the same way as direct CLI input.

## Parallelization

Parallelization is optional for implementation but not useful for this spec itself. If implementation is split, use two local agents in separate worktrees so schema/client changes and runtime driver changes can progress independently:

- Agent A: GraphQL client wiring
  - Execution mode: local.
  - Worktree: `/Users/bens/Desktop/warp-managed-mcp-graphql`.
  - Branch: `managed-mcp-cli-resolution-graphql`.
  - Owns `crates/warp_graphql_schema`, `crates/graphql`, and `app/src/server/server_api/managed_mcp.rs`.
- Agent B: Driver/runtime resolution
  - Execution mode: local.
  - Worktree: `/Users/bens/Desktop/warp-managed-mcp-driver`.
  - Branch: `managed-mcp-cli-resolution-driver`.
  - Owns `app/src/ai/agent_sdk/driver.rs`, MCP parsing helpers, and driver tests.

Merge Agent A first because Agent B needs the `ManagedMcpClient` trait and GraphQL output type. Land as one combined PR after both branches are reconciled and targeted tests pass.

## Risks and Mitigations

- **Secret leakage:** server-returned config can contain proxy headers or unresolved placeholders. Keep all resolution in memory, avoid logging `mcpConfigJson`, and do not write resolved config back to snapshots or config files.
- **Parser mismatch:** managed command configs can contain placeholders outside env/headers. Add the managed-specific synthesis helper instead of weakening the general user-json parser behavior.
- **UUID ambiguity:** a UUID could theoretically exist both locally and as a managed MCP. Resolve local first to preserve current behavior and avoid surprising users with network-dependent resolution for existing local installs.
- **Proxy token expiry:** this change mints proxy config once per run. Do not add token refresh in v1; document failures as MCP startup/runtime failures and revisit if long-running managed URL MCPs need refresh.

## Follow-ups

- Consider a future `MCPSpec::ManagedUuid` or config-level discriminator only if local-first UUID ambiguity becomes a real issue.
- Consider shared parsing utilities between user JSON and managed returned config if more server-produced MCP config shapes appear.
