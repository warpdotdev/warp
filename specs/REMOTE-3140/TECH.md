# Auto-attach Factory MCP to third-party harness runs
## Summary
[REMOTE-3140](https://linear.app/warpdotdev/issue/REMOTE-3140/auto-attach-factory-mcp-to-third-party-harness-cloud-runs) gives Claude Code and Codex runs launched by the Warp driver the same authenticated `warp-factory` MCP server and bootstrap skill that the Oz harness receives. The change is client-only. It does not store the parent credential in server-owned run configuration.

## Product behavior
1. With `FactoryMcp` enabled and usable parent credentials, a Claude Code or Codex run receives `warp-factory` without an explicit `--mcp` entry.
2. The server URL is `{server_root}/api/v1/mcp/factory`. The request header is `Authorization: Bearer <parent credential>`.
3. An explicit MCP server named `warp-factory` wins. Warp does not add or overwrite the built-in server.
4. A disabled flag or missing usable credentials skips the built-in server. The run and its other MCP servers continue normally.
5. Claude Code and Codex can discover the existing `factory-mcp` bootstrap skill through their native filesystem skill roots.
6. The bootstrap skill reads `skill://warp/factory-mcp/SKILL.md` when the harness exposes MCP resource reads. If it does not, the skill uses the Factory MCP server instructions and live tool descriptions as the fallback and does not claim that the full canonical workflow was loaded.

## Technical design
### Current paths
- [`app/src/ai/agent_sdk/driver/mcp_startup.rs (164-216) @ 5a6ded1e`](https://github.com/warpdotdev/warp/blob/5a6ded1e8413badee286363cdcc0ec5e3a1b4373/app/src/ai/agent_sdk/driver/mcp_startup.rs#L164-L216) resolves only explicit `MCPSpec` values into third-party harness JSON.
- [`app/src/ai/agent_sdk/driver/mcp_startup.rs (383-416) @ 5a6ded1e`](https://github.com/warpdotdev/warp/blob/5a6ded1e8413badee286363cdcc0ec5e3a1b4373/app/src/ai/agent_sdk/driver/mcp_startup.rs#L383-L416) already owns the shared Factory MCP eligibility and explicit-name precedence in `builtin_factory_mcp_for_run`.
- [`app/src/ai/agent_sdk/driver.rs (3061-3096) @ 5a6ded1e`](https://github.com/warpdotdev/warp/blob/5a6ded1e8413badee286363cdcc0ec5e3a1b4373/app/src/ai/agent_sdk/driver.rs#L3061-L3096) passes the resolved map to each third-party harness.
- [`app/src/ai/mcp/builtin.rs (39-103) @ 5a6ded1e`](https://github.com/warpdotdev/warp/blob/5a6ded1e8413badee286363cdcc0ec5e3a1b4373/app/src/ai/mcp/builtin.rs#L39-L103) defines the stable name, bearer selection, URL, and authenticated installation.
- [`resources/bundled/skills/factory-mcp/SKILL.md @ 5a6ded1e`](https://github.com/warpdotdev/warp/blob/5a6ded1e8413badee286363cdcc0ec5e3a1b4373/resources/bundled/skills/factory-mcp/SKILL.md) is a thin bootstrap. Oz lists it through `SkillManager`; the canonical, server-versioned workflow and references remain MCP resources.
- [`app/src/ai/agent_sdk/driver/harness/skill_dirs_publish.rs @ 5a6ded1e`](https://github.com/warpdotdev/warp/blob/5a6ded1e8413badee286363cdcc0ec5e3a1b4373/app/src/ai/agent_sdk/driver/harness/skill_dirs_publish.rs) publishes `WARP_SKILL_DIRS` skills into harness-native roots, but does not publish bundled skills.

### MCP resolution
Update `AgentDriver::resolve_mcp_specs_to_json` in `mcp_startup.rs`:

1. Resolve and serialize the explicit installations as today.
2. Read the parent client `Credentials` from `AuthStateProvider` on the driver foreground context. Do not read a token from task data, managed secrets, or environment variables.
3. Build the taken-name set from the keys of the explicit resolved map. This uses the exact names the harness will receive.
4. Call `builtin_factory_mcp_for_run(credentials, taken_names)`.
5. When it returns an installation, pass that installation through `mcp_installations_to_json` and add it to the resolved map.
6. Return the resolved map. Harness setup treats built-in and explicit servers identically after exact-name precedence has been resolved.

Do not gate this code on `task_id`, sandbox detection, or local/cloud execution mode. Any run that reaches the third-party Warp driver follows the same code. The existing feature, credential, and collision checks decide whether attachment occurs.

### Auth and security
- Use `builtin::builtin_bearer_token` through `builtin_factory_mcp_for_run`. Cloud runs therefore use the parent Warp API key. Local Firebase credentials retain the existing pinned-token behavior.
- Put the bearer only in the ephemeral resolved MCP map and the harness-native runtime configuration that consumes it.
- Do not add Factory MCP to `AgentConfigSnapshot.MCPServers`, mutate `Task.mcp_specs`, upload it as a managed secret, include it in the prompt, or emit it in logs and setup events.
- Claude Code writes a temporary `--mcp-config` that is deleted with its runner.
- Every Codex run gets a unique owner-only temporary `CODEX_HOME`. Warp seeds its `config.toml` and `auth.json` from the user's persistent Codex home, exposes the verified persistent plugin directory, and then writes the run's resolved MCP map and auth overrides only into that isolated home. Each resolved MCP server replaces its complete seeded server table so stale transport fields or `enabled = false` cannot alter the run configuration.
- The Codex command receives the isolated path through `CODEX_HOME`, so simultaneous built-in and explicit exact-name runs cannot overwrite each other's configuration. Fresh and resumed transcripts are created in that run home's `sessions` tree and uploaded from there.
- Codex cleanup removes the complete temporary home on every cleanup disposition. `TempDir` drop is the fallback when setup or execution exits before normal cleanup. The user's persistent `config.toml`, `auth.json`, plugins, and sessions are never mutated by driver setup or cleanup.
- A missing credential is fail-open for Factory MCP only. It must not fail harness preparation or remove explicit MCP servers.

### Factory skill and resources
Extend the shared publisher in `harness/skill_dirs_publish.rs` so harness setup can publish the feature-gated bundled `factory-mcp` directory in addition to `WARP_SKILL_DIRS` sources:

- Claude Code target: `<harness_working_dir>/.claude/skills/factory-mcp`.
- Codex target: `<harness_working_dir>/.agents/skills/factory-mcp`.
- Source: `<bundled_resources_dir>/bundled/skills/factory-mcp`.
- Reuse the existing symlink, Git exclude, repeat-run, and conflict handling. Do not copy or fork the skill.
- Process configured `WARP_SKILL_DIRS` sources first, so a configured `factory-mcp` skill keeps precedence over the bundled bootstrap.
- Keep the same `FactoryMcp` feature gate as Oz. Skill publication does not depend on credentials or on whether the built-in lost an MCP name collision; the bootstrap already verifies server availability before use.

Update the bootstrap text to define the resource fallback in Product behavior 6. Do not embed the canonical Factory workflow in Warp. The live MCP resources and tool schemas must remain server-owned.

## Supported harness matrix
- **Oz:** No behavior change. The driver starts the built-in server, `SkillManager` exposes the bundled bootstrap, and Warp's MCP resource reader reads the canonical resource.
- **Claude Code:** Attach HTTP MCP with `headers` through `--mcp-config`. Publish the bootstrap under `.claude/skills`. Use MCP resources when the installed Claude version exposes them; otherwise use the documented fallback.
- **Codex:** Attach HTTP MCP with `http_headers` in an owner-only, per-run `CODEX_HOME`, then remove the complete temporary home when the runner exits. Publish the bootstrap under `.agents/skills`. Use MCP resources when the installed Codex version exposes them; otherwise use the documented fallback.
- **Gemini:** Deferred. Its adapter currently ignores `resolved_mcp_servers` and has no equivalent skill publication path in this ticket.

## Decisions
- **Inject in the client, not the server.** The client already owns built-in eligibility and final harness serialization. Server injection would persist credentials in `AgentConfigSnapshot` and expose them in run MCP configuration.
- **Explicit configuration wins by exact server name.** This matches Oz and prevents Warp from replacing user intent. URL or UUID matching is not used.
- **Publish only the bootstrap skill.** Copying the full workflow into the client would drift from the live Factory tool catalog. The MCP resource stays authoritative.
- **Use native filesystem skills instead of a new cross-harness skill API.** Claude Code and Codex already discover these roots, and the existing publisher defines safe collision behavior.
- **Defer Gemini as a complete unit.** Adding only a skill without the MCP transport would advertise unavailable tools.

## Assumptions
- `FactoryMcp` remains the rollout switch for both the built-in server and bootstrap skill.
- The parent API key is valid for the lifetime of a cloud run. Token refresh is not added.
- Native harness MCP resource support can vary by installed version; server instructions and live tool descriptions are the intentional reduced-capability fallback.

## Out of scope
- Server, worker, protocol, or `AgentConfigSnapshot` changes.
- Gemini MCP and skill integration.
- OAuth refresh for long local runs.
- A generic publisher for every Warp bundled skill.
- Changes to Factory MCP tools, resources, or authorization policy.

## Validation criteria
Add focused tests under `app/src/ai/agent_sdk/driver`:

1. `resolve_mcp_specs_to_json` with no explicit specs, `FactoryMcp` enabled, and API-key credentials returns `warp-factory` with the expected server-root URL and exact bearer header.
2. The same path omits the built-in when the flag is off or credentials are absent and preserves every explicit server.
3. An explicit `warp-factory` entry is unchanged and no built-in entry overwrites it.
4. The resolved built-in serializes with the bearer header through `serialize_claude_mcp_config` and `write_codex_mcp_servers`.
5. Resolution leaves the input `MCPSpec` list unchanged. No snapshot, prompt, UI, or log assertion contains the API key.
6. Harness setup publishes the bundled bootstrap into the Claude and Codex native roots when the flag is on, omits it when off, is idempotent, and follows existing sandbox and non-sandbox collision behavior.
7. A bundled-skill test pins the canonical resource URI and the reduced-capability fallback text.
8. Codex gives each run a distinct temporary home with mode `0700` and writes credential-bearing `config.toml` with mode `0600` on Unix. Simultaneous built-in and explicit exact-name HTTP runs seeded from a disabled stdio `warp-factory` table retain independent MCP values without stale `command`, `args`, `env`, `cwd`, or `enabled` fields, while the persistent `config.toml` and `auth.json` remain byte-for-byte unchanged and verified plugins remain available.
9. Codex uses the isolated `sessions` tree for fresh transcript discovery and resume rehydration, removes the temporary home during cleanup, and relies on `TempDir` drop as the fallback.

Run:

- `cargo test -p warp --lib ai::agent_sdk::driver::mcp_startup::tests`
- `cargo test -p warp --lib ai::agent_sdk::driver::harness::claude_code::tests`
- `cargo test -p warp --lib ai::agent_sdk::driver::harness::codex::tests`
- `cargo test -p warp --lib ai::agent_sdk::driver::harness::skill_dirs_publish::tests`
- `cargo test -p warp --lib ai::skills::bundled::tests`

No visual proof is required. This work changes agent configuration and skill discovery, not rendered UI.
