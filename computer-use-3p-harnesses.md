# Computer use for third-party harnesses

Research notes, 2026-11-09. Covers the `warp` client repo and the `warp-server` repo. Paths are prefixed with the repo they belong to.

**Terms.** "CU" is computer use: a model driving a GUI through screenshots plus mouse and keyboard input. "Oz" is Warp's own server-driven agent loop. The third-party (3p) harnesses are external coding CLIs that Warp launches and that own their own agent loop: Claude Code, Codex, Gemini CLI, Pi and OpenCode. They are driven over a PTY (interactive CLI in a terminal block) or over ACP (Agent Client Protocol, through an adapter process).

## Bottom line

No third-party harness can use Warp's native computer use. Native CU is a server-run subagent built on the model provider's built-in computer tool, and Warp never makes a third-party harness's model calls.

The cloud sandbox is already CU-capable for every harness. The gap is the tool itself plus the policy, visibility and artifact pipeline around it.

## 1. For each 3p harness, why can't it support native CU? Could any support it natively?

### The structural reason, common to all of them

- **Native CU lives in warp-server.** The Oz main agent calls a `computer_use` tool, and the server then runs a separate computer-use subagent. That subagent declares the provider's built-in tool and translates its actions into Warp's client action protocol.
  - Anthropic: `computer_toolset_20260801`, `computer_20251124` or `computer_20250124`, chosen per model and host (`warp-server/logic/ai/llm/anthropic/util/util.go:1305-1387`).
  - OpenAI: the Responses API computer tool.
  - There is no plain function-calling fallback: any non-OpenAI model gets the Anthropic tool (`warp-server/logic/ai/multi_agent/agents/computer_use/agent.go:285-294`).
  - The subagent is pinned to one provider because "each provider has their own native computer use tools, so it's difficult/impossible to reformat tool calls from one provider to another" (`agent.go:445`).
- **Third-party harnesses call providers directly with the customer's credentials.** There is no Warp proxy or base-URL override, so Warp cannot add a provider tool to their requests. A server spec states it plainly: "Inference goes straight from the harness to the provider" (`warp-server/specs/third-party-harness-cost-estimation/TECH.md:5`).
- **The server documents the Oz-only state.** `computer_use_model_id` is "a sub-setting of the built-in Oz harness ... preserved but inert under a third-party harness" (`warp-server/logic/ai/ambient_agents/sources/agent_config.go:70`).
- **The client drops the setting for non-Oz harnesses.** `computer_use_config` is `None` unless the harness is Oz:
  - `warp/app/src/ai/agent_sdk/mod.rs:757`
  - `warp/app/src/ai/agent_sdk/standby.rs:605`
  - `warp/app/src/terminal/view/ambient_agent/model.rs:1088`
  - `warp/app/src/ai/orchestration/remote_child.rs:276`
- **Native CU is effectively Anthropic-only in prod.** OpenAI's computer tool is flagged off (`warp-server/config/prod.yaml:428`), and warp-server has no Gemini CU at all.

### What is already in place

The sandbox is not the blocker. The Xvfb sidecar is attached whenever computer use is enabled, independent of harness, alongside the coding-CLI sidecar (`warp-server/logic/ai/ambient_agents/workers/namespace/namespace.go:821`). The worker passes `--computer-use` to every harness and only `--computer-use-model` is Oz-gated (`warp-server/logic/ai/ambient_agents/workers/common/task_utils.go:1431-1443`). `computer_use_enabled` defaults to true.

### Per harness

| Harness | How Warp runs it | Its own CU upstream | Usable natively? |
|---|---|---|---|
| Claude Code | Enabled in prod. ACP with PTY fallback. | Built-in `computer-use` MCP server, research preview. macOS only, interactive only (no `-p`), Pro/Max/Ultra plan through a claude.ai login, not available on Bedrock, Vertex or Foundry. | Not in cloud: the sandbox is Linux with API-key or Bedrock auth. Only plausible on a local macOS PTY run with a subscription login. |
| Codex | Enabled in prod. ACP with PTY fallback. | Official CU is a desktop app feature (macOS and Windows). The CLI reportedly reaches it only through a bundled plugin that needs the desktop app's helper; this is undocumented. | No. |
| Gemini | Flag off in prod. PTY only. Warp passes it no MCP servers. | Browser agent only (drives Chrome through the accessibility tree, with an optional visual model). No desktop CU. | Browser-only at best. |
| Pi | Enabled in prod. ACP only. MCP delivered through `pi-mcp-adapter`. | None; function tools only. | No. |
| OpenCode | Not a server harness. Local child only. No MCP servers passed. | None. | No. |

