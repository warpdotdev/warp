# Product Spec: Support for Agent Client Protocol (ACP)

**Issue:** [warpdotdev/warp#7326](https://github.com/warpdotdev/warp/issues/7326)
**Figma:** none provided

## Summary

Warp should support the Agent Client Protocol (ACP) as a client, allowing any
ACP-compatible coding agent (e.g. opencode, Kimi CLI, Mistral's Devstral, custom
agents) to run inside Warp's native agent UX — blocks, diff view, file tree, and
conversation history — without requiring a custom Warp integration per agent.

## Problem

Today, each third-party coding agent (Claude Code, Codex, Gemini CLI) requires a
dedicated harness in Warp. Adding a new agent means writing new integration code.
Meanwhile, ACP is an emerging open standard (backed by Zed, JetBrains, and a
growing ecosystem) that defines a standard JSON-RPC-over-stdio protocol for
agent-editor communication — analogous to how LSP standardized language server
integration. Without ACP support, Warp users cannot run the growing number of
ACP-native agents inside Warp's UI, and must fall back to raw TUI output.

## Goals

- Any ACP-compatible agent can be launched inside Warp and uses Warp's native
  agent UI (conversation blocks, diff view, file tree, tool call rendering).
- Users can configure ACP agents the same way they configure existing harnesses
  (via the settings UI and/or the Warp config file).
- Users can opt in to passing Warp's MCP servers to an ACP agent session. This is off by default per agent.
- The ACP harness validates that the target agent CLI is installed before launch
  and surfaces a helpful install link if not.

## Non-goals

- Warp as an ACP *server* (other IDEs using Warp's agent) — this is a follow-up.
- Supporting ACP agents that require remote/HTTP transport (stdio only in v1).
- Replacing existing dedicated harnesses (Claude Code, Codex, Gemini) — those
  remain as-is for agents that benefit from custom integrations.

## User experience

### Configuring an ACP agent

1. User opens Settings → Agents → Third-party CLI agents.
2. A new "ACP agents" section lists any configured ACP agents alongside existing
   harnesses.
3. User clicks "Add ACP agent" and enters:
   - **Name** (e.g. "opencode", "Kimi")
   - **Command** (e.g. `opencode`, `kimi`)
   - **Args** (optional, e.g. `--model gpt-4o`)
4. Warp validates the command exists on PATH. Commands are executed via 
   argv directly — no shell interpolation. Synced configs that would 
   execute a new command on another machine require explicit confirmation 
   before first run.
5. The configured agent appears in the harness selector dropdown in the agent
   input footer.

### Launching an ACP agent session

1. User selects an ACP agent from the harness selector and sends a prompt.
2. Warp spawns the agent process over stdio and initiates the ACP handshake
   (initialize → initialized).
3. The agent's responses render in Warp's native conversation UI:
   - Text responses appear as agent message blocks.
   - File edits surface in Warp's diff view for review.
   - Tool calls (if passed via MCP) render as tool call blocks.
4. The session persists. Session resumption might be out of scope for v1.

### Edge cases

- **Agent not installed:** Warp shows an inline error with install instructions
  before attempting to spawn the process.
- **Agent crashes mid-session:** Warp surfaces an error block and marks the
  conversation as ended, matching behavior of existing harnesses.
- **ACP handshake fails:** Warp shows a descriptive error (e.g. "Agent did not
  respond to initialize within 10 seconds") rather than hanging.
- **Agent sends unsupported capability:** Warp ignores unknown capability fields
  gracefully (forward-compatible).
- **No MCP servers configured:** ACP session launches without MCP context;
  no error is shown (same as existing harness behavior).

## Success criteria

1. An ACP agent configured with a valid command launches successfully and
   produces a conversation block in Warp's UI.
2. A file edit proposed by the agent appears in Warp's diff view.
3. If the command is not on PATH, Warp shows an error before spawning.
4. If the ACP handshake times out, Warp surfaces a descriptive error.
5. Configured ACP agents appear in the harness selector alongside Claude Code,
   Codex, and Gemini.
6. Users can opt in to passing Warp's configured MCP servers to an ACP agent session; passthrough is off by default.
7. ACP conversation turns are stored as Warp block snapshots and are 
   viewable in conversation history. No ACP session IDs or agent-side 
   transcripts are stored in v1 (resumption is a follow-up).

## Open questions

1. How should Warp handle ACP agents that advertise capabilities Warp doesn't
   yet render (e.g. multi-file context beyond diff view)?
2. Should there be a curated list of known ACP agents with pre-filled commands
   and install URLs (similar to how Codex and Gemini are pre-configured)?
