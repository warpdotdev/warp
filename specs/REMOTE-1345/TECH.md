# REMOTE-1345: MCP Servers for Third-Party Harnesses

## Context

MCP server setup in the agent driver is gated on `HarnessKind::Oz` (`driver.rs:1258-1303`). Third-party harnesses (Claude Code, Codex, Gemini) receive zero MCP servers even when the user specifies them via `--mcp` or environment config. File-based MCP discovery (`driver.rs:1325-1341`) is also Oz-only.

The user's MCP config arrives as `Task.mcp_specs: Vec<MCPSpec>` (`driver.rs:336`), where each spec is either a `MCPSpec::Uuid` (reference to an installed server) or `MCPSpec::Json` (inline JSON config). For the Oz harness, `resolve_mcp_specs` (`driver.rs:826`) splits these into UUID refs and ephemeral `TemplatableMCPServerInstallation` objects, which are then spawned as child processes (stdio) or HTTP clients by `TemplatableMCPServerManager`.

Third-party harnesses have their own MCP client implementations:
- **Claude Code** reads `.mcp.json` or accepts `--mcp-config <json-file>` with `{ "mcpServers": { name: { command, args, env } } }` format.
- **Codex** reads `~/.codex/config.toml` with `[mcp_servers.name]` sections: `command`/`args`/`env` (stdio) or `url`/`bearer_token_env_var` (HTTP).

### Key files
- `app/src/ai/agent_sdk/driver.rs:1258-1303` — Oz-only MCP setup block
- `app/src/ai/agent_sdk/driver.rs:826-858` — `resolve_mcp_specs`
- `app/src/ai/agent_sdk/driver/harness/mod.rs:83-91` — `prepare_environment_config` trait method
- `app/src/ai/agent_sdk/driver/harness/claude_code.rs:66-78` — Claude impl
- `app/src/ai/agent_sdk/driver/harness/codex.rs:50-62` — Codex impl
- `app/src/ai/mcp/parsing.rs:382-397` — `resolve_json` (template → final JSON)
- `app/src/ai/mcp/templatable_installation.rs:113-156` — `apply_secrets`
- `app/src/ai/mcp/mod.rs:178-228` — `JSONTransportType`, `JSONMCPServer`
- `app/src/ai/agent_sdk/driver/environment.rs:289-300` — single-repo `cd` into repo subdir

## Proposed changes

### Approach: let each harness spawn its own MCP servers

Write resolved MCP configs into each harness's native config format before the CLI launches. The harness CLI spawns/connects to servers itself. This avoids multiplexing stdio pipes and keeps MCP lifecycle owned by the harness.

### 1. Resolve MCP specs to `HashMap<String, JSONMCPServer>` on the foreground spawner

New helper `resolve_mcp_specs_to_json` on `AgentDriver` (`driver.rs:788`), runs on the foreground spawner (model context required):
- Delegates to existing `resolve_mcp_specs` to split into `(existing_uuids, ephemeral_installations)`.
- For UUIDs: `TemplatableMCPServerManager::get_installed_server(uuid)` → clone → `apply_secrets` → `resolve_json` → `serde_json::from_str::<HashMap<String, JSONMCPServer>>`.
- For ephemeral: `apply_secrets` → `resolve_json` → `serde_json::from_str`.
- Collect all into `HashMap<String, JSONMCPServer>`.

Called in `prepare_harness` before `build_runner`. `prepare_harness` is only called from the `HarnessKind::ThirdParty` branch; the Oz path uses its own `resolve_mcp_specs` → `start_mcp_servers` unchanged.

### 2. Remove `prepare_environment_config` from the trait; fold config setup into `build_runner`

The `prepare_environment_config` method was removed from `ThirdPartyHarness` entirely. Each harness now does its config setup (auth, trust, onboarding, MCP) inside `build_runner`, which already receives all the context it needs.

I did this because we started to split handling for environment config across `build_runner` and `prepare_environment_config` depending on the harness, which resulted in us passing the same parameters
to both functions for all harnesses, but most harnesses would only use the parameters in one function or the other. `build_runner` and `prepare_environment_config` are always called
back-to-back.