Claude Code is the only harness with real CU of its own, and its conditions exclude Warp's Linux, API-key cloud runs.

## 2. For harnesses without CU support, what approaches could close the gap, and what are the tradeoffs?

| Approach | For | Against |
|---|---|---|
| **A. Warp CU MCP server.** The harness's own model drives generic screenshot, click and type tools. This is the in-progress, uncommitted `warp/crates/computer_use_mcp` crate, served by a hidden `computer-use-mcp` worker subcommand. | Client-only, and follows the precedent of auto-attaching the `warp-factory` MCP server (`warp/specs/REMOTE-3140/TECH.md`). Reaches Claude, Codex and Pi through existing MCP plumbing. Inference stays on the customer's provider key. | Not the provider's trained tool type. Screenshots accumulate in the main agent's context with no pruning and no subagent isolation. Warp gets no approval step, recording, artifacts or UI unless they are rebuilt (see section 3). |
| **B. Delegate to Oz's CU subagent.** Expose one high-level tool or skill: task in, text summary out. This is the same contract the Oz main agent already uses. | Reuses the native tool, the tuned prompt, screenshot pruning and offload, recording, artifacts and approval. Works for any harness, including ones with text-only models, and keeps images out of the harness's context. | Inference moves to Warp-billed models rather than the customer's key. Needs a new server entry point: today the subagent only runs inside a native Oz conversation. A child started with `oz agent run-cloud --computer-use` would land in a different sandbox with a different display, so it could not see what the harness just built. |
| **C. The harness's own CU.** | Vendor-built and tuned, with its own guardrails (per-app approval, terminal excluded from screenshots, global abort key). No Warp tool surface. | Only Claude Code on a local Mac with a subscription login. Nothing in cloud. Warp sees none of it. |
| **D. Proxy that injects the provider-native tool.** Route harness traffic through a Warp endpoint and add the computer tool to requests. | Truly native. | No proxy exists. The harness does not know the tool, so the proxy would have to execute the whole CU loop and hide it from the harness. Breaks with subscription and Bedrock auth. Fragile across CLI versions. |
| **E. Browser-only tools.** Playwright or Chrome DevTools MCP; `playwright-cli` is already in the sandbox. | Cheap and text-based. Needs no image support from the harness or model. | Web only. No native apps. |

**Recommendation.** Continue with A as the baseline, with E as a complement for web-only verification. Whether B is worth building is the open decision, and it turns on billing: B gives parity with Oz but charges CU through Warp rather than the customer's provider key.

### How approach A reaches each harness today

| Harness | PTY | ACP |
|---|---|---|
| Claude Code | Temp JSON passed with `--mcp-config` | `mcpServers` in `session/new` |
| Codex | `[mcp_servers.<name>]` in `~/.codex/config.toml` | `mcpServers` in `session/new` |
| Gemini | None: `_resolved_mcp_servers` is unused (`warp/app/src/ai/agent_sdk/driver/harness/gemini.rs:75`) | Not driven over ACP in cloud |
| Pi | Not applicable | Excluded from `session/new`; written to `mcp.json` and loaded by `pi-mcp-adapter` (`warp/app/src/ai/agent_sdk/driver/harness/pi.rs:23`) |
| OpenCode | None | Not applicable |

The natural injection point is next to the existing built-in servers at `warp/app/src/ai/agent_sdk/driver/mcp_startup.rs:673`. Use stdio: HTTP servers are sent over ACP without checking the agent's `mcpCapabilities`. The local child-agent launch path (`warp/app/src/pane_group/pane/local_harness_launch.rs`) passes no MCP config at all.

### Per-harness risks to test for approach A

- **Claude Code:** its 25,000-token MCP output cap (`MAX_MCP_OUTPUT_TOKENS`) also applies to image results.
- **Codex:** images are reportedly dropped when a tool result sets `structuredContent`. The current server is safe, because rmcp's `CallToolResult::success` leaves it unset. The default tool timeout is 60 seconds.
- **Pi:** the `pi-acp` adapter accepts session MCP servers but does not wire them through, which Warp already works around with `mcp.json`. Newer Pi versions have built-in MCP that defaults to a script-callable "codemode" exposure rather than declaring tools to the model. Whether images in direct tool results reach the model is unverified.
- **Gemini:** Warp's PTY driver would first need to write MCP config for it.
- **Models without image input:** Codex substitutes a text placeholder and OpenCode sessions reportedly break.

## 3. Other than the model making CU tool calls, what other gaps need closing?

