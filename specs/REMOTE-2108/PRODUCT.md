# REMOTE-2108: Native /cloud-agent UI — Feature Parity Gap with REST API / CLI / SDK

## Problem Statement

When launching a cloud agent run from the native Warp `/cloud-agent` slash command (or any
in-app cloud mode entry point), users cannot configure many options that are available when
starting runs programmatically via the REST API, CLI (`oz agent run-cloud`), or SDK.

This document audits the full set of gaps, categorises them by priority, and proposes a path
forward for each one.

---

## Current State

### What the `/cloud-agent` UI exposes today

The `AmbientAgentViewModel::build_default_spawn_config` method assembles the configuration
that is sent on every in-app cloud spawn. It populates exactly these fields:

| Field | Source in UI |
|---|---|
| `environment_id` | Environment selector dropdown |
| `model_id` | Model selector (Oz models only; falls back to `"auto"` for BYOK/custom-router) |
| `computer_use_enabled` | Derived from workspace-level AI-autonomy setting — not directly settable per-run |
| `worker_host` | Read from `WARP_CLOUD_MODE_DEFAULT_HOST` env var or workspace setting — no per-run dropdown |
| `harness` | Harness selector (Oz, Claude Code, Gemini, Codex) |
| `harness_auth_secrets` | Auth-secret selector for non-Oz harnesses |

Top-level `SpawnAgentRequest` fields that the UI always sets automatically:
- `prompt` — what the user types in the compose bar
- `mode` — derived from prompt prefix (`/plan`, `/orchestrate`) or toolbar
- `interactive` — always `true` for in-app runs
- `attachments` — images/files attached via file picker or drag-and-drop

### What the REST API / CLI / SDK additionally support

The public `POST /agent/runs` endpoint (schema `RunAgentRequest` + `AmbientAgentConfig`)
and the `oz agent run-cloud` CLI expose the following options that are **not surfaced** in
the native `/cloud-agent` UI today:

#### AmbientAgentConfig gaps

| Gap | API field | CLI flag | Notes |
|---|---|---|---|
| MCP server selection | `config.mcp_servers` | `--mcp <SPEC>` | **Most-requested gap.** See REMOTE-2099. Can reference Warp-managed MCP servers by UUID (`warp_id`) or provide inline stdio/SSE server configs. |
| Skill as base prompt | `config.skill_spec` / `config.skills` | `--skill <SPEC>` | Allows a SKILL.md to serve as the base system prompt. Also exposed via top-level `skill` field. |
| Custom system/base prompt | `config.base_prompt` | _(config file only)_ | Injects a custom preamble before the user prompt. Useful for domain-specific context. |
| Run name / label | `config.name` | `--name <NAME>` | Used for grouping, filtering (`GET /agent/runs?name=…`), and traceability. UI auto-generates titles. |
| Idle timeout after run | `config.idle_timeout_minutes` | _(not yet in CLI)_ | How long to keep the sandbox alive after the agent finishes (default 10 min, max 60 min). |
| Session sharing level | `config.session_sharing.public_access` | _(not yet in CLI)_ | Grant `VIEWER`/`EDITOR` access to anyone-with-link for the resulting shared session and conversation. |
| Memory stores | `config.memory_stores` | _(not yet in CLI)_ | Attach persistent memory stores to the run. |
| Inference providers | `config.inference_providers` | _(not yet in CLI)_ | Per-run inference provider overrides (BYOK/custom endpoint). |
| Runner override | `config.runner_id` | `--runner <ID>` | Override which compute runner (docker image + instance shape) the environment uses. |
| Worker host (per-run) | `config.worker_host` | `--host <WORKER_ID>` | UI reads this from an env var/workspace setting globally; API allows setting it per-run. |
| Computer use (per-run toggle) | `config.computer_use_enabled` | `--computer-use` / `--no-computer-use` | UI derives this from workspace AI-autonomy setting; API/CLI allow explicit per-run override. |

#### Top-level RunAgentRequest gaps