`build_runner` signature gained two new params:
- `resolved_env_vars: &HashMap<OsString, OsString>` (previously passed to `prepare_environment_config`)
- `resolved_mcp_servers: &HashMap<String, JSONMCPServer>`

This consolidates per-run setup (temp files, MCP config) and static config (auth, trust) into one method per harness. Gemini currently ignores `resolved_mcp_servers`.

### 3. Claude Code: write MCP config in `ClaudeHarnessRunner::new`, pass via `--mcp-config`

Inside `build_runner` → `ClaudeHarnessRunner::new`:
- `prepare_claude_environment_config` is called first for static config (auth, trust, onboarding) — unchanged from before, just moved into `build_runner`.
- If `resolved_mcp_servers` is non-empty, `serialize_claude_mcp_config` produces `{ "mcpServers": { name: config } }` JSON.
- Written to a `NamedTempFile` with `.json` suffix, stored as `_temp_mcp_config_file` on the runner so it lives until the CLI exits.
- `claude_command()` gained a `mcp_config_path: Option<&str>` param; when present, appends `--mcp-config '<path>'`.

New types for serialization: `ClaudeMcpConfig` (wrapper with `mcp_servers` field) and `ClaudeMcpServerEntry` (tagged enum: `Stdio { command, args, env }` / `Http { url, headers }`).

Using `--mcp-config` instead of writing `.mcp.json` avoids a cwd problem: in the single-repo case, `environment.rs` does `cd_in_terminal(repo_name)` so the terminal cwd is `{working_dir}/{repo}`, not `working_dir`. Claude discovers `.mcp.json` relative to its cwd, so a file at `working_dir/.mcp.json` wouldn't be found. The flag is path-independent and also avoids conflicts with repo-owned `.mcp.json` files.

### 4. Codex: write `[mcp_servers]` entries to `config.toml`

`prepare_codex_environment_config` (now called inside `build_runner`) passes `resolved_mcp_servers` through to `prepare_codex_config_toml`, which calls the new `write_codex_mcp_servers` helper:
- `CLIServer { command, args, env }` → `[mcp_servers.name]` with `command`, `args`, `env` fields.
- `SSEServer { url, headers }` → `[mcp_servers.name]` with `url` and `http_headers` (inline table) fields.

Codex config is global (`~/.codex/config.toml`), not cwd-relative, so no path issues.

### 5. Transport mapping

Warp's `JSONTransportType` maps to each harness as follows:

**`CLIServer { command, args, env, working_directory }`:**
- Claude Code: `{ "type": "stdio", "command": "...", "args": [...], "env": {...} }`
- Codex: `command = "..."`, `args = [...]`, `env = { ... }` under `[mcp_servers.name]`

**`SSEServer { url, headers }`:**
- Claude Code: `{ "type": "http", "url": "...", "headers": {...} }`
- Codex: `url = "..."`, `http_headers = { ... }` under `[mcp_servers.name]`

## Testing and validation

1. **Unit tests for Claude MCP config serialization** (`claude_code_tests.rs`) — `serialize_claude_mcp_config_cli_server` and `serialize_claude_mcp_config_sse_server` verify `HashMap<String, JSONMCPServer>` produces valid `--mcp-config` JSON for both transport types.
2. **Unit tests for Codex MCP config serialization** (`codex_tests.rs`) — `write_codex_mcp_servers_cli_server` and `write_codex_mcp_servers_sse_server` verify `HashMap<String, JSONMCPServer>` produces valid `[mcp_servers]` TOML entries.
3. **Existing test updates** — `claude_command` tests, `prepare_codex_config_toml` tests updated for new function signatures.
4. **Regression** — Oz harness MCP setup unchanged (existing tests cover this).

## Follow-ups

- **File-based MCP discovery for Codex**: Claude Code auto-discovers `.mcp.json` in cloned repos. Codex would need discovered configs translated to `config.toml` format.
- **Profile MCP servers**: `start_profile_mcp_servers` resolves UUID-based servers from the profile allowlist. Same resolution path as `task.mcp_specs` — could be included in a follow-up.
- **Gemini MCP support**: add when Gemini CLI supports MCP configuration.
