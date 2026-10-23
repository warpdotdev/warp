# Tech Spec: Support for Agent Client Protocol (ACP)

**Issue:** [warpdotdev/warp#7326](https://github.com/warpdotdev/warp/issues/7326)
**Product spec:** `specs/GH7326/product.md`

## Context

Warp currently supports third-party coding agents through a `ThirdPartyHarness`
trait defined in `app/src/ai/agent_sdk/driver/harness/mod.rs`. Each supported
agent (Claude Code, Gemini) implements this trait in its own file and is
registered as a variant of the `Harness` enum in `crates/warp_cli/src/agent.rs`
and `CLIAgent` enum in `app/src/terminal/cli_agent.rs`.

ACP (Agent Client Protocol) is a standard JSON-RPC-over-stdio protocol for
agent-editor communication. Rather than adding a new per-agent harness for every
ACP-compatible CLI, a single `AcpHarness` implementation covers all of them.
The user supplies the command (e.g. `opencode`, `kimi`) and Warp handles the
protocol layer generically.

### Relevant files

- `crates/warp_cli/src/agent.rs:123` — `pub enum Harness` — needs new `Acp` variant
- `app/src/terminal/cli_agent.rs:108` — `pub enum CLIAgent` — needs new `Acp` variant with user-configurable command
- `app/src/ai/agent_sdk/driver/harness/mod.rs:57` — `ThirdPartyHarness` trait definition — the contract `AcpHarness` must implement
- `app/src/ai/agent_sdk/driver/harness/gemini.rs` — `GeminiHarness` — closest structural analog to `AcpHarness`
- `app/src/ai/agent_sdk/driver/harness/mod.rs:133` — `HarnessKind` enum — may need `Acp` variant
- `app/src/terminal/view/ambient_agent/harness_selector.rs:61` — harness dropdown — needs ACP entry
- `app/src/server/server_api/harness_support.rs` — server-side harness registration — needs ACP format slug

## Proposed changes

### 1. `crates/warp_cli/src/agent.rs`

Add a new `Acp` variant to `pub enum Harness`:

```rust
/// Delegate to any ACP-compatible agent CLI (user-configured command).
#[value(name = "acp")]
Acp,
```

Place between `OpenCode` and the `Unknown` fallback variant.

### 2. `app/src/terminal/cli_agent.rs`

Add a fieldless `Acp` variant to `pub enum CLIAgent`:

```rust
/// An ACP-compatible agent with a user-configured command.
Acp,
```

The user-configured command, display name, and args are stored in a separate
`AcpAgentConfig` struct in the AI settings model rather than inside the enum,
avoiding conflicts with `CLIAgent`'s existing `Copy` and `&'static str`
constraints. The `command_prefix()` and `display_name()` implementations look
up the active config from settings at call time. The `icon()` arm returns
`None` for v1.

### 3. New file: `app/src/ai/agent_sdk/driver/harness/acp.rs`

Create `AcpHarness` implementing `ThirdPartyHarness`. The struct carries the
user-configured command and optional args:

```rust
pub(crate) struct AcpHarness {
    command: String,
    args: Vec<String>,
}
```

Key `ThirdPartyHarness` method implementations:

- `harness()` → `Harness::Acp`
- `cli_agent()` → `CLIAgent::Acp`
- `validate()` → call `validate_cli_installed(&self.command, self.install_docs_url())`
- **Execution model:** The command is executed directly via argv (not 
  shell interpolation) to prevent injection. PATH resolution follows 
  the same rules as existing harnesses. Synced config that causes 
  command execution on another machine should require explicit user 
  confirmation before first run.
- `install_docs_url()` → `None` (unknown for arbitrary agents; follow-up can add a registry)
- `prepare_environment_config()` → write ACP session config if needed; for v1 this may be a no-op or minimal JSON config file with MCP server list
- `build_command()` → spawn `self.command` with `self.args`. No additional ACP-specific launch flags are injected by Warp — ACP-compatible agents communicate over stdio by default. Any required flags are user-supplied via the Args field.

The ACP protocol handshake and session flow is handled in a new
`run_acp_session()` function within this file, called from `run()`.

**Step 1 — Initialize:** Warp sends an `initialize` request:

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "method": "initialize",
  "params": {
    "protocolVersion": 1,
    "clientCapabilities": {
      "fs": { "readTextFile": true, "writeTextFile": true },
      "terminal": true
    },
    "clientInfo": { "name": "warp", "title": "Warp", "version": "<warp_version>" }
  }
}
```

The agent responds with its `protocolVersion`, `agentCapabilities`, and
`agentInfo`. If versions don't match, Warp closes the connection and
surfaces a descriptive error to the user.

**Step 2 — Session setup:** Warp calls `session/new` with workspace context.

**Step 3 — Prompt loop:** Warp sends prompts via `session/prompt` and
streams response chunks into Warp's conversation blocks.

**Timeout:** If no initialize arrives within 10 seconds, `run()` returns
`AgentDriverError::HarnessConfigSetupFailed` with a descriptive message.

**MCP passthrough:** After initialization, Warp offers to pass its 
configured MCP servers to the ACP agent session. The user must 
explicitly enable MCP passthrough per agent in settings — it is off 
by default. Only MCP servers the user has already authorized in Warp 
are eligible for passthrough.

### 4. `app/src/ai/agent_sdk/driver/harness/mod.rs`

- Add `mod acp;` and `pub(crate) use acp::AcpHarness;`
- Add `Acp` variant to `HarnessKind` (line 133) if this enum is exhaustively matched anywhere
- Register `AcpHarness` in the harness dispatch logic alongside `ClaudeHarness` and `GeminiHarness`

### 5. `app/src/terminal/view/ambient_agent/harness_selector.rs`

Add `Harness::Acp` to the harness selector dropdown. For user-configured ACP
agents, each appears as a separate entry using its display name (the command
string). This requires reading from a list of `AcpAgentConfig` entries persisted 
in the AI settings model. Each entry has a stable UUID identity, a 
display name, a command, and optional args. The selector serializes 
the selected agent by UUID. Config is local-only and not synced across 
machines in v1.

### 6. `app/src/server/server_api/harness_support.rs`

Add an `"acp"` format slug for ACP conversations, used when creating a
conversation record on the server. This follows the pattern of `"gemini_cli"`
for Gemini.

## Data flow

```
User selects ACP agent + sends prompt
  → AcpHarness::validate() checks command on PATH
  → AcpHarness::build_command() spawns process over stdio
  → run_acp_session() sends initialize, awaits initialize response (10s timeout)
  → calls session/new with workspace context
  → enters prompt/response loop:
      Warp sends prompt via session/prompt
      Agent streams response chunks
      Warp renders chunks as conversation blocks
      File edits → diff view
      Agent file/terminal requests → routed through Warp's existing approval UX
      Tool calls → tool call blocks
  → on agent exit or crash → conversation marked ended
```

## Tradeoffs

- **ACP agent config storage:** ACP agent configuration (name, command, args) 
  is stored in a dedicated `AcpAgentConfig` struct in the AI settings model. 
  `CLIAgent::Acp` remains fieldless to preserve `Copy` and `&'static str` 
  compatibility. The harness selector reads from the settings model to populate 
  per-agent entries. **Open question for Warp team: what is the preferred home 
  for persisting `AcpAgentConfig` — alongside existing third-party CLI agent 
  settings, or a new dedicated section?**
- **Stdio-only in v1:** Excludes ACP agents that communicate over HTTP. This
  covers the majority of current ACP agents (opencode, Kimi, Gemini, Codex all
  support stdio) and avoids auth complexity for v1.

## Testing and validation

Invariant-to-test mapping (from `product.md` success criteria):

1. **Unit test in `acp.rs`:** `AcpHarness::validate()` returns `Ok(())` for a
   command on PATH and `Err(AgentDriverError::CliNotInstalled)` for a missing
   command.
2. **Unit test:** `run_acp_session()` returns a timeout error if the mock agent
   process does not respond to `initialize` within the deadline.
3. **Integration test under `crates/integration/`:** Spawn a minimal ACP echo
   agent (a small test binary that implements initialize/session/new/session/prompt and
   returns a static response), send a prompt, and assert a conversation block
   is produced.
4. **Manual:** Configure `opencode` as an ACP agent, send a prompt, confirm
   response renders as a Warp conversation block.
5. **Manual:** Confirm `opencode` appears in the harness selector alongside
   Claude Code, Codex, and Gemini.
6. **Regression:** `cargo nextest run` passes across existing harness tests
   (`local_harness_launch_tests.rs`, `cli_agent_tests.rs`,
   `mod_test.rs` in `driver/harness/`).

## Risks and mitigations

- **ACP spec churn:** The ACP spec is still evolving. Mitigation: pin to the
  stable `initialize`/`session/new`/`session/prompt` core and treat unknown
  fields as ignored (forward-compatible deserialization with `#[serde(flatten)]`
  or `deny_unknown_fields = false`).
- **CLIAgent enum serialization:** `CLIAgent::Acp` is fieldless so it serializes
  cleanly. The `AcpAgentConfig` settings struct will need migration guards if its
  schema changes in future versions.
- **No icon for ACP agents:** The harness selector will show ACP agents without
  a logo. Mitigation: acceptable for v1; a generic "agent" icon can be added as
  a follow-up.

## Follow-ups

- Session resumption for ACP agents (requires ACP `loadSession` capability).
- A curated registry of known ACP agents with pre-filled commands and install URLs.
- Warp as ACP server (exposing Warp Agent to external IDEs like Zed).
- Per-agent icons for known ACP CLIs.
- HTTP transport support for remote ACP agents.
