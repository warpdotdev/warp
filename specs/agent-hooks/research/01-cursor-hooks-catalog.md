# Cursor Hooks: Source-Backed Catalog

Research snapshot: 2026-10-19. The live Cursor Hooks and plugin references do not show a document version or publication date; dates and versions below are included only where Cursor’s changelog/release notes show them. This report records the published contract, not inferred runtime behavior. In particular, when a field’s requiredness, regex engine, or behavior is not specified, it is called out as “docs do not say.”

## 1. Configuration

### 1.1 Native `hooks.json` shape

Cursor’s Hooks reference describes a `hooks.json` object with a numeric `version` and a `hooks` object mapping event names to arrays of hook definitions. The consulted references do not present a formal machine-readable JSON Schema document. The exact top-level and per-definition fields documented in the live reference are:

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `version` | `number` | Required | Configuration schema version; must be a positive integer. Cursor’s examples use `1`. |
| `hooks` | object mapping hook names to arrays | Docs do not explicitly say whether this top-level field is required; all native examples include it. | The hook-name-to-definition mapping. |
| `command` | `string` | Required in the per-script options table | Shell string, absolute path, or relative path to execute. |
| `type` | `"command" \| "prompt"` | Optional; default `"command"` | Hook execution type. |
| `timeout` | `number` | Optional; default “platform default” | Execution timeout in seconds. |
| `loop_limit` | `number \| null` | Optional; default `5` for Cursor hooks | Per-script follow-up limit for `stop` and `subagentStop`; `null` means no limit. Cursor says Claude Code hooks default to `null`. |
| `failClosed` | `boolean` | Optional; default `false` | If true, hook failure blocks the action. |
| `matcher` | `string` | Optional | Regex filter. Empty string or `"*"` matches everything. |
| `prompt` | Type and requiredness are not stated | Shown for `type: "prompt"` | Natural-language evaluation prompt. |
| `model` | Type is not stated | Optional | Overrides the default prompt-hook model. |

There is a documentation inconsistency: the per-script option table marks `command` as required even though the prompt-hook example has `type` and `prompt` but no `command`. The Hooks reference does not explain whether prompt definitions omit `command` by design or whether the table is command-hook-only. The plugin reference’s `hooks/hooks.json` example also omits top-level `version`, while the native Hooks reference says `version` is required. It does not say whether plugin hook files have an exception.

Example native project/user format, as documented:

```json
{
  "version": 1,
  "hooks": {
    "afterFileEdit": [
      {
        "command": ".cursor/hooks/format.sh"
      }
    ]
  }
}
```