- **Enablement plumbing.** `computer_use_config` is `None` for non-Oz harnesses (sites listed in section 1), so the run's `computer_use_enabled` never reaches a third-party driver. Nothing injects the CU server yet.
- **Approval and policy.** The execution-profile permission (default `Never`), the org-level computer use setting and the `RequestComputerUse` approval prompt are evaluated only when building an Oz request (`warp/app/src/ai/agent/api.rs:391`). Third-party runs auto-approve every MCP tool call: the ACP policy only inspects execute and file-write calls (`warp/app/src/ai/agent_sdk/driver/harness/acp/policy.rs`), and PTY runs use the CLIs' global bypass flags. An org setting of "never" must therefore gate injection of the server.
- **Model calibration.** For Oz, the server picks screenshot size limits and key-name conventions per model. The MCP server takes a static `--model-family` of `anthropic` or `openai`, which has to be derived from the harness and its configured model. Pi can run any provider, and Gemini models expect coordinates on a normalized 0 to 999 grid. The Anthropic preset is 1568 px on the long edge, while Opus 4.7 and later accept up to 2576 px.
- **Context growth.** Oz prunes screenshots once there are more than 25, keeping the newest 3 (`warp-server/logic/ai/multi_agent/agents/computer_use/screenshot_pruning.go:37`), and offloads them to storage. A third-party harness keeps every base64 screenshot in its own context and transcript. Warp captures those transcripts with caps of 8 MiB per line and 64 MiB total (`warp/app/src/ai/agent_sdk/driver/harness/transcript_persistence.rs:19`), and ACP conversation uploads are capped at 256 MB (`warp-server/router/handlers/public_api/harness_support.go:54`).
- **Visibility in Warp.** The ACP mapping flattens image results to the text `[image image/png]` (`warp/app/src/ai/agent_sdk/driver/harness/acp/protocol.rs:350`), so a CU call renders as a generic MCP card. PTY has no tool-result channel at all. There is no CU row and no screenshot viewer.
- **Artifacts.** `SCREENSHOT` artifacts are created only by the Oz server tool `report_screenshot`, which reads the latest screenshot from the native message history (`warp-server/logic/ai/multi_agent/runtime/report_screenshot.go`). The harness-support `report-artifact` endpoint accepts only pull requests and external references. A generic file upload does exist through the `oz-upload-file` skill and the hidden `oz artifact upload` command, but it produces an untyped file artifact. Slack attachments and PR-description media embedding are driven by Oz-only server tools.
- **Recording.** Start and stop are model-called Oz tools, backed by a controller inside the Warp app process that logs each action batch for the smart cut and pointer burn-in, then uploads the video and thumbnail against the conversation. The MCP server is a separate process and passes `pointer_sink: None` (`warp/crates/computer_use_mcp/src/session.rs:172`), so there is no video.
- **Window targeting.** The MCP session is full-screen only (`background_enabled: false`). That is fine on a dedicated sandbox display. On a local run it takes over the user's real mouse and keyboard, with none of the per-window targeting or teardown that Oz drives on subagent completion and cancel.
- **Guidance.** Oz has tuned prompts on when to use CU (GUI work only, build first, the subagent observes and the parent judges) and validators that re-prompt when required screenshots or videos are missing. Third-party harnesses would need a bundled skill, like the existing `oz-*` harness skills, plus a section in the system prompt returned by `resolve-prompt`.
- **Telemetry and usage.** The `ComputerUse.*` and `Recording.*` events and the CU usage counters are Oz-only.

## What was verified

- **Code:** the main code claims above were checked directly in both repos.
- **Outside the two repos:** the claim that the sandbox starts a display for any harness rests on `warp-agent-docker/entrypoint.bash`.
- **Upstream docs read directly:** the Claude Code computer use page, Pi's MCP docs and the `pi-acp` README.
- **Not re-checked:** the Codex CLI plugin behaviour, the Codex `structuredContent` behaviour, the OpenCode behaviour and the Gemini details come from a web research pass over GitHub issues and source.

## Upstream sources

- Claude Code computer use: https://code.claude.com/docs/en/computer-use
- Claude Code MCP: https://code.claude.com/docs/en/mcp
- Anthropic computer use tool: https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool
- OpenAI computer use guide: https://developers.openai.com/api/docs/guides/tools-computer-use
- Codex CLI computer use reports: https://github.com/openai/codex/issues/20851 and https://github.com/openai/codex/issues/19544
- Gemini CLI browser agent: https://github.com/google-gemini/gemini-cli/blob/main/docs/core/subagents.md
- Gemini CLI MCP: https://geminicli.com/docs/tools/mcp-server/
- Pi MCP: https://pi.dev/docs/latest/mcp
- `pi-acp` adapter: https://github.com/svkozak/pi-acp
- OpenCode MCP: https://opencode.ai/docs/mcp-servers/
- ACP session setup: https://agentclientprotocol.com/protocol/session-setup