| Gap | API field | CLI flag | Notes |
|---|---|---|---|
| Custom run title | `title` | _(not exposed)_ | Separate from `config.name`; used as the display title for the run. UI always uses auto-generation. |
| Team ownership | `team` | _(scoped via `--scope`)_ | Explicit team vs. personal run. UI determines this automatically based on workspace context. |
| Agent identity | `agent_identity_uid` | `--agent <UID>` | Run under a named-agent identity (applies the agent's config and attributes credits to it). |

---

## Gap Prioritization

### P0 — MCP server selection (REMOTE-2099, in progress)
The inability to select MCP servers at cloud-agent launch is the most commonly reported gap
(by both the customer in this issue and in multiple internal reports). The server already
supports `config.mcp_servers` fully; the gap is purely in the UI compose flow.

**Recommendation:** Surface a Warp-managed MCP server picker in the cloud-agent compose
footer (similar to the harness selector and environment selector that already exist). This
is tracked separately in REMOTE-2099 and REMOTE-1808.

### P1 — Skill selection
Power users launching agents from the CLI routinely pass `--skill` to inject a SKILL.md as
a base system prompt. There is no equivalent in the `/cloud-agent` compose UI.

**Recommendation:** Add a skill picker (could reuse the existing `/skills` slash command
picker pattern) to the cloud-agent compose footer, or allow `/cloud-agent --skill <SPEC>`
syntax in the slash command argument.

### P1 — Custom run name
The `config.name` field is used heavily in API automation to categorize and filter runs.
In-app runs always get auto-generated titles.

**Recommendation:** Allow users to optionally enter a name/title in the compose UI before
dispatching. A lightweight optional text field in the compose footer would suffice.

### P2 — Base prompt / system prompt
`config.base_prompt` is a developer-facing feature useful for giving the agent
domain-specific context without embedding it in every prompt. It is available via the CLI
config file (`AGENTS.md`-adjacent workflow) but not from the UI.

**Recommendation:** Expose via a collapsible "Advanced" section in the compose UI, or
document that the CLI / config file is the recommended path for this use case.

### P2 — Idle timeout
`config.idle_timeout_minutes` is useful when users want to keep the sandbox alive for a
follow-up interaction after the agent finishes. The default (10 min) is often too short for
exploratory sessions.

**Recommendation:** Expose as a per-run option in an "Advanced" section. The common values
(10 min, 30 min, 60 min) can be presented as a dropdown.

### P3 — Session sharing configuration
`config.session_sharing.public_access` allows sharing a cloud session link with anyone.
Currently the UI has no way to set this at launch time.

**Recommendation:** Document the API path for now; consider surfacing in an "Advanced"
section in a future iteration.

### P3 — Agent identity
Running as a named agent identity (`agent_identity_uid`) is a programmatic / automation
feature. It is not a common interactive workflow.

**Recommendation:** Document the API path; no UI surfacing needed for now.

### P3 — Memory stores, inference providers, runner override
These are advanced/enterprise features primarily accessed programmatically.

**Recommendation:** Document the API/CLI path; defer UI surfacing.

---

## Relationship to Related Issues

- **REMOTE-2099** (In Progress) — MCP servers not available when starting a cloud agent from
  terminal or during cloud handoff. This is the highest-priority item from this audit.
- **REMOTE-2009** (Backlog) — Ability to select individual MCP tools per agent.
- **REMOTE-1808** (Todo) — Agent- and environment-scoped MCP servers.

---

## Recommended Next Steps

1. Continue REMOTE-2099 (MCP picker in cloud-agent compose UI) as the immediate follow-on
   from this audit.
2. Create separate tickets for skill selection (P1) and run naming (P1) with this audit as
   context.
3. Update the Oz documentation to explicitly note which launch options are only available
   via the REST API / CLI / SDK, so users know where to go for advanced configuration.
4. Revisit this audit when new fields are added to `AmbientAgentConfig` to keep the gap
   list current.