Source: [Cursor Hooks](https://cursor.com/docs/hooks) (undated live reference; accessed 2026-10-19); [Plugins reference](https://cursor.com/docs/reference/plugins) (undated live reference).

### 1.2 Configuration locations and precedence

| Scope | Location / source | Working directory for hook process |
| --- | --- | --- |
| Enterprise, MDM-managed | macOS: `/Library/Application Support/Cursor/hooks.json`; Linux/WSL: `/etc/cursor/hooks.json`; Windows: `C:\ProgramData\Cursor\hooks.json` | Enterprise config directory. |
| Team, dashboard-distributed | Cursor Dashboard → Team Content → Hooks; Enterprise-only | Managed hooks directory. |
| Project | `<project-root>/.cursor/hooks.json` | Project root. Project hooks run only in a trusted workspace. |
| User | `~/.cursor/hooks.json` | `~/.cursor/`. |
| Cursor Plugin | `.cursor-plugin/plugin.json` may name a hooks config path or inline hook config; default discovery is `<plugin-root>/hooks/hooks.json` | Docs do not say which working directory is used for plugin hook commands or how relative command paths resolve. |
| Claude Code compatibility | `.claude/settings.local.json`, `.claude/settings.json`, and `~/.claude/settings.json` when third-party imports are enabled | Docs do not say the working directory used for imported Claude hook commands. |

For native config sources, Cursor gives this precedence order, highest to lowest: **Enterprise → Team → Project → User**. All matching hooks from every source run. Permission results merge by severity, `deny` over `ask` over `allow`, irrespective of source. `user_message` and `agent_message` values are concatenated. For other fields (the docs give `followup_message` as an example), the last response wins; responses merge in priority order, so a lower-priority source can override a higher-priority source for those fields.

The precedence list does not include Cursor Plugin hooks. The separate Claude compatibility page gives a seven-source order that inserts Claude files after native User hooks; see §6. The docs do not say where plugin hooks fall relative to native or Claude-configured hooks.

**Order and concurrency:** the official Jan. 8, 2026 CLI changelog says hooks execute in parallel with merged responses. The Hooks reference says all matching hooks run and responses merge, but does not say whether the desktop app uses the same parallel scheduling. With parallel execution, “last response” does not establish a deterministic order among equal-priority hooks; docs do not specify a tie-break rule. The order of entries in a hook array is not documented as an execution-order guarantee.

Sources: [Cursor Hooks — Configuration and Team Distribution](https://cursor.com/docs/hooks); [Third Party Hooks — Priority Order](https://cursor.com/docs/reference/third-party-hooks); [Plugins reference](https://cursor.com/docs/reference/plugins); [Jan. 8, 2026 CLI changelog](https://cursor.com/changelog/cli-jan-08-2026).

### 1.3 Reloading, workspace trust, and distribution

- Cursor watches `hooks.json` files and reloads them on save. If hooks still do not load, the troubleshooting section recommends restarting Cursor.
- Project hooks require a trusted workspace.
- Team hooks are synced to team members automatically. The reference states automatic synchronization every thirty minutes and OS targeting for platform-specific hooks.
- Customize manages extensions at user, team, and workspace scope. Cursor Plugin hooks are available there; Agent Plugins (the open standard) package skills and MCP servers, while Cursor Plugins can additionally package hooks.
- For exact plugin paths, the Cursor Plugin manifest may set `hooks` to a path or inline hook config. If unspecified, Cursor discovers `hooks/hooks.json` and parses its event names.

Sources: [Cursor Hooks — Team Distribution and Troubleshooting](https://cursor.com/docs/hooks); [Customize Cursor](https://cursor.com/docs/customize-cursor); [Plugins reference](https://cursor.com/docs/reference/plugins).

### Partner integrations
Cursor’s Dec. 22, 2025 partner post lists the following implementations. These are partner-described use cases, not additional hook-schema guarantees.

| Partner | Cursor-described use |
| --- | --- |
| MintMCP | Uses `beforeMCPExecution` and `afterMCPExecution` to inventory MCP servers, monitor tool usage, and scan responses for sensitive data before they reach the model. |
| Oasis Security | Uses hooks to enforce least-privilege policies and maintain audit trails. |
| Runlayer | Wraps MCP tools and integrates with its MCP broker for centralized control and visibility. |
| Corridor | Provides real-time feedback on code implementation and security design decisions. |
| Semgrep | Scans AI-generated code for vulnerabilities and gives feedback so the agent can regenerate. |
| Endor Labs | Intercepts package installs and scans for malicious dependencies. |
| Snyk | Uses Evo Agent Guard to review agent actions in real time for prompt injection and dangerous tool calls. |
| 1Password | Checks required 1Password Environment files before shell commands, enabling just-in-time secrets without writing credentials to disk. |

Source: [Hooks for security and platform teams](https://cursor.com/blog/hooks-partners), Dec. 22, 2025; [Cursor Hooks — Partner Integrations](https://cursor.com/docs/hooks).

### 1.4 Matcher semantics

The documented matcher is a regex string. `""` and `"*"` match everything. Cursor does not name the regex flavor, specify case sensitivity, or say whether matching is an unanchored search or a whole-string match. “Matched against the full command string” describes the input string, not regex anchoring behavior.

| Hook | Value matched against |
| --- | --- |
| `preToolUse`, `postToolUse`, `postToolUseFailure` | Tool type/name, including `Shell`, `Read`, `Write`, `Grep`, `Delete`, `Task`, and MCP tools in `MCP:<tool_name>` form. |
| `subagentStart`, `subagentStop` | Subagent type, e.g. `generalPurpose`, `explore`, `shell`. |
| `beforeShellExecution`, `afterShellExecution` | Full shell command string. |
| `beforeReadFile` | Literal `Read`. |
| `afterFileEdit` | Literal `Write`. |
| `beforeTabFileRead` | Literal `TabRead`. |
| `afterTabFileEdit` | Literal `TabWrite`. |
| `beforeSubmitPrompt` | Literal `UserPromptSubmit`. |
| `stop` | Literal `Stop`. |
| `afterAgentResponse` | Literal `AgentResponse`. |
| `afterAgentThought` | Literal `AgentThought`. |
| `beforeMCPExecution`, `afterMCPExecution`, lifecycle events | No matcher target is specified in the “Available matchers by hook” list; docs do not say what value (if any) is matched. |

Sources: [Cursor Hooks — Matcher Configuration](https://cursor.com/docs/hooks); [Plugins reference — Hooks format](https://cursor.com/docs/reference/plugins).

### 1.5 Process environment

The Hooks reference says scripts receive these environment variables:

| Variable | Meaning | Presence |
| --- | --- | --- |
| `CURSOR_PROJECT_DIR` | Workspace root directory | Always |
| `CURSOR_VERSION` | Cursor version string | Always |
| `CURSOR_USER_EMAIL` | Authenticated user email | If logged in |
| `CURSOR_TRANSCRIPT_PATH` | Conversation transcript path | If transcripts are enabled |
| `CURSOR_CODE_REMOTE` | String `"true"` for a remote workspace | For remote workspaces |
| `CLAUDE_PROJECT_DIR` | Alias for project directory for Claude compatibility | Always |

`CURSOR_TRANSCRIPT_PATH` matches `transcript_path` when present. The standard hook envelope also contains `user_email` and `transcript_path`. A `sessionStart` response may provide `env`; those variables are available to later hook executions in that session. Docs do not enumerate the rest of the inherited operating-system environment or say whether `sessionStart.env` is exported to tool processes outside the hook runner.

Source: [Cursor Hooks — Environment Variables](https://cursor.com/docs/hooks); [Cursor IDE release notes](https://cursor.com/docs/release-notes/ide) (the latest hook-specific note says shell/MCP hooks receive `tool_use_id` and all hooks receive `transcript_path` when available).

## 2. Execution contract

### 2.1 Standard I/O and common input

Command hooks are spawned processes. They receive one JSON object on stdin and return JSON on stdout; the transport is stdio. Hook definitions and examples use JSON in both directions. Cursor does not publish a formal machine-readable JSON Schema for the full event union.

All hooks receive this common base set in addition to event-specific fields, except `workspaceOpen`, whose reduced envelope is described in §3.21:

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `conversation_id` | `string` | Included in the common base; docs do not otherwise state requiredness | Stable conversation ID across turns. |
| `generation_id` | `string` | Included in the common base; docs do not otherwise state requiredness | Current generation ID; changes with each user message. |
| `model` | `string` | Included in the common base; docs do not otherwise state requiredness | Legacy model slug configured for the composer that triggered the hook. |
| `model_id` | `string` | Optional | Structured model ID when available. |
| `model_params` | array of objects `{ "id": "string", "value": "string" }` | Optional | Selected model parameters such as thinking, context, or effort. |
| `hook_event_name` | `string` | Included in the common base; docs do not otherwise state requiredness | Hook currently running. |
| `cursor_version` | `string` | Included in the common base; docs do not otherwise state requiredness | Cursor application version. |
| `workspace_roots` | array of strings | Included in the common base; docs do not otherwise state requiredness | Workspace root folders; may contain multiple paths. |
| `user_email` | `string \| null` | Included; may be null | Authenticated user email, if available. |
| `transcript_path` | `string \| null` | Included; may be null | Main conversation transcript path; null if transcripts are disabled. |

The base schema marks `model_id` and `model_params` optional. For most event-specific payloads, the reference shows fields in examples but does not separately state which are required. That status is identified below as “docs do not say” unless explicitly marked.

Source: [Cursor Hooks — Common schema](https://cursor.com/docs/hooks).

### 2.2 Exit codes, schema errors, and fail-open/fail-closed

| Process result | Documented behavior |
| --- | --- |
| Exit code `0` | Hook succeeded; use its JSON output. |
| Exit code `2` | Block the action; equivalent to returning `permission: "deny"`. |
| Any other non-zero exit | Hook failure; action proceeds by default (fail-open). |
| Crash or timeout | Failure; action proceeds by default. Set `failClosed: true` to block on these failures. |
| Invalid JSON or a response that does not match the event’s schema | Blocks the action for permission hooks, even if `failClosed` is `false`. |

The six hooks Cursor explicitly labels as permission hooks for malformed-JSON/schema behavior are `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `beforeTabFileRead`, `subagentStart`, and `preToolUse`. This list does not include `beforeSubmitPrompt`, even though its own contract can block prompt submission with `continue: false`; malformed-output behavior for that event is not stated.

The configuration table includes “no output” among failures controlled by `failClosed`, while the docs separately say invalid JSON/schema blocks permission hooks even when `failClosed` is false. They do not clarify whether empty stdout is always treated as invalid JSON or as the no-output failure case.

Source: [Cursor Hooks — Command-Based Hooks and Per-Script Options](https://cursor.com/docs/hooks).

### 2.3 Decision merge and overlapping hooks

When multiple matching definitions run, every matching hook runs and their responses merge:

- `deny` beats `ask`, and `ask` beats `allow`, regardless of source priority.
- `user_message` and `agent_message` concatenate.
- For other fields (including `followup_message`), last response wins. Given the listed priority order, a lower-priority source can override a higher-priority source for these fields.
- CLI hooks were documented as running in parallel as of Jan. 8, 2026. The documentation does not specify desktop execution concurrency or a deterministic “last” result among same-priority hooks.

For one shell tool call, `preToolUse` is the generic hook for every tool type and `beforeShellExecution` is the shell-specific hook. If both are configured and match, both apply before execution; the docs do not specify a serial order between them. Their permission results follow the merge rules above.

Source: [Cursor Hooks — Configuration](https://cursor.com/docs/hooks); [Jan. 8, 2026 CLI changelog](https://cursor.com/changelog/cli-jan-08-2026).

### 2.4 Output fields: types, presence, and effect

The event subsections show the documented output objects. This inventory makes field types and requiredness explicit; where the reference shows a field but does not state its presence requirement, that is recorded as “docs do not say.”

| Hook | Output field | Type | Required/optional | Meaning |
| --- | --- | --- | --- | --- |
| `sessionStart` | `env` | object | Optional | Session variables available to later hook executions. |
| `sessionStart` | `additional_context` | `string` | Optional | Added to initial system context. |
| `sessionStart` | `continue` | Type not stated | Accepted, ignored | Does not block session creation, even if false. |
| `sessionStart` | `user_message` | Type not stated | Accepted, ignored | No caller effect documented. |
| `preToolUse` | `permission` | `string`; `allow`, `deny`, and schema-accepted `ask` | Docs do not say | Allow or deny; `ask` is not enforced for this hook. |
| `preToolUse` | `user_message` | `string` | Optional | Shown to user when denied. |
| `preToolUse` | `agent_message` | `string` | Optional | Fed back to agent when denied. |
| `preToolUse` | `updated_input` | object | Optional | Replacement tool input. |
| `postToolUse` | `updated_mcp_tool_output` | object | Optional | Replaces MCP tool output seen by the model. |
| `postToolUse` | `additional_context` | `string` | Optional | Injected after tool result. |
| `postToolUseFailure` | `additional_context` | `string` | Optional | Injected after failed tool result. |
| `subagentStart` | `permission` | `string`; `allow` or `deny` | Docs do not say | Allow/deny spawning; `ask` is treated as `deny`. |
| `subagentStart` | `user_message` | `string` | Optional | Shown when subagent is denied. |
| `subagentStop` | `followup_message` | `string` | Optional | Automatic follow-up, consumed only for completed status. |
| `beforeShellExecution` | `permission` | `string`; `allow`, `deny`, or `ask` | Docs do not say | Shell permission decision; precise ask UI behavior is not specified. |
| `beforeShellExecution` | `user_message` | `string` | Docs do not say | Message shown in client. |
| `beforeShellExecution` | `agent_message` | `string` | Docs do not say | Message sent to agent. |
| `beforeMCPExecution` | `permission` | `string`; `allow`, `deny`, or `ask` | Docs do not say | MCP permission decision. |
| `beforeMCPExecution` | `user_message` | `string` | Docs do not say | Message shown in client. |
| `beforeMCPExecution` | `agent_message` | `string` | Docs do not say | Message sent to agent. |
| `beforeReadFile` | `permission` | `string`; `allow` or `deny` | Docs do not say | Allow/block file read. |
| `beforeReadFile` | `user_message` | `string` | Optional | Shown to user when denied. |
| `beforeSubmitPrompt` | `continue` | `boolean` | Docs do not say | Whether submission proceeds. |
| `beforeSubmitPrompt` | `user_message` | `string` | Optional | Shown when submission is blocked. |
| `preCompact` | `user_message` | `string` | Optional | Message to show when compaction occurs; does not change compaction. |
| `stop` | `followup_message` | `string` | Optional | Automatically submitted as next user message when non-empty. |
| `beforeTabFileRead` | `permission` | `string`; `allow` or `deny` | Docs do not say | Allow/block Tab file read. |
| `workspaceOpen` | `pluginPaths` | array of strings | Optional | Absolute plugin directories to load for current workspace. |
| `afterShellExecution`, `afterMCPExecution`, `afterFileEdit`, `afterAgentResponse` | No output fields documented | — | — | Reference gives no output schema / response effect. |
| `sessionEnd` | No output fields | — | — | Fire-and-forget; response is logged but not used. |
| `afterAgentThought`, `afterTabFileEdit` | No output fields currently supported | — | — | Reference explicitly says no supported output fields. |

Source: [Cursor Hooks — event output contracts](https://cursor.com/docs/hooks).

## 3. Per-hook reference

For each event below, “common base” means the fields in §2.1 plus the event-specific fields shown. The `hook_event_name` value is the exact event name. Inputs below distinguish fields stated by the reference from fields whose requiredness is not stated. Shell/MCP `tool_use_id` is a later addition documented in release notes but omitted from the Hooks reference examples.

### 3.1 `sessionStart`

**Trigger:** A new composer conversation is created. It is fire-and-forget; the agent loop does not wait for or enforce a blocking response.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `session_id` | `string` | Docs do not say | Unique session identifier; same as `conversation_id`. |
| `is_background_agent` | `boolean` | Docs do not say | Background-agent session vs. interactive session. |
| `composer_mode` | `string` | Optional | Starting mode, e.g. `agent`, `ask`, `edit`. |
| `repo_urls` | Array of URLs | Self-Hosted claim only; requiredness not stated | URLs for repositories in the claimed run. |
| `repos` | Array of objects | Self-Hosted claim only; requiredness not stated | One entry per repository; entries contain `repo_url`, `primary`, and `ref` when the request pins a branch or commit. |
| `repos[].repo_url` | Type not stated | Included in each documented `repos` entry | Repository URL. |
| `repos[].primary` | Boolean indicated by the documented `true` value | Included in each documented `repos` entry | `true` for the repository the run routed on. |
| `repos[].ref` | Type not stated | Present when the request pins a branch or commit | Pinned branch or commit reference. |

**Output:**

```json
{
  "env": { "<key>": "<value>" },
  "additional_context": "<context to add to conversation>"
}
```

`env` sets session variables for subsequent hook executions; `additional_context` is added to the conversation’s initial system context. The schema also accepts `continue` and `user_message`, but current callers ignore them; `continue: false` does not prevent session creation.

**Can block?** No; fire-and-forget and not enforced. **Can modify?** Can add initial context and session-scoped variables. **Cloud:** No for Cursor-managed Cloud Agents; yes on Self-Hosted Machines when the session claims the worker.

Source: [Cursor Hooks — `sessionStart`](https://cursor.com/docs/hooks); [Hooks on Pools](https://cursor.com/docs/cloud-agent/self-hosted/pool).

### 3.2 `sessionEnd`

**Trigger:** A composer conversation ends. Fire-and-forget for logging, analytics, or cleanup; response is logged but not used.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `session_id` | `string` | Docs do not say | Unique ID of ending session. |
| `reason` | `string` enum | Docs do not say | `completed`, `aborted`, `error`, `window_close`, or `user_close`. |
| `duration_ms` | `number` | Docs do not say | Total session duration in milliseconds. |
| `is_background_agent` | `boolean` | Docs do not say | Whether the session was a background agent. |
| `final_status` | `string` | Docs do not say | Final session status. |
| `error_message` | `string` | Optional | Error details when `reason` is `error`. |

**Output:** No output fields; fire-and-forget.

**Can block?** No. **Can modify?** No documented output effect. **Cloud:** No for Cursor-managed Cloud Agents; yes on Self-Hosted Machines when the claim is released.

Source: [Cursor Hooks — `sessionEnd`](https://cursor.com/docs/hooks); [Hooks on Pools](https://cursor.com/docs/cloud-agent/self-hosted/pool).

### 3.3 `preToolUse`

**Trigger:** Before any tool executes; generic hook for Shell, Read, Write, MCP, Task, etc. Use a matcher to filter by tool.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `tool_name` | `string` | Docs do not say | Tool type/name, e.g. `Shell`. |
| `tool_input` | `object` | Docs do not say | Proposed tool input, e.g. command and working directory. |
| `tool_use_id` | `string` | Docs do not say | Tool call ID. |
| `cwd` | `string` | Docs do not say | Current working directory. |
| `agent_message` | `string` | Docs do not say | Agent message associated with the tool proposal. |

The event example also shows common `model`, `model_id`, and `model_params`.

**Output:**

```json
{
  "permission": "allow",
  "user_message": "<message shown in client when denied>",
  "agent_message": "<message sent to agent when denied>",
  "updated_input": { "command": "npm ci" }
}
```

`permission` is documented as `allow` or `deny`; the schema accepts `ask` but Cursor says it is not enforced for `preToolUse` today. `updated_input` replaces the proposed tool input.

**Can block?** Yes, with `deny`; `ask` is explicitly not enforced. **Can modify?** Yes, through `updated_input`; messages are delivered only on denial. **Cloud:** Yes.

Source: [Cursor Hooks — `preToolUse`](https://cursor.com/docs/hooks).

### 3.4 `postToolUse`

**Trigger:** After successful tool execution. It can audit, add analytics, and inject context.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `tool_name` | `string` | Docs do not say | Tool type/name. |
| `tool_input` | `object` | Docs do not say | Input passed to the tool. |
| `tool_output` | `string` | Docs do not say | JSON-stringified result payload, not raw terminal text. |
| `tool_use_id` | `string` | Docs do not say | Tool call ID. |
| `cwd` | `string` | Docs do not say | Current working directory. |
| `duration` | `number` | Docs do not say | Execution time in milliseconds. |

**Output:**

```json
{
  "updated_mcp_tool_output": { "modified": "output" },
  "additional_context": "Test coverage report attached."
}
```

`updated_mcp_tool_output` replaces output seen by the model for MCP tools only. `additional_context` is injected after the tool result. No output behavior for non-MCP tool results is documented.

**Can block?** No documented pre-execution blocking; this is after success. **Can modify?** Yes, MCP result only, and can add context. **Cloud:** Yes.

Source: [Cursor Hooks — `postToolUse`](https://cursor.com/docs/hooks).

### 3.5 `postToolUseFailure`

**Trigger:** When a tool fails, times out, or is denied; useful for error tracking and recovery.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `tool_name` | `string` | Docs do not say | Tool type/name. |
| `tool_input` | `object` | Docs do not say | Tool input. |
| `tool_use_id` | `string` | Docs do not say | Tool call ID. |
| `cwd` | `string` | Docs do not say | Current working directory. |
| `error_message` | `string` | Docs do not say | Failure description. |
| `failure_type` | `string` enum | Docs do not say | `error`, `timeout`, or `permission_denied`. |
| `duration` | `number` | Docs do not say | Milliseconds until failure. |
| `is_interrupt` | `boolean` | Docs do not say | Whether user interrupt/cancellation caused failure. |

**Output:**

```json
{
  "additional_context": "Tests time out on CI runners. Retry with --maxWorkers=2."
}
```

**Can block?** No documented ability to block an already-failed/denied tool. **Can modify?** Can inject recovery context; no replacement failure/tool output is documented. **Cloud:** Yes.

Source: [Cursor Hooks — `postToolUseFailure`](https://cursor.com/docs/hooks).

### 3.6 `subagentStart`

**Trigger:** Before spawning a subagent (Task tool); can allow or deny creation.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `subagent_id` | `string` | Docs do not say | Unique subagent instance ID. |
| `subagent_type` | `string` | Docs do not say | Type such as `generalPurpose`, `explore`, or `shell`. |
| `task` | `string` | Docs do not say | Task description given to the subagent. |
| `parent_conversation_id` | `string` | Docs do not say | Parent conversation ID. |
| `tool_call_id` | `string` | Docs do not say | Tool call that triggered the subagent. |
| `subagent_model` | `string` | Docs do not say | Model the subagent will use. |
| `is_parallel_worker` | `boolean` | Docs do not say | Whether it is a parallel worker. |
| `git_branch` | `string` | Optional | Branch the subagent will operate on, if applicable. |

**Output:**

```json
{
  "permission": "allow",
  "user_message": "<message shown when denied>"
}
```

`permission` accepts `allow` or `deny`; `ask` is unsupported and treated as `deny`.

**Can block?** Yes, denies subagent creation. **Can modify?** No tool-input rewrite is documented. **Cloud:** Yes.

Source: [Cursor Hooks — `subagentStart`](https://cursor.com/docs/hooks).

### 3.7 `subagentStop`

**Trigger:** When a subagent completes, errors, or is aborted; can initiate follow-up actions.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `subagent_type` | `string` | Docs do not say | Subagent type. |
| `status` | `string` enum | Docs do not say | `completed`, `error`, or `aborted`. |
| `task` | `string` | Docs do not say | Task description. |
| `description` | `string` | Docs do not say | Short purpose description. |
| `summary` | `string` | Docs do not say | Subagent output summary. |
| `duration_ms` | `number` | Docs do not say | Execution duration. |
| `message_count` | `number` | Docs do not say | Number of messages exchanged. |
| `tool_call_count` | `number` | Docs do not say | Number of subagent tool calls. |
| `loop_count` | `number` | Docs do not say | Prior `subagentStop` follow-ups for this subagent; starts at 0. |
| `modified_files` | array of strings | Docs do not say | Files modified by subagent. |
| `agent_transcript_path` | `string \| null` | Docs do not say | Separate subagent transcript path. |

**Output:**

```json
{
  "followup_message": "<auto-continue with this message>"
}
```

`followup_message` is optional and is consumed only when status is `completed`. It can start a loop-style flow and shares the `stop` hook’s configurable per-script `loop_limit` (Cursor default 5).

**Can block?** No documented way to undo a completed/error/aborted subagent; can initiate a follow-up only on completion. **Can modify?** Adds a follow-up message. **Cloud:** Yes.

Source: [Cursor Hooks — `subagentStop`](https://cursor.com/docs/hooks).

### 3.8 `beforeShellExecution`

**Trigger:** Before a shell command executes. If `preToolUse` is also configured for `Shell`, both generic and shell-specific hooks match; relative ordering is not stated.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `command` | `string` | Docs do not say | Full terminal command. |
| `cwd` | `string` | Docs do not say | Current working directory. |
| `sandbox` | `boolean` | Docs do not say | Whether command is sandboxed. |
| `tool_use_id` | Type is not stated | Docs do not say | Matches the corresponding `preToolUse` call; added in the IDE release notes. |

**Output:**

```json
{
  "permission": "allow",
  "user_message": "<message shown in client>",
  "agent_message": "<message sent to agent>"
}
```

The general permission vocabulary includes `allow`, `deny`, and `ask`. The docs describe `ask` as a schema value here but do not specify exactly what UI confirmation occurs, whether it is always honored, or how it interacts with Auto-Run modes. `user_message` and `agent_message` are documented as the user-facing and agent-facing messages when denied.

**Can block?** Yes, `deny`; `ask` behavior is not fully specified. **Can modify?** No command-rewrite field is documented in this hook’s output. **Cloud:** Yes (command hooks).

Sources: [Cursor Hooks — `beforeShellExecution`](https://cursor.com/docs/hooks); [Cursor IDE release notes](https://cursor.com/docs/release-notes/ide) (`tool_use_id` addition).

### 3.9 `afterShellExecution`

**Trigger:** After a shell command executes; useful for audit/metrics.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `command` | `string` | Docs do not say | Full executed command. |
| `output` | `string` | Docs do not say | Full captured terminal output. |
| `duration` | `number` | Docs do not say | Milliseconds spent executing; excludes approval wait time. |
| `sandbox` | `boolean` | Docs do not say | Whether command ran sandboxed. |
| `tool_use_id` | Type is not stated | Docs do not say | Matches the corresponding `preToolUse` call; added in the IDE release notes. |

**Output:** No output schema is shown.

**Can block?** No documented pre-execution blocking; fires after execution. **Can modify?** No structured output modification is documented. **Cloud:** Yes.

Sources: [Cursor Hooks — `afterShellExecution`](https://cursor.com/docs/hooks); [Cursor IDE release notes](https://cursor.com/docs/release-notes/ide) (`tool_use_id` addition).

### 3.10 `beforeMCPExecution`

**Trigger:** Before an MCP tool executes. If `preToolUse` is configured for MCP tools, both can match; docs do not define their relative order.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `tool_name` | `string` | Docs do not say | MCP tool name. |
| `tool_input` | `string` | Docs do not say | JSON parameters string passed to the tool. |
| `mcp_server_name` | `string` | Docs do not say | Server key from `mcp.json`. |
| `mcp_server_url` | `string` | Conditional | Present for HTTP/SSE servers. |
| `url` | `string` | Conditional | Alias for `mcp_server_url`, present for HTTP/SSE servers. |
| `command` | `string` | Conditional | Stdio launch command and arguments joined with spaces; present for stdio servers. |
| `tool_use_id` | Type is not stated | Docs do not say | Matches the corresponding `preToolUse` call; added in the IDE release notes. |

For HTTP/SSE servers, `url` and `mcp_server_url` are shown; for stdio servers, `command` is shown. The docs recommend identifying a server by `mcp_server_name` (and `tool_name`), not `command`, which may differ across installs.

**Output:**

```json
{
  "permission": "allow",
  "user_message": "<message shown in client>",
  "agent_message": "<message sent to agent>"
}
```

The reference lists `allow`, `deny`, and `ask` for this output. It recommends `failClosed: true` for security-critical MCP hooks.

**Can block?** Yes, `deny`; exact `ask` UI behavior is not specified. **Can modify?** No MCP input rewrite field is documented here. **Cloud:** No for Cursor-managed Cloud Agents; command-based project hooks run on Self-Hosted Machines.

Sources: [Cursor Hooks — `beforeMCPExecution`](https://cursor.com/docs/hooks); [Cursor IDE release notes](https://cursor.com/docs/release-notes/ide); [Hooks on Pools](https://cursor.com/docs/cloud-agent/self-hosted/pool).

### 3.11 `afterMCPExecution`

**Trigger:** After an MCP tool executes; input includes tool parameters and full JSON result.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `tool_name` | `string` | Docs do not say | Executed MCP tool name. |
| `tool_input` | `string` | Docs do not say | JSON parameters string passed to the tool. |
| `mcp_server_name` | `string` | Docs do not say | Server key in `mcp.json`. |
| `mcp_server_url` | `string` | Conditional | Present for HTTP/SSE servers. |
| `result_json` | `string` | Docs do not say | JSON string of tool response. |
| `duration` | `number` | Docs do not say | Milliseconds; excludes approval wait time. |
| `tool_use_id` | Type is not stated | Docs do not say | Matches the corresponding `preToolUse` call; added in the IDE release notes. |

**Output:** No output schema is shown.

**Can block?** No documented pre-execution blocking; fires after execution. **Can modify?** No structured output modification is documented; unlike `postToolUse`, no response-replacement field is listed. **Cloud:** No for Cursor-managed Cloud Agents; command-based hooks run on Self-Hosted Machines.

Sources: [Cursor Hooks — `afterMCPExecution`](https://cursor.com/docs/hooks); [Cursor IDE release notes](https://cursor.com/docs/release-notes/ide); [Hooks on Pools](https://cursor.com/docs/cloud-agent/self-hosted/pool).

### 3.12 `beforeReadFile`

**Trigger:** Before Agent reads a file; the payload includes file contents and attachments. Cursor describes it as access control for preventing sensitive files from being sent to the model.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `file_path` | `string` | Docs do not say | Absolute path to file being read. |
| `content` | `string` | Docs do not say | Full file contents. |
| `attachments` | array of objects | Docs do not say | Prompt context attachments; each object has `type` (`"file"` or `"rule"`) and `file_path` (absolute path). |

**Output:**

```json
{
  "permission": "allow",
  "user_message": "<message shown when denied>"
}
```

`permission` is `allow` or `deny`; `user_message` is optional.

**Can block?** Yes, denies the read. **Can modify?** The documented output has no replacement-content field. The reference therefore documents blocking, not redacting-and-passing modified contents. This contrasts with general security copy describing redaction; see §7. **Cloud:** Yes.

Source: [Cursor Hooks — `beforeReadFile`](https://cursor.com/docs/hooks); [LLM Safety and Controls](https://cursor.com/docs/enterprise/llm-safety-and-controls).

### 3.13 `afterFileEdit`

**Trigger:** After Agent edits a file; useful for formatters or accounting.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `file_path` | `string` | Docs do not say | Absolute path to edited file. |
| `edits` | array of objects | Docs do not say | Each edit has `old_string` (`string`) and `new_string` (`string`). |
Cursor's Jan. 8, 2026 CLI changelog says `afterFileEdit` was fixed to provide `old_string` with the file's previous content for proper diff capture.

**Output:** No output schema is shown.

**Can block?** No pre-edit block is documented; fires after edit. **Can modify?** No structured replacement output is documented. A command may perform external side effects, but the hook protocol’s effect on the completed edit is not specified. **Cloud:** Yes.

Source: [Cursor Hooks — `afterFileEdit`](https://cursor.com/docs/hooks).

### 3.14 `beforeSubmitPrompt`

**Trigger:** After the user hits Send but before the backend request; can prevent submission.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `prompt` | `string` | Docs do not say | User’s prompt text. |
| `attachments` | array of objects | Docs do not say | Each object has `type` (`"file"` or `"rule"`) and `file_path` (absolute path). |

**Output:**

```json
{
  "continue": true,
  "user_message": "<message shown to user when blocked>"
}
```

`continue` is a boolean deciding whether submission proceeds; `user_message` is optional and shown when blocked. No `updated_prompt` field is documented in the current Hook reference.

**Can block?** Yes, with `continue: false`. **Can modify?** The current reference has no prompt-replacement field. **Cloud:** Yes.

**Documented mismatch:** Cursor’s Jan. 22, 2026 changelog says `beforeSubmitPrompt` can “modify prompts,” but the live reference’s output schema lists only `continue` and `user_message`. The changelog does not name a rewrite field or explain how to modify the submitted text.

Sources: [Cursor Hooks — `beforeSubmitPrompt`](https://cursor.com/docs/hooks); [Cursor 2.4 changelog, Jan. 22, 2026](https://cursor.com/changelog/2-4).

### 3.15 `preCompact`

**Trigger:** Before context-window compaction/summarization. The reference calls it observational and says it cannot block or modify compaction.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `trigger` | `string` enum | Docs do not say | `auto` or `manual`. |
| `context_usage_percent` | `number` | Docs do not say | Current context usage, 0–100. |
| `context_tokens` | `number` | Docs do not say | Current context token count. |
| `context_window_size` | `number` | Docs do not say | Maximum context tokens. |
| `message_count` | `number` | Docs do not say | Conversation message count. |
| `messages_to_compact` | `number` | Docs do not say | Number of messages to summarize. |
| `is_first_compaction` | `boolean` | Docs do not say | Whether this is first compaction. |

**Output:**

```json
{
  "user_message": "<message to show when compaction occurs>"
}
```

`user_message` is optional.

**Can block?** No. **Can modify?** No compaction modification; may provide a user message. **Cloud:** Yes.

Source: [Cursor Hooks — `preCompact`](https://cursor.com/docs/hooks).

### 3.16 `stop`

**Trigger:** When the agent loop ends. Can automatically submit a follow-up user message.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `status` | `string` enum | Docs do not say | `completed`, `aborted`, or `error`. |
| `loop_count` | `number` | Docs do not say | Number of automatic follow-ups already triggered; starts at 0. |

**Output:**

```json
{
  "followup_message": "<message text>"
}
```

`followup_message` is optional; when non-empty, Cursor automatically submits it as the next user message. Default loop limit is 5 per script; set `loop_limit: null` for no cap.

**Can block?** The documented effect is automatic follow-up, not a standalone “deny stop” decision. **Can modify?** Adds a follow-up user message. **Cloud:** Yes.

Source: [Cursor Hooks — `stop`](https://cursor.com/docs/hooks).

### 3.17 `afterAgentResponse`

**Trigger:** After the agent has completed an assistant message.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `text` | `string` | Docs do not say | Assistant final text. |

**Output:** The event reference shows no output schema.

**Can block?** Docs do not say; no blocking output is documented. **Can modify?** Docs do not say; no response-replacement output is documented. **Cloud:** Yes.

Source: [Cursor Hooks — `afterAgentResponse`](https://cursor.com/docs/hooks).

### 3.18 `afterAgentThought`

**Trigger:** After an agent thinking block completes. The payload exposes the fully aggregated thinking text.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `text` | `string` | Docs do not say | Fully aggregated thinking text. |
| `duration_ms` | `number` | Optional | Thinking-block duration in milliseconds. |

**Output:** No output fields are currently supported.

**Can block?** No documented output. **Can modify?** No. **Cloud:** Yes.

Source: [Cursor Hooks — `afterAgentThought`](https://cursor.com/docs/hooks).

### 3.19 `beforeTabFileRead`

**Trigger:** Before Tab inline completions read a file. It fires for Tab only, not Agent, and does not include `attachments`.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `file_path` | `string` | Docs do not say | Absolute path to file. |
| `content` | `string` | Docs do not say | File contents. |

**Output:**

```json
{
  "permission": "allow"
}
```

The schema allows `allow` or `deny`.

**Can block?** Yes, with `deny`. **Can modify?** No replacement-content field is documented. **Cloud:** No; Tab completion is IDE-only.

Source: [Cursor Hooks — `beforeTabFileRead`](https://cursor.com/docs/hooks).

### 3.20 `afterTabFileEdit`

**Trigger:** After Tab inline completions edit a file. It fires for Tab only, not Agent.

**Input fields, in addition to common base:**

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `file_path` | `string` | Docs do not say | Absolute path to edited file. |
| `edits` | array of objects | Docs do not say | Each has `old_string`, `new_string`, `range`, `old_line`, and `new_line`. `range` has `start_line_number`, `start_column`, `end_line_number`, `end_column`. The docs do not explicitly type those four coordinates; the examples use numbers. |

**Output:** No output fields are currently supported.

**Can block?** No documented output. **Can modify?** No. **Cloud:** No; Tab completion is IDE-only.

Source: [Cursor Hooks — `afterTabFileEdit`](https://cursor.com/docs/hooks).

### 3.21 `workspaceOpen`

**Trigger:** Once when Cursor opens a workspace and again on every workspace-folder change; skipped when there are zero workspace folders. Runs in the desktop app and CLI, independently of an agent session.

**Input:** The full per-session common schema does not apply. The documented fields are:

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `hook_event_name` | `string` | Docs do not say | Exact value `workspaceOpen`. |
| `cursor_version` | `string` | Docs do not say | Cursor version. |
| `workspace_roots` | array of strings | Docs do not say | Absolute workspace roots. |
| `user_email` | `string \| null` | Docs do not say | Authenticated user email, if available. |

The reference explicitly says the input omits `conversation_id`, `generation_id`, `model`, `session_id`, and `transcript_path`.

**Output:**

```json
{
  "pluginPaths": ["<absolute path>", "..."]
}
```

`pluginPaths` is an optional array of strings containing absolute plugin-directory paths to load for the current workspace.

**Can block?** No block output is documented. **Can modify?** Can request additional plugin paths for the workspace. **Cloud:** No; IDE lifecycle only.

Source: [Cursor Hooks — common schema and `workspaceOpen`](https://cursor.com/docs/hooks).

## 4. Prompt-based hooks

Cursor documents two execution types, command and prompt. A prompt hook is an LLM evaluation of a natural-language condition. The Hooks reference demonstrates it on `beforeShellExecution`, but does not enumerate which other hook events permit `type: "prompt"`. The Third Party Hooks page says prompt-based hooks are supported in the Claude compatibility layer, but does not give a per-event prompt-hook matrix.

Documented prompt hook fields:

| Field | Type | Required/optional | Meaning |
| --- | --- | --- | --- |
| `type` | `"prompt"` | Optional config field; selects prompt execution | Prompt-based rather than command-based execution. |
| `prompt` | Type/requiredness not specified | Shown in the example | Natural-language evaluation instruction. |
| `timeout` | `number` | Optional; platform default if omitted | Seconds; example uses `10`. |
| `model` | Type not specified | Optional | Overrides default LLM model. |

Documented evaluation result:

```json
{
  "ok": true,
  "reason": "<optional explanation>"
}
```

The docs say prompt hooks use a fast model by default, but do not name that model or specify how `ok`/`reason` are mapped onto each event’s native output fields (`permission`, `continue`, etc.).

`$ARGUMENTS` is replaced with the hook input JSON. If `$ARGUMENTS` is absent from the prompt template, Cursor appends hook input automatically. Timeout is configured in seconds; docs do not specify whether prompt timeout/failure follows the same `failClosed` behavior as command hooks.

Prompt-based hooks do not run in Cursor-managed Cloud Agents or Self-Hosted Machines workers; those environments run command-based hooks only. The stated reason for Cursor-managed Cloud Agents is that prompt hooks require authentication wiring unavailable in that execution environment.

Sources: [Cursor Hooks — Hook Types](https://cursor.com/docs/hooks); [Third Party Hooks — Supported Features](https://cursor.com/docs/reference/third-party-hooks); [Hooks on Pools](https://cursor.com/docs/cloud-agent/self-hosted/pool).

## 5. Cloud agent support

Cursor-managed Cloud Agents can begin in a read-only exploratory environment. Hooks do not run during those turns; they start once the agent has a writable environment. Cloud agents run command hooks only.

| Hook | Cursor-managed Cloud Agent | Stated reason if unsupported |
| --- | --- | --- |
| `sessionStart` | No | Deferred while agents can start read-only; hook would run too late for true session start. |
| `sessionEnd` | No | No editor-lifetime session boundary; tied to IDE session. |
| `preToolUse` | Yes | — |
| `postToolUse` | Yes | — |
| `postToolUseFailure` | Yes | — |
| `subagentStart` | Yes | — |
| `subagentStop` | Yes | — |
| `beforeShellExecution` | Yes | — |
| `afterShellExecution` | Yes | — |
| `beforeMCPExecution` | No | Deferred because hooks may not load in read-only turns and MCP timing is unclear. |
| `afterMCPExecution` | No | Same reason as `beforeMCPExecution`. |
| `beforeReadFile` | Yes | — |
| `afterFileEdit` | Yes | — |
| `beforeSubmitPrompt` | Yes | — |
| `preCompact` | Yes | — |
| `stop` | Yes | — |
| `afterAgentResponse` | Yes | — |
| `afterAgentThought` | Yes | — |
| `beforeTabFileRead` | No | Tab completions are an IDE feature. |
| `afterTabFileEdit` | No | Tab completions are an IDE feature. |
| `workspaceOpen` | No | IDE lifecycle hook. |

Cloud configuration sources: project `.cursor/hooks.json`, Enterprise Team hooks from the dashboard, and enterprise-managed hooks. User-level `~/.cursor/hooks.json` is unavailable because Cloud Agent VMs do not have the local home-directory config.

**Self-Hosted Machines distinction:** Pools and My Machines run command-based project hooks from their workspace and, on Enterprise, Team and enterprise-managed hooks. Unlike Cursor-managed Cloud Agents, self-hosted workers do run `sessionStart` on claim and `sessionEnd` on release. Pool CLI flags `--on-session-start <command>` and `--on-session-end <command>` add one command for each event outside `.cursor/hooks.json`. Self-hosted workers do not run prompt hooks, Tab hooks, or `workspaceOpen`. On a claim, the `sessionStart` payload has additional `repo_urls` (array of URLs) and `repos` (one object per repository, with `repo_url`, `primary`, and `ref` when the request pins a branch/commit); these fields are not in the general event reference.

Sources: [Cursor Hooks — Cloud agent support](https://cursor.com/docs/hooks); [Team Pools — Hooks](https://cursor.com/docs/cloud-agent/self-hosted/pool); [My Machines — Hooks](https://cursor.com/docs/cloud-agent/self-hosted/my-machines).

## 6. Third-party hook compatibility: Claude Code

### 6.1 Loading and configuration shape

Cursor loads Claude Code hooks when **Include Third-Party Plugins, Skills, and Other Configs** is enabled in Cursor Settings → Agents → Third-Party Imports; the page says this setting is on by default. The files Cursor documents are:

| Scope | File | Description |
| --- | --- | --- |
| Project local | `.claude/settings.local.json` | Project-specific, gitignored overrides. |
| Project | `.claude/settings.json` | Project hooks checked into the repo. |
| User | `~/.claude/settings.json` | User-global hooks. |

The compatibility example uses a top-level `hooks` object. Each event maps to an array of matcher groups; a group contains `matcher` and nested `hooks`, with command handlers represented by `type: "command"` and `command`.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Shell",
        "hooks": [
          {
            "type": "command",
            "command": "./hooks/validate-shell.sh"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": ".*",
        "hooks": [
          {
            "type": "command",
            "command": "./hooks/audit.sh"
          }
        ]
      }
    ]
  }
}
```

Cursor’s combined priority order is Enterprise → Team → native project `.cursor/hooks.json` → native user `~/.cursor/hooks.json` → Claude project-local `.claude/settings.local.json` → Claude project `.claude/settings.json` → Claude user `~/.claude/settings.json`. All matching sources run. Note the merge-order contradiction in §1.2/§7: this page says higher-priority sources take precedence, whereas the main Hooks reference says “last response wins” for non-permission fields and explicitly says lower-priority sources override higher-priority ones.

### 6.2 Event-name and tool-name mapping

| Claude Code event | Cursor hook | Cursor compatibility page |
| --- | --- | --- |
| `PreToolUse` | `preToolUse` | Supported |
| `PostToolUse` | `postToolUse` | Supported |
| `UserPromptSubmit` | `beforeSubmitPrompt` | Supported |
| `Stop` | `stop` | Supported |
| `SubagentStop` | `subagentStop` | Supported |
| `SessionStart` | `sessionStart` | Supported |
| `SessionEnd` | `sessionEnd` | Supported |
| `PreCompact` | `preCompact` | Supported |
| `Notification` | — | Explicitly unsupported |
| `PermissionRequest` | — | Explicitly unsupported |

| Claude Code tool name | Cursor tool name | Supported |
| --- | --- | --- |
| `Bash` | `Shell` | Yes |
| `Read` | `Read` | Yes |
| `Write` | `Write` | Yes |
| `Edit` | `Write` | Yes |
| `Grep` | `Grep` | Yes |
| `Task` | `Task` | Yes |
| `WebFetch` | `WebFetch` | Yes |
| `WebSearch` | `WebSearch` | Yes |
| `Glob` | — | No |

Cursor says tool matchers are regex patterns. Its docs do not say whether tool-name mapping changes the `tool_name` value in the stdin payload or only the matcher target; they also do not specify broad field-by-field input-envelope translation for imported Claude events.

### 6.3 Response translation

For `PreToolUse`, Cursor supports Claude’s nested format and its own flat response format:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "Blocked by policy",
    "updatedInput": { "command": "npm ci" }
  }
}
```

```json
{
  "permission": "deny",
  "user_message": "Blocked by policy",
  "updated_input": { "command": "npm ci" }
}
```

Cursor explicitly maps `permissionDecision` → `permission`, `permissionDecisionReason` → `user_message`, and `updatedInput` → `updated_input`. It does not document a mapping from `permissionDecisionReason` to `agent_message`.

For `Stop` and `SubagentStop`, both nested and legacy flat Claude response shapes are accepted:

```json
{
  "hookSpecificOutput": {
    "decision": "block",
    "reason": "Tasks incomplete, continue working"
  }
}
```

```json
{
  "decision": "block",
  "reason": "Tasks incomplete, continue working"
}
```

Cursor treats `decision: "block"` with `reason` as an automatic follow-up, equivalent to its native output:

```json
{
  "followup_message": "Tasks incomplete, continue working"
}
```

Cursor also documents exit code `2` as blocking; `0` uses JSON output; other exit codes fail open by default.

### 6.4 Compatibility limitations and unknowns

- Cursor’s support table explicitly lists eight supported event mappings and two explicitly unsupported events (`Notification`, `PermissionRequest`). Claude Code currently documents a larger event set; Cursor’s compatibility page does not state support status for every other upstream event, so unlisted events are “docs do not say,” not presumed supported.
- `subagentStart` is available in Cursor-native hooks, but Claude Code has only `SubagentStop`.
- `loop_limit` and Team/Enterprise dashboard hook distribution are Cursor-native-only.
- Cursor’s compatibility page says command hooks, prompt hooks, nested `hookSpecificOutput`, exit-code-2 blocking, regex tool matchers, and timeout configuration are supported.
- Cursor’s page does not spell out exact input-field transformations, all upstream Claude hook configuration options accepted/ignored, working directory for imported scripts, or how Claude `allow`/`ask` decisions interact with Cursor’s own permission system.

For Claude Code’s own current event/configuration contract (not Cursor’s compatibility layer), see Anthropic’s [Hooks reference](https://docs.anthropic.com/en/docs/claude-code/hooks). Cursor-specific loading, mapping, and response translation are sourced from Cursor’s [Third Party Hooks](https://cursor.com/docs/reference/third-party-hooks) page.

## 7. User-facing behavior, observability, and notable inconsistencies

### User-facing behavior

- `user_message` is documented as the message shown in the client for a denied action; `agent_message` is fed back to the agent on denial. If multiple hooks set either field, the values concatenate.
- `beforeSubmitPrompt` can block submission with `continue: false`; its optional `user_message` is shown to the user.
- `preCompact.user_message` is described as a message to show when compaction occurs.
- `stop.followup_message` is automatically submitted as a next user message; `subagentStop.followup_message` is only consumed when the subagent status is `completed`.
- `workspaceOpen.pluginPaths` can add plugin directories for the current workspace.
- A permission value of `ask` appears in some permission schemas, but UI confirmation details are not given in the Hooks reference. For `preToolUse`, `ask` is explicitly accepted by schema but not enforced; for `subagentStart`, `ask` is treated as `deny`. The docs do not say what confirmation prompt a `beforeShellExecution` or `beforeMCPExecution` `ask` response produces or whether it is honored across permission/Auto-Run modes.

### Logs and debugging

- Customize has a Hooks tab; the Hooks output channel shows configured/executed hooks and errors.
- Troubleshooting advises checking valid JSON, relative command paths, and exit code `2` for blocking; restart Cursor if watched config reload does not resolve loading.
- The docs do not say where hook stderr is routed or whether arbitrary stdout beyond the JSON response is captured.

### Surprising or inconsistent documentation

1. **`ask` is not uniformly actionable.** `preToolUse` explicitly says `ask` is not enforced, while `subagentStart` treats it as deny. `beforeShellExecution` / `beforeMCPExecution` list `ask` but do not explain resulting UI behavior.
2. **Prompt rewriting is advertised without a schema field.** The Jan. 22, 2026 changelog says `beforeSubmitPrompt` can modify prompts; its live output schema only lists `continue` and `user_message`. No replacement prompt field is documented.
3. **Redaction is described without a content-replacement output.** Introductory/security material describes scanning/redacting file contents; `beforeReadFile` and `beforeTabFileRead` accept content but only document permission outputs. No documented field returns redacted content to the agent.
4. **Source-precedence descriptions conflict.** The Third Party Hooks page says higher-priority sources take precedence; the main Hooks page says last response wins for other fields and that lower-priority sources override higher-priority ones.
5. **Parallel execution vs. “last response.”** The CLI changelog says hook commands run in parallel with merged responses; the reference does not explain deterministic “last” response ordering, especially for same-priority hooks.
6. **Configuration field inconsistency.** Native per-script options call `command` required, but prompt-hook examples omit it. Plugin hook examples omit `version` although native config calls it required.
7. **Prompt support is not enumerated per event.** The docs show a prompt hook for `beforeShellExecution` and say the type is generally available, but do not list the events that accept it.
8. **Field requiredness is incomplete.** Most event references show payload fields without stating whether each is required. This report does not infer requiredness.
9. **Matcher details are incomplete.** Regex flavor, case sensitivity, and anchoring/search semantics are not documented.
10. **Exact hook environment inheritance is incomplete.** Cursor names its own environment variables and working directory by source, but does not document the full inherited process environment.
11. **Pool payload extends the reference.** Self-Hosted `sessionStart` receives `repo_urls` and `repos`, which are not present in the general sessionStart schema.
12. **Changelog history vs. current status:** Cursor 1.7 (Sep. 29, 2025) launched Hooks as “beta”; the current Hooks reference does not state whether the feature is still beta.

Sources: [Cursor Hooks](https://cursor.com/docs/hooks); [LLM Safety and Controls](https://cursor.com/docs/enterprise/llm-safety-and-controls); [Cursor 2.4 changelog](https://cursor.com/changelog/2-4); [Third Party Hooks](https://cursor.com/docs/reference/third-party-hooks); [Jan. 8, 2026 CLI changelog](https://cursor.com/changelog/cli-jan-08-2026); [Cursor 1.7 changelog](https://cursor.com/changelog/1-7).

## 8. Compact summary

“Input fields” below are event-specific additions to the common base in §2.1, except `workspaceOpen`, which uses its reduced envelope. Cloud support refers to Cursor-managed Cloud Agents; Self-Hosted Machines exceptions are in §5.

| Hook name | Category | Can block? | Can modify? | Input fields | Output fields | Cloud supported? |
| --- | --- | --- | --- | --- | --- | --- |
| `sessionStart` | Agent lifecycle | No | `env`, `additional_context` | `session_id`, `is_background_agent`, `composer_mode` | `env`, `additional_context`; ignored `continue`, `user_message` | No |
| `sessionEnd` | Agent lifecycle | No | No documented output effect | `session_id`, `reason`, `duration_ms`, `is_background_agent`, `final_status`, `error_message` | None | No |
| `preToolUse` | Agent/tool permission | Yes: `deny`; `ask` not enforced | `updated_input` | `tool_name`, `tool_input`, `tool_use_id`, `cwd`, `agent_message` | `permission`, `user_message`, `agent_message`, `updated_input` | Yes |
| `postToolUse` | Agent/tool result | Not before execution | MCP output only; can inject context | `tool_name`, `tool_input`, `tool_output`, `tool_use_id`, `cwd`, `duration` | `updated_mcp_tool_output`, `additional_context` | Yes |
| `postToolUseFailure` | Agent/tool failure | No documented block | Can inject recovery context | `tool_name`, `tool_input`, `tool_use_id`, `cwd`, `error_message`, `failure_type`, `duration`, `is_interrupt` | `additional_context` | Yes |
| `subagentStart` | Subagent permission | Yes: deny; `ask` treated as deny | No documented rewrite | `subagent_id`, `subagent_type`, `task`, `parent_conversation_id`, `tool_call_id`, `subagent_model`, `is_parallel_worker`, `git_branch` | `permission`, `user_message` | Yes |
| `subagentStop` | Subagent lifecycle | Follow-up, not a documented stop denial | `followup_message` (only consumed on `completed`) | `subagent_type`, `status`, `task`, `description`, `summary`, `duration_ms`, `message_count`, `tool_call_count`, `loop_count`, `modified_files`, `agent_transcript_path` | `followup_message` | Yes |
| `beforeShellExecution` | Shell permission | Yes: deny; ask behavior unspecified | No command rewrite field documented | `command`, `cwd`, `sandbox`, `tool_use_id` | `permission`, `user_message`, `agent_message` | Yes |
| `afterShellExecution` | Shell result | No documented pre-block | No structured output modification documented | `command`, `output`, `duration`, `sandbox`, `tool_use_id` | No output schema shown | Yes |
| `beforeMCPExecution` | MCP permission | Yes: deny; ask behavior unspecified | No input rewrite field documented | `tool_name`, `tool_input`, `mcp_server_name`, conditional `url`/`mcp_server_url` or `command`, `tool_use_id` | `permission`, `user_message`, `agent_message` | No |
| `afterMCPExecution` | MCP result | No documented pre-block | No replacement field in this event | `tool_name`, `tool_input`, `mcp_server_name`, conditional `mcp_server_url`, `result_json`, `duration`, `tool_use_id` | No output schema shown | No |
| `beforeReadFile` | Agent file permission | Yes: deny | No content replacement field documented | `file_path`, `content`, `attachments` | `permission`, optional `user_message` | Yes |
| `afterFileEdit` | Agent file result | No documented pre-block | No structured rewrite documented | `file_path`, `edits[{old_string,new_string}]` | No output schema shown | Yes |
| `beforeSubmitPrompt` | Prompt submission | Yes: `continue: false` | Changelog says modify prompt; current schema has no rewrite field | `prompt`, `attachments` | `continue`, optional `user_message` | Yes |
| `preCompact` | Context lifecycle | No | Cannot modify compaction; may show message | `trigger`, `context_usage_percent`, `context_tokens`, `context_window_size`, `message_count`, `messages_to_compact`, `is_first_compaction` | Optional `user_message` | Yes |
| `stop` | Agent lifecycle | Follow-up, not a documented stop denial | `followup_message` | `status`, `loop_count` | Optional `followup_message` | Yes |
| `afterAgentResponse` | Agent response | Docs do not say; no output shown | Docs do not say; no output shown | `text` | No output schema shown | Yes |
| `afterAgentThought` | Agent reasoning | No documented output | No | `text`, optional `duration_ms` | No fields currently supported | Yes |
| `beforeTabFileRead` | Tab file permission | Yes: deny | No content replacement field documented | `file_path`, `content` | `permission` (`allow`/`deny`) | No |
| `afterTabFileEdit` | Tab file result | No documented output | No | `file_path`, `edits` (including `old_string`, `new_string`, `range`, `old_line`, `new_line`) | No fields currently supported | No |
| `workspaceOpen` | App lifecycle | No documented block | Adds `pluginPaths` | `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email` | Optional `pluginPaths` | No |

Sources: [Cursor Hooks — hook reference and Cloud support](https://cursor.com/docs/hooks); [Hooks on Pools](https://cursor.com/docs/cloud-agent/self-hosted/pool); [Cursor IDE release notes](https://cursor.com/docs/release-notes/ide).

## Source index

- [Cursor Hooks](https://cursor.com/docs/hooks) — live reference for configuration, common schema, event contracts, Cloud support, environment variables, and troubleshooting. No document date/version shown.
- [Third Party Hooks](https://cursor.com/docs/reference/third-party-hooks) — Claude Code file loading, mappings, nested/flat response translation, and limitations. No document date/version shown.
- [Plugins reference](https://cursor.com/docs/reference/plugins) — Cursor Plugin manifest and `hooks/hooks.json` discovery. No document date/version shown.
- [Customize Cursor](https://cursor.com/docs/customize-cursor) — Customize scopes and extension components. No document date/version shown.
- [Team Pools](https://cursor.com/docs/cloud-agent/self-hosted/pool) and [My Machines](https://cursor.com/docs/cloud-agent/self-hosted/my-machines) — self-hosted hook execution and pool lifecycle details. No document date/version shown.
- [Hooks for security and platform teams](https://cursor.com/blog/hooks-partners) — Cursor partner integrations, Dec. 22, 2025.
- [Cursor 1.7 changelog](https://cursor.com/changelog/1-7) — Hooks launch as beta, Sep. 29, 2025.
- [Cursor 2.1 changelog](https://cursor.com/changelog/2-1) — project-level hooks, Nov. 21, 2025.
- [Cursor 2.4 changelog](https://cursor.com/changelog/2-4) — hook coverage, prompt-modification claim, CLI/MDM compatibility, Jan. 22, 2026.
- [Jan. 8, 2026 CLI changelog](https://cursor.com/changelog/cli-jan-08-2026) — parallel execution, merged responses, and `afterFileEdit.old_string`.
- [Side Chats and Conversation Search changelog](https://cursor.com/changelog/side-chat) — new Cloud Agent hooks for prompts, responses, thinking, subagents, compaction, and turn completion.
- [Cursor IDE release notes](https://cursor.com/docs/release-notes/ide) — hook correlation field `tool_use_id` and transcript-path note.
- [Anthropic Claude Code Hooks reference](https://docs.anthropic.com/en/docs/claude-code/hooks) — upstream Claude Code event/configuration contract; not evidence of Cursor adapter behavior where Cursor’s own page is silent.
