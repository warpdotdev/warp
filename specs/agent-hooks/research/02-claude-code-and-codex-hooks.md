# Agent hook formats: Claude Code, Codex, and Cursor

Research snapshot: 2026-10-19. This report is fact-finding, not an abstraction design. It uses the official vendor documentation as the behavior source and official SDK/repository schemas only to clarify field types or call out schema/doc discrepancies. The Claude Code and Codex docs pages do not show an overall release version or publication date; feature-specific version requirements are retained where documented. Cursor’s hook file schema identifies itself as version `1`.

## A. Claude Code hooks

Primary sources: [Hooks reference](https://code.claude.com/docs/en/hooks), [Hooks guide](https://code.claude.com/docs/en/hooks-guide), [Settings and precedence](https://code.claude.com/docs/en/settings), [Plugin components](https://code.claude.com/docs/en/plugins/components), [Plugin reference](https://code.claude.com/docs/en/plugins-reference), and [Agent SDK hooks](https://code.claude.com/docs/en/agent-sdk/hooks). The official [Python Agent SDK type declarations](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/types.py) are used only to establish TypedDict requiredness for the smaller SDK callback surface; that surface does not cover every current CLI hook event.

### Configuration locations and precedence

Claude Code’s settings-based hook object is nested under the top-level `hooks` key. The supported scopes documented for native settings are:

| Location | Scope / behavior |
| --- | --- |
| `~/.claude/settings.json` | User settings, applied across projects on the machine. |
| `.claude/settings.json` | Shared project settings, normally committed for teammates. |
| `.claude/settings.local.json` | Project-local personal settings; Claude Code git-ignores it when it creates it. |
| Managed policy settings | Organization settings delivered as `managed-settings.json`, MDM policy, or server-managed settings. |
| Plugin `hooks/hooks.json` | Loaded while that plugin is enabled. A plugin manifest can also provide a `hooks` path or inline hook config. |
| Skill or agent frontmatter | Hooks are active while the skill or agent component is active. |

The settings precedence order is **managed > command-line `--settings` > project-local > shared project > user**. Hook arrays are not simply replaced at each level: settings lists merge, all matching hooks are considered, and an identical handler defined in multiple settings files runs once. A copy from a plugin or skill remains separate. `allowManagedHooksOnly` can suppress user, project, and plugin hooks; hooks from plugins force-enabled by managed `enabledPlugins` are exempt. Settings-file hooks wait for workspace trust in interactive sessions; `-p` and SDK sessions treat the folder as trusted. See [settings precedence](https://code.claude.com/docs/en/settings#settings-precedence), [hook locations](https://code.claude.com/docs/en/hooks#hook-locations), and [plugin hooks](https://code.claude.com/docs/en/plugins/components#hooks).

### Configuration schema and handler fields

The native shape is event → matcher groups → handlers:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PROJECT_DIR}/.claude/hooks/check.sh",
            "args": []
          }
        ]
      }
    ]
  }
}
```

Exact field names and documented types/requiredness:

| Level | Field | Type | Required? | Meaning |
| --- | --- | --- | --- | --- |
| Settings / plugin wrapper | `hooks` | object keyed by event name | Required to define hooks | Contains event-to-matcher-group arrays. Settings use the `hooks` setting; plugin `hooks/hooks.json` also has a top-level `hooks` wrapper and may have top-level `description`. |
| Plugin wrapper | `description` | string | Optional | Human-readable description of plugin hooks. |
| Event entry | event name, e.g. `PreToolUse` | array of matcher groups | Required for that event entry | Names a lifecycle event. No configuration `version` field is documented for Claude hooks. |
| Matcher group | `matcher` | string | Optional | Selects event occurrences by event-specific value. |
| Matcher group | `hooks` | array of handlers | Required by the documented three-level shape | One or more handlers to invoke when the group matches. |
| All handler types | `type` | string enum | Yes | One of `command`, `http`, `mcp_tool`, `prompt`, or `agent`. |
| All handler types | `if` | string | Optional | One permission-rule expression evaluated against tool name and arguments for tool events only. It is not a general boolean expression. |
| All handler types | `timeout` | number (seconds) | Optional | Per-handler timeout. Defaults are 600 seconds for `command`, `http`, and `mcp_tool`; 30 for `prompt`; 60 for `agent`. Some events lower the default. |
| All handler types | `statusMessage` | string | Optional | Spinner status shown while the handler runs. |
| All handler types | `once` | boolean | Optional | Removes the hook after a successful first run. Only honored in skill frontmatter; ignored in settings and agent frontmatter. |
| `command` | `command` | string | Yes | Shell command, or executable name/path when `args` is present. |
| `command` | `args` | array of strings | Optional | When present, starts `command` directly with this argument vector, without a shell. |
| `command` | `async` | boolean | Optional | Runs in background without blocking. |
| `command` | `asyncRewake` | boolean | Optional | Runs in background and wakes Claude on exit code 2; stderr, or stdout if stderr is empty, becomes a system reminder. |
| `command` | `shell` | string enum | Optional | `bash` or `powershell`; defaults to Bash, or PowerShell on Windows when Git Bash is unavailable. Ignored with `args`. |
| `command` / `http` | `onFailure` | string enum | Optional | `continue` or `block` for handler failure/timeout. Requires Claude Code v2.1.295 or later. |
| `http` | `url` | string | Yes | POST destination for the event JSON. |
| `http` | `headers` | object of string values | Optional | Additional request headers; environment references are interpolated only when allowlisted. |
| `http` | `allowedEnvVars` | array of strings | Optional | Names permitted for environment-variable interpolation in `headers`; required for any interpolation. |
| `mcp_tool` | `server` | string | Yes | Configured MCP server name; plugin server names are scoped. |
| `mcp_tool` | `tool` | string | Yes | Tool name on that server. |
| `mcp_tool` | `input` | object | Optional | Tool arguments; string values can interpolate `${path}` from hook input JSON. |
| `prompt` / `agent` | `prompt` | string | Yes | LLM evaluation prompt; `$ARGUMENTS` inserts input JSON, otherwise the input is appended. |
| `prompt` / `agent` | `model` | string | Optional | Model override; otherwise background-functionality default. |
| `prompt` | `continueOnBlock` | boolean | Optional | If true on supported events, returns a failed evaluation reason to Claude and continues rather than ending the turn. Default false. |

`agent` hooks are explicitly experimental. `prompt` and `agent` use the structured response `{ "ok": true|false, "reason": "...", "impossible": true|false }`; `reason` is required when `ok` is false, and `impossible` is optional for prompt hooks on Stop/SubagentStop. The `continueOnBlock` field applies only to prompt hooks. See [handler fields](https://code.claude.com/docs/en/hooks#hook-handler-fields), [prompt hooks](https://code.claude.com/docs/en/hooks#prompt-based-hooks), and [agent hooks](https://code.claude.com/docs/en/hooks#agent-based-hooks).

### Matchers, execution, and environment

For ordinary matchers, `""`, `"*"`, and an omitted matcher match all. Strings containing only letters, digits, `_`, `-`, spaces, comma, and `|` are exact names or exact alternatives; any other character selects unanchored JavaScript-regex matching. Thus `Edit|Write` is an exact alternative list, while `^Notebook` is a regular expression. `FileChanged` and `StopFailure` have a narrower exact-match rule. The `matcher` value is tested against different event data: tool names for tool events, session source, end reason, notification type, subagent type, compaction trigger, model target, config source, and so on. Some events have no matcher support; the reference lists those explicitly. See [matcher patterns](https://code.claude.com/docs/en/hooks#matcher-patterns).

All matching handlers run in parallel. The same handler duplicated across settings files runs once; plugin/skill copies are distinct. The explicit decision precedence documented for `PreToolUse` is `deny > defer > ask > allow`; `PreModelSwitch` is `deny > ask > allow`. The reference does not give one universal ordering/merge rule for every event output field.

Hook commands run in Claude Code’s current directory and inherit its environment, subject to documented subprocess scrubbing. Path placeholders are also exported as environment variables:

| Variable | Availability / meaning |
| --- | --- |
| `CLAUDE_PROJECT_DIR` | Project root where the session started; remains the main checkout path when Claude enters a worktree. |
| `CLAUDE_PLUGIN_ROOT` | Plugin installation directory for plugin hooks. |
| `CLAUDE_PLUGIN_DATA` | Plugin persistent data directory. |
| `CLAUDE_ENV_FILE` | SessionStart (and Setup) command hooks can write shell `export` lines here to persist variables into later Bash commands in the session. |
| `CLAUDE_CODE_REMOTE` | Set to `"true"` in remote web environments; not set in local CLI. |
| `CLAUDE_CODE_BRIDGE_SESSION_ID` | Remote Control session ID while a local session has an active Remote Control connection; requires v2.1.199+. |
| `CLAUDE_EFFORT` | Effort level for applicable tool-context hooks, when available. |

In addition to the named variables, hook input may contain `effort` and other context fields; these are JSON fields rather than environment variables. See [reference scripts by path](https://code.claude.com/docs/en/hooks#reference-scripts-by-path), [common input fields](https://code.claude.com/docs/en/hooks#common-input-fields), and [persist environment variables](https://code.claude.com/docs/en/hooks#persist-environment-variables).

### Common input envelope

The CLI hook payload is a JSON object on stdin; HTTP hooks receive it as the POST body. Common exact wire names are snake_case. The reference’s common-input table does not formally state universal requiredness; the official Python SDK `TypedDict` marks `session_id`, `transcript_path`, and `cwd` required, and marks `permission_mode`, `prompt_id`, and `effort` optional. CLI event docs warn that some common fields, particularly `permission_mode`, are absent on some events.

| Field | Type | Requiredness / meaning |
| --- | --- | --- |
| `session_id` | string | Required in SDK base input; current session identifier. |
| `prompt_id` | string | Optional; UUID for the user prompt being processed; absent until first user input. |
| `transcript_path` | string | Required in SDK base input; path to conversation JSON. The file is asynchronous and may lag the in-memory conversation. |
| `cwd` | string | Required in SDK base input; working directory when invoked. |
| `scratchpad_dir` | string | Optional/conditional; session scratchpad path; absent if unavailable. v2.1.257+. |
| `permission_mode` | string enum | Optional/conditional; `default`, `plan`, `acceptEdits`, `auto`, `dontAsk`, or `bypassPermissions`; not all events receive it. |
| `effort` | object | Optional/conditional; has `level` (`low`, `medium`, `high`, `xhigh`, `max`) when available for tool-context events. |
| `hook_event_name` | string literal | Event discriminator, such as `PreToolUse`; the event examples and SDK types use it on each payload. |
| `agent_id` | string | Optional/conditional; identifies subagent or in-process teammate context. |
| `agent_type` | string | Optional/conditional; agent name, including subagent or `--agent` name. |

Event-specific field table below lists the additional fields. Where a field’s exact type or formal requiredness is not stated by the reference, this report says so rather than inventing a schema. The complete event names and firing points are listed in the [Hooks reference](https://code.claude.com/docs/en/hooks#hook-lifecycle); the same event set is described for SDK callbacks with an explicit SDK support subset in [Agent SDK hooks](https://code.claude.com/docs/en/agent-sdk/hooks).
### Claude Code JSON output fields

These exact names are the command/HTTP hook JSON wire fields. The common JSON output documentation does not define one universal event-specific schema: fields are accepted, ignored, or interpreted differently according to event.

| Field | Type | Requiredness | Meaning |
| --- | --- | --- | --- |
| `continue` | boolean | Optional; default `true` | `false` stops processing entirely where the event honors it; takes precedence over event-specific decisions. |
| `stopReason` | string | Optional | User-facing reason when `continue:false`; remains in conversation. |
| `suppressOutput` | boolean | Optional; default `false` | Accepted but currently has no effect. |
| `systemMessage` | string | Optional | Warning shown to user; event may discard it. |
| `terminalSequence` | string | Optional | Emits allowlisted terminal sequences (OSC 0/1/2/9/99/777 or BEL); not a general terminal-output channel. |
| `decision` | string enum | Optional | For listed events, only `"block"` is valid; omit to proceed. `PreToolUse`'s older `approve`/`block` format is deprecated. |
| `reason` | string | Optional generally; required for some `decision:"block"` results such as Stop | Explanation for a blocked or continued action; display/delivery depends on event. |
| `hookSpecificOutput` | object | Optional | Event-specific result object. When present, `hookEventName` is required and must be the exact event name. |
| `hookSpecificOutput.hookEventName` | string literal | Required inside `hookSpecificOutput` | Binds the output to the hook event. |
| `hookSpecificOutput.permissionDecision` | string enum | Optional | For `PreToolUse`/`PreModelSwitch`: `allow`, `deny`, `ask`; `PreToolUse` also accepts `defer`. |
| `hookSpecificOutput.permissionDecisionReason` | string | Optional | Reason shown to user/Claude according to decision. |
| `hookSpecificOutput.updatedInput` | object | Optional | Full replacement tool-input object before execution. |
| `hookSpecificOutput.additionalContext` | string | Optional | Context injected to Claude or a subagent for the relevant event. |
| `hookSpecificOutput.decision` | object | Required when a `PermissionRequest` hook decides | Contains `behavior` (`allow`/`deny`) and optional event-specific values such as `updatedInput`, `updatedPermissions`, `message`, `interrupt`. |
| `hookSpecificOutput.updatedToolOutput` | any/tool-shaped value | Optional | Replaces model-visible tool output for `PostToolUse`. |
| `hookSpecificOutput.updatedMCPToolOutput` | any | Optional/deprecated | Replaces MCP output only; prefer `updatedToolOutput`. |
| `hookSpecificOutput.retry` | boolean | Optional | `true` lets the model retry a denied action for `PermissionDenied`. |
| `hookSpecificOutput.displayContent` | string | Optional | `MessageDisplay` screen-only replacement. |
| `hookSpecificOutput.watchPaths` | array of absolute path strings | Optional | Replaces dynamic paths watched by `FileChanged`. |
| `hookSpecificOutput.worktreePath` | string | Required for HTTP `WorktreeCreate` success | Path to the created worktree. Command hooks instead print the path as the last non-empty stdout line. |
| `hookSpecificOutput.action` | string enum | Required to answer/alter an elicitation | `accept`, `decline`, or `cancel`. |
| `hookSpecificOutput.content` | object | Optional, required with `accept` for form values | Elicitation form values or replacement result content. |

For prompt/agent handlers, the LLM decision response is a separate shape: `ok` (boolean, required), `reason` (string, required when `ok:false`), and `impossible` (boolean, optional for prompt hooks; ignored by agent hooks). See [JSON output](https://code.claude.com/docs/en/hooks#json-output), [Decision control](https://code.claude.com/docs/en/hooks#decision-control), and [prompt response schema](https://code.claude.com/docs/en/hooks#response-schema).

### Claude Code event catalog: input and output

All events inherit the applicable common input fields above. `hookSpecificOutput` is an event-specific object that carries `hookEventName`; the JSON output reference documents universal `continue`, `stopReason`, `suppressOutput`, `systemMessage`, and `terminalSequence` fields. Unless marked otherwise, decision fields are sparse: omit them to let the usual flow proceed.

| Event | Additional input fields (name, type, requiredness, meaning) | Output / decision control |
| --- | --- | --- |
| `SessionStart` | `source` (string enum; present: `startup`, `resume`, `clear`, `compact`, `fork`); `model` (string, optional); `agent_type` (string, optional); `session_title` (string, optional). On `resume`/`fork` with a prior assistant response, v2.1.251+ may include `seconds_since_last_response` (number), `context_tokens` (number), `prompt_cache_likely_expired` (boolean), `estimated_cache_write_usd` (number). | `hookSpecificOutput.additionalContext`, `initialUserMessage`, `sessionTitle`, `watchPaths`, `reloadSkills`. Plain-text stdout is also context. After compaction, `continue:false` ends the turn before another model request. |
| `Setup` | `trigger` (string enum: `init` or `maintenance`). | No decision control. Only command and MCP-tool hooks run. Output fields are discarded; variables may be persisted via `CLAUDE_ENV_FILE`. |
| `InstructionsLoaded` | `file_path` (absolute string), `memory_type` (string enum: `User`, `Project`, `Local`, `Managed`), `load_reason` (string enum: `session_start`, `nested_traversal`, `path_glob_match`, `include`, `compact`); `globs`, `trigger_file_path`, `parent_file_path` are conditional fields. | No decision control; JSON output such as `systemMessage`/`continue` is discarded. |
| `UserPromptSubmit` | `prompt` (string); `session_title` (string, when session has a custom title). | Top-level `decision:"block"` + `reason`; `hookSpecificOutput.additionalContext`, `sessionTitle`, `suppressOriginalPrompt`; plain-text stdout also injects context. Cannot rewrite the prompt. |
| `UserPromptExpansion` | `expansion_type` (string enum: `slash_command`, `mcp_prompt`), `command_name` (string), `command_args` (string), `command_source` (string; docs do not say complete value set), `prompt` (string). | Top-level `decision:"block"` + `reason`; `hookSpecificOutput.additionalContext`. Blocks expansion, not the user’s original text. |
| `MessageDisplay` | `turn_id` (UUID string), `message_id` (UUID string), `index` (zero-based number), `final` (boolean), `delta` (string). | `hookSpecificOutput.displayContent` changes what is displayed only; no block, and transcript/model content stays original. |
| `PreToolUse` | `tool_name` (string), `tool_input` (tool-specific object), `tool_use_id` (string); MCP tools also get `mcp_server` (object with `name` and `source`). The full nested `tool_input` type depends on the tool. | `hookSpecificOutput.permissionDecision` is `allow`, `deny`, `ask`, or `defer`; `permissionDecisionReason`, `updatedInput` (replacement object), and `additionalContext`. Multiple-hook precedence: `deny > defer > ask > allow`. |
| `PermissionRequest` | `tool_name` (string), `tool_input` (tool-specific object), optional `permission_suggestions` (array of permission-update objects); MCP tools also get `mcp_server`. No `tool_use_id` is documented for this event. | `hookSpecificOutput.decision.behavior` is `allow` or `deny`; optional `updatedInput` / `updatedPermissions` for allow, `message` / `interrupt` for deny. Exit code 2 does not decide this event; use the JSON decision. |
| `PostToolUse` | `tool_name` (string), `tool_input` (tool-specific object), `tool_response` (tool-specific output object/string), `tool_use_id` (string), `duration_ms` (number, optional); MCP tools also get `mcp_server`. | Top-level `decision:"block"` + `reason`; `hookSpecificOutput.additionalContext`, `classifierContext`, `updatedToolOutput`, or `updatedMCPToolOutput`. Replacement affects what Claude sees only; the tool already ran. |
| `PostToolUseFailure` | `tool_name` (string), `tool_input` (object), `tool_use_id` (string), `error` (string), `is_interrupt` (boolean, optional), `duration_ms` (number, optional); MCP tools also get `mcp_server`. | `hookSpecificOutput.additionalContext`; no undo or pre-execution gate. |
| `PostToolBatch` | `tool_calls` (array); each item has `tool_name`, `tool_input`, `tool_use_id`, and `tool_response`. | `hookSpecificOutput.additionalContext`; top-level `decision:"block"` + `reason` or `continue:false` stops the loop before the next model call. |
| `PermissionDenied` | `tool_name` (string), `tool_input` (object), `tool_use_id` (string), `reason` (string); MCP tools also get `mcp_server`. | `hookSpecificOutput.retry` (boolean). `true` tells the model it may retry; it does not reverse the denial. Ignored for no-verdict auto-mode denials. |
| `Notification` | `message` (string), `notification_type` (string enum; documented matcher values include `permission_prompt`, `idle_prompt`, `auth_success`, several elicitation/agent/quota values), `title` (string, optional). | No blocking/modification. `terminalSequence` can emit an allowlisted terminal notification; `systemMessage` and `continue` are discarded. |
| `SubagentStart` | `agent_id` (string), `agent_type` (string). | `hookSpecificOutput.additionalContext` for subagent. Cannot block creation. |
| `SubagentStop` | `stop_hook_active` (boolean), `agent_id` (string), `agent_type` (string), `agent_transcript_path` (string), `last_assistant_message` (string); `background_tasks` and `session_crons` arrays when the task registry is reachable. | Same continuation controls as `Stop`: `decision:"block"` + required `reason`, or `hookSpecificOutput.additionalContext`; `continue:false` stops. |
| `TaskCreated` | `task_id` (string), `task_subject` (string), optional `task_description` (string), `teammate_name` (string), and deprecated `team_name` (string). | `decision:"block"` + `reason` or exit code 2 removes/rolls back the newly created task. `continue:false` is ignored. |
| `TaskCompleted` | `task_id` (string), `task_subject` (string), optional `task_description`, `teammate_name`, deprecated `team_name`; `agent_id` may identify the teammate/subagent. | `decision:"block"` + `reason` or exit code 2 prevents completion. `continue:false` stops a teammate only when teammate completion triggered the event; ignored for `TaskUpdate` completion. |
| `Stop` | `stop_hook_active` (boolean), `last_assistant_message` (string, optional), `background_tasks` (array) and `session_crons` (array) when registry reachable. | `decision:"block"` + required `reason` keeps Claude working; `hookSpecificOutput.additionalContext` is non-error feedback that also continues; `continue:false` stops processing. An 8-consecutive-continuation cap applies. |
| `StopFailure` | `error` (string enum), optional `error_details` (string), optional `last_assistant_message` (string). | No decision control. JSON and exit code are ignored, except `terminalSequence`. |
| `TeammateIdle` | `teammate_name` (string), `team_name` (string, deprecated), `agent_id` (optional string; v2.1.290+). | Exit code 2 feeds stderr back and keeps teammate working; `continue:false` + `stopReason` stops the teammate. |
| `ConfigChange` | `source` (string enum: `user_settings`, `project_settings`, `local_settings`, `policy_settings`, `skills`), `file_path` (string, optional). | `decision:"block"` + `reason` or exit code 2 blocks applying the change, except `policy_settings` cannot be blocked. `systemMessage`/`continue` are discarded. |
| `CwdChanged` | `old_cwd` (string), `new_cwd` (string). | No decision control. `hookSpecificOutput.watchPaths` replaces the dynamic `FileChanged` watch list. |
| `DirectoryAdded` | `directory` (absolute path string), `source` (string enum: `slash_command`, `register_repo_root`). | No decision control; the add has happened. `systemMessage` handling differs by source. |
| `FileChanged` | `file_path` (absolute path string), `event` (string enum: `change`, `add`, `unlink`). | No decision control. `hookSpecificOutput.watchPaths` replaces the dynamic watch list; matcher config paths remain watched. |
| `WorktreeCreate` | `name` (slug string). | Command hook prints path as last non-empty stdout line; HTTP returns `hookSpecificOutput.worktreePath`. No normal decision shape; failure/missing path fails creation. |
| `WorktreeRemove` | `worktree_path` (absolute path string). | Exit status decides removal; nonzero fails when the directory still exists. JSON output is not the contract. |
| `PreCompact` | `trigger` (string enum: `manual`, `auto`), `custom_instructions` (string or `null`). | `continue:false` or `decision:"block"` + `reason` can block compaction. |
| `PostCompact` | `trigger` (string enum: `manual`, `auto`), `compact_summary` (string). | No decision control over compaction that already happened. |
| `PreModelSwitch` | `from_model` (string), `to_model` (string), `requested_model` (string or `null`), `source` (string), `context_tokens` (number), `prompt_cache_warm` (boolean), `cache_ttl` (string), `estimated_cache_write_usd` (number), `pricing` (string). | `hookSpecificOutput.permissionDecision` allow/deny/ask or top-level `decision:"block"`; no `defer`, `updatedInput`, or `additionalContext`. Precedence deny > ask > allow. |
| `PostModelSwitch` | Same model-switch fields as PreModelSwitch; `source` additionally includes `auto` and `resume`. | Plain-text stdout or `hookSpecificOutput.additionalContext` goes to Claude’s next request. No ability to undo the switch. |
| `SessionEnd` | `reason` (string enum; matcher values: `clear`, `resume`, `logout`, `prompt_input_exit`, `other`). | No decision control. Cleanup only; 1.5-second default budget, raised by per-hook `timeout` up to 60 seconds or `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS`. |
| `Elicitation` | `mcp_server_name` (string), `message` (string), optional `mode` (string; `form` or `url`), `url` (string), `elicitation_id` (string), `requested_schema` (object). | `hookSpecificOutput.action` = `accept`, `decline`, or `cancel`; `content` object for accepted form fields. Exit 2 declines. |
| `ElicitationResult` | `mcp_server_name` (string), `action` (string), optional `mode`, `elicitation_id`, `content` (object). | Can pass through, replace all response `content` with `action:"accept"`, decline, or cancel; top-level `decision:"block"` also declines. |

**Schema caveat:** The CLI reference gives event-specific tables and examples, but it does not publish a single closed JSON Schema with required arrays for all 33 events. The table uses explicit “optional/absent/conditional” wording when available; where the reference gives a field in an example but not formal requiredness, the formal requiredness is not stated. The official Python SDK `HookEvent` union lists ten callback events, while the official SDK docs mark additional events TypeScript-only; neither SDK subset is the CLI event list.

### Exit-code and HTTP behavior

General command-hook behavior: exit 0 is success and JSON fields apply; exit 2 is a blocking error only for the events below; any other exit or process/parse failure is normally non-blocking, unless `onFailure:"block"` is set on a command/HTTP hook (v2.1.295+). JSON output can still decide behavior on a nonzero code other than 2 when valid; exit 2’s block cannot be overridden by JSON. HTTP hooks use status/body rather than exit code; successful 2xx empty body equals exit 0/no output, 2xx JSON uses the same schema, non-2xx/connection failure is non-blocking unless `onFailure:"block"`. See [exit-code output](https://code.claude.com/docs/en/hooks#exit-code-output), [per-event exit code 2 behavior](https://code.claude.com/docs/en/hooks#exit-code-2-behavior-per-event), and [HTTP response handling](https://code.claude.com/docs/en/hooks#http-response-handling).

| Event(s) | Exit code 2 behavior |
| --- | --- |
| `PreToolUse` | Blocks the tool call. |
| `PermissionRequest` | Not honored; normal permission flow continues. Return JSON `decision` instead. |
| `UserPromptSubmit`, `UserPromptExpansion` | Rejects prompt/blocks expansion. |
| `Stop`, `SubagentStop` | Keeps agent/subagent working; stderr is continuation feedback. |
| `TeammateIdle` | Prevents idle; teammate continues with stderr feedback. |
| `TaskCreated` | Rolls back task creation. |
| `TaskCompleted` | Prevents task completion. |
| `ConfigChange` | Blocks the change except for `policy_settings`. |
| `PostToolUse`, `PostToolUseFailure` | Does not undo the tool; feeds stderr to Claude as feedback. |
| `PostToolBatch` | Stops the agentic loop before the next model call. |
| `PermissionDenied` | Ignored; denial already happened. Use JSON `retry` (ignored for no-verdict denials). |
| `Notification`, `Setup`, `SessionStart`, `SessionEnd`, `CwdChanged`, `FileChanged`, `PostCompact`, `StopFailure`, `InstructionsLoaded` | No block; details vary (some show stderr, some discard output). |
| `SubagentStart` | Cannot block creation; stderr is user-facing only. |
| `DirectoryAdded` | Cannot block because the directory is already added; stderr goes to debug log. |
| `PreCompact` | Blocks compaction. |
| `PreModelSwitch` | Blocks the switch. |
| `PostModelSwitch` | Cannot undo switch; stderr is user-facing only. |
| `Elicitation`, `ElicitationResult` | Declines or blocks the response. |
| `WorktreeCreate` | Any nonzero exit fails worktree creation. |
| `WorktreeRemove` | Nonzero fails removal if the directory still exists. |
| `MessageDisplay` | Original text displays. |

For failures not covered by exit code 2, `onFailure:"block"` applies only to command/HTTP hooks; it does not affect Stop/SubagentStop/TaskCompleted/TeammateIdle or background command hooks. Windows without Git Bash has a documented shell-form PreToolUse/PermissionRequest exception that can block on failure even when `onFailure` is unset.

### Result aggregation

All matching hooks run in parallel. The reference explicitly documents `PreToolUse` precedence (`deny > defer > ask > allow`) and `PreModelSwitch` precedence (`deny > ask > allow`). It does not specify a general conflict rule for every output field or a deterministic execution order for same-event hooks. Context channels can accumulate; the docs do not define a universal merge policy for all fields. See [hook handler fields](https://code.claude.com/docs/en/hooks#hook-handler-fields), [PreToolUse decision control](https://code.claude.com/docs/en/hooks#pretooluse-decision-control), and [PreModelSwitch decision control](https://code.claude.com/docs/en/hooks#premodelswitch-decision-control).

## B. OpenAI Codex hooks

Primary sources: [Codex Hooks](https://developers.openai.com/codex/hooks), [Advanced Configuration](https://developers.openai.com/codex/config-advanced), [Configuration Reference](https://developers.openai.com/codex/config-reference), [Managed Configuration](https://developers.openai.com/codex/enterprise/managed-configuration), and official source/schema files in [`openai/codex`](https://github.com/openai/codex/tree/main/codex-rs/hooks). The docs say hooks are enabled by default; OpenAI’s [May 14, 2026 announcement](https://openai.com/index/work-with-codex-from-anywhere/) explicitly calls them generally available.

### Configuration sources, trust, and schema

Codex hooks exist and are not merely the `notify` command. `notify` is a separate command invoked for notifications; the lifecycle hook system has event/matcher/handler config and control output.

| Location | Scope / rule |
| --- | --- |
| `~/.codex/hooks.json` | User hooks. |
| `~/.codex/config.toml` | User hooks inline under `[hooks]`. |
| `<repo>/.codex/hooks.json` | Project hooks; project `.codex/` layer must be trusted. |
| `<repo>/.codex/config.toml` | Project hooks inline under `[hooks]`; ignored when project is untrusted. |
| Active system/managed configuration | Managed hooks can be set inline in `requirements.toml` delivered via system, MDM, cloud, or managed configuration. `managed_dir` / `windows_managed_dir` locate external scripts; Codex does not distribute them. |
| Enabled plugin | Default `hooks/hooks.json` under the plugin root, or a `hooks` entry in `.codex-plugin/plugin.json` (path, path array, inline object, or inline object array). Manifest path replaces default file when provided. Plugin hooks require trust review. |

All matching hooks from all sources load; a higher-precedence config layer does not replace lower-layer hooks. If one layer contains both `hooks.json` and inline `[hooks]`, Codex merges them and warns. Project-local hooks are trust-gated; managed hooks are trusted by policy and cannot be disabled in `/hooks`. `allow_managed_hooks_only = true` excludes user, project, session, and plugin hooks while preserving managed ones. Feature key is `[features].hooks` (default enabled); `codex_hooks` is a deprecated alias. See [locations](https://developers.openai.com/codex/hooks#where-codex-looks-for-hooks), [trust review](https://developers.openai.com/codex/hooks#review-and-trust-hooks), and [managed hooks](https://developers.openai.com/codex/hooks#managed-hooks-from-requirementstoml).

| Config level | Field | Type | Required? | Meaning |
| --- | --- | --- | --- | --- |
| `hooks.json` top level | `description` | string | Optional | Metadata only; does not change execution. |
| `hooks.json` top level | `hooks` | object | Required for the documented JSON shape | Maps event names to arrays of matcher groups. |
| Inline TOML / managed `[hooks]` | event names such as `PreToolUse` | array of matcher groups | Optional per event | Same event structure as `hooks.json`. Managed `[hooks]` additionally supports `managed_dir` and `windows_managed_dir`. |
| Matcher group | `matcher` | string | Optional | Regex/exact matcher; omitted, `""`, or `"*"` matches all supported event occurrences. |
| Matcher group | `hooks` | array of handlers | Required by documented shape; parser defaults missing list to empty | Handlers to run when the group matches. |
| Any handler | `type` | string enum | Required | Active types: `command`, `mcp_tool`. `prompt` and `agent` parse but are skipped. |
| `command` | `command` | string | Required | Shell command to run. |
| `command` | `commandWindows` | string | Optional | Windows-only override; TOML accepts `command_windows` or `commandWindows`. |
| `command` | `timeout` | number, seconds | Optional | Default 600 seconds for most hooks; `SessionEnd` and `Interrupt` default to 1 second and have a 3-second maximum. |
| `command` | `async` | boolean | Optional | `true` runs command in background; background hooks cannot block, approve, or rewrite. |
| `command` | `statusMessage` | string | Optional | UI status while hook runs. |
| `command` | `additionalContextLimit` | integer | Optional | Approximate token threshold for spilling `additionalContext`; default about 2,500, positive integer customizes, `0` disables spill for that handler. |
| `mcp_tool` | `server` | string | Required | Name of an already-connected MCP server. |
| `mcp_tool` | `tool` | string | Required | Tool name on that server. |
| `mcp_tool` | `input` | object | Optional | JSON argument templates; `${field.nested}` expands from hook input. |
| `mcp_tool` | `timeout` | number, seconds | Optional | Default 600 seconds; shorter of hook/server timeout applies. |
| `mcp_tool` | `statusMessage` | string | Optional | UI status while hook runs. |
| `prompt` / `agent` | additional fields | docs do not say | docs do not say | These handler types are parsed but not executed. The current Rust config enum has empty `Prompt {}` and `Agent {}` variants. |

Codex runs commands with the session `cwd`. Its plugin hooks receive `PLUGIN_ROOT`, `PLUGIN_DATA`, `CLAUDE_PLUGIN_ROOT`, and `CLAUDE_PLUGIN_DATA`. The docs do not enumerate the complete inherited process environment. See [config shape](https://developers.openai.com/codex/hooks#config-shape), [MCP tool hooks](https://developers.openai.com/codex/hooks#mcp-tool-hooks), and [`HookHandlerConfig`](https://github.com/openai/codex/blob/main/codex-rs/config/src/hook_config.rs).

### Matchers and event-specific inputs

Codex matchers are strings parsed as follows by the current Rust implementation: empty and `"*"` match all; strings containing only ASCII letters/digits, `_`, and `|` are exact names or exact `|` alternatives; strings containing other characters are Rust regexes, tested unanchored unless anchored explicitly. The current docs list these matcher-capable events: `PermissionRequest`, `PostToolUse`, `PostCompact`, `PreCompact`, `PreToolUse`, `SessionEnd`, `SessionStart`, `SubagentStart`, and `SubagentStop`. `UserPromptSubmit`, `Stop`, and `Interrupt` ignore `matcher`. `apply_patch` accepts aliases `Edit` or `Write`; input still reports `tool_name: "apply_patch"`. Current tool coverage includes shell (`Bash`), `apply_patch`, MCP, and most local function tools; hosted tools such as `WebSearch` are not covered, and the docs warn some specialized paths opt out. Sources: [Codex matcher docs](https://developers.openai.com/codex/hooks#matcher-patterns) and [Rust matcher implementation](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/engine/matcher.rs).

Codex’s shared input fields and status are:

| Field | Type | Requiredness / meaning |
| --- | --- | --- |
| `session_id` | string | Required in current per-event schemas; subagent hooks use the parent session ID. |
| `transcript_path` | string or `null` | Required property in current per-event schemas; path may be null. Convenience metadata, not a stable transcript interface. |
| `cwd` | string | Required in current per-event schemas; session working directory. |
| `hook_event_name` | string literal | Required event discriminator. |
| `model` | string | Codex-specific active model slug; required in schemas that include it, but absent from `SessionEnd`. |
| `turn_id` | string | Codex-specific active turn ID; required in turn-scoped schemas, absent from session-scoped events. |
| `permission_mode` | string enum | Required when present; appears on the event subset listed below. Values are `default`, `acceptEdits`, `plan`, `dontAsk`, and `bypassPermissions`. |

The docs list `permission_mode` on `SessionStart`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop`, `Stop`, and `Interrupt`. Current source also serializes optional `agent_id` / `agent_type` attribution on some tool events from a subagent; the docs do not define these as common public fields. See [common input fields](https://developers.openai.com/codex/hooks#common-input-fields), [generated schemas](https://github.com/openai/codex/tree/main/codex-rs/hooks/schema/generated), and [PreToolUse source](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/events/pre_tool_use.rs).

| Event | Event-specific input fields (exact names/types/meaning) |
| --- | --- |
| `SessionStart` | `source` (string enum; required; `startup`, `resume`, `clear`, `compact`, `fork`; matcher input). |
| `SessionEnd` | `reason` (string; required; currently constant `other`; matcher input). |
| `SubagentStart` | `turn_id`, `agent_id`, `agent_type`, `permission_mode` (all required in current schema; `agent_id` is subagent ID; `agent_type` is profile/type and matcher input). |
| `PreToolUse` | `turn_id`, `tool_name`, `tool_use_id`, `tool_input`, `permission_mode` (required in current schema; canonical tool name, tool-call ID, tool-specific JSON value, and current permission mode). Bash and `apply_patch` use `tool_input.command`; MCP/local-function tools use their argument object. |
| `PermissionRequest` | `turn_id`, `tool_name`, `tool_input`, `permission_mode` (required); `tool_input.description` is `string|null` when Codex provides one. No `tool_use_id` is documented for this event. |
| `PostToolUse` | `turn_id`, `tool_name`, `tool_use_id`, `tool_input`, `tool_response`, `permission_mode` (required; canonical name, invocation ID, input JSON, tool output JSON, and mode). |
| `PreCompact`, `PostCompact` | `turn_id` and `trigger` (required; `trigger` is the string enum `manual` or `auto`, used by matcher). |
| `UserPromptSubmit` | `turn_id`, `prompt`, `permission_mode` (required; submitted prompt is a string). Matcher is ignored. |
| `SubagentStop` | `turn_id`, `agent_id`, `agent_type`, `agent_transcript_path`, `stop_hook_active`, `last_assistant_message`, `permission_mode` (required properties; transcript and last message are nullable strings). |
| `Stop` | `turn_id`, `stop_hook_active`, `last_assistant_message`, `permission_mode` (required properties; last message is nullable string). |
| `Interrupt` | `turn_id` and `permission_mode` (required). Main-thread active turns only; no idle-thread or subagent event. |

The docs define field types above for these event fields but do not provide a closed required-field JSON Schema in the prose tables; the generated per-event schemas are linked under [Schemas](https://developers.openai.com/codex/hooks#schemas). The current [Rust event source](https://github.com/openai/codex/tree/main/codex-rs/hooks/src/events) also serializes optional `agent_id` / `agent_type` attribution for some in-subagent tool events, but the docs’ common-input table does not list these as common fields.

### Codex outputs, decisions, and per-event semantics

Codex command hooks read one JSON object from stdin and generally return a JSON object on stdout; plain text is accepted only on events documented as context-taking. Current common JSON fields:

| Field | Type | Required? | Meaning |
| --- | --- | --- | --- |
| `continue` | boolean | Optional; default `true` | `false` stops that hook run; effect on the agent depends on event. |
| `stopReason` | string | Optional | Recorded reason for a stopped hook run. |
| `systemMessage` | string | Optional | Warning in UI or event stream. |
| `suppressOutput` | boolean | Optional; default `false` | Parsed but not implemented. |
| `decision` | string enum | Optional | `block` is the supported top-level decision on documented events. PreToolUse also parses legacy `approve`/`block`. |
| `reason` | string | Optional generally; semantically required with blocking `decision` on `Stop`/`SubagentStop` | Explanation or continuation prompt. |
| `hookSpecificOutput` | object | Optional | When present, includes required `hookEventName` and event-specific fields. |
| `hookSpecificOutput.hookEventName` | string literal | Required inside `hookSpecificOutput` | Identifies the event to which the output applies. |
| `hookSpecificOutput.permissionDecision` | enum `allow` / `deny` / `ask` | Optional | PreToolUse gate; `ask` is parsed but unsupported by current docs/runtime. |
| `hookSpecificOutput.permissionDecisionReason` | string | Optional | Gate feedback/reason. |
| `hookSpecificOutput.updatedInput` | object; tool-specific shape | Optional | Replacement arguments object, supported only with PreToolUse `permissionDecision:"allow"`. The generated schema does not constrain its shape. |
| `hookSpecificOutput.additionalContext` | string | Optional | Extra model/developer context for supported events. |
| `hookSpecificOutput.decision` | object | Optional | PermissionRequest decision object. |
| `hookSpecificOutput.decision.behavior` | enum `allow` / `deny` | Required inside decision object | Approves or denies the pending approval request. |
| `hookSpecificOutput.decision.message` | string | Optional | Denial message. |
| `hookSpecificOutput.decision.updatedInput` | Not specified by docs/schema | Reserved; presence fails closed | Not supported on PermissionRequest; the generated schema gives no JSON Schema `type`. |
| `hookSpecificOutput.decision.updatedPermissions` | Not specified by docs/schema | Reserved; presence fails closed | Not supported on PermissionRequest; the generated schema gives no JSON Schema `type`. |
| `hookSpecificOutput.decision.interrupt` | boolean | Reserved; `true` fails closed | Not supported on PermissionRequest. |
| `hookSpecificOutput.updatedMCPToolOutput` | Not specified by generated schema | Optional in schema, unsupported at runtime | Parsed but not implemented for PostToolUse; the generated schema gives no JSON Schema `type`. |

| Event | Supported output shape and effect |
| --- | --- |
| `SessionStart` | Plain stdout becomes extra developer context. JSON supports common fields and `hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: string }`. After compaction, `continue:false` ends the turn before another model request. |
| `SessionEnd` | Advisory only; output does not steer Codex or keep the thread open. |
| `SubagentStart` | Plain stdout becomes subagent developer context. JSON supports `systemMessage` and `hookSpecificOutput.additionalContext`; `continue:false` is parsed but does not prevent start. |
| `PreToolUse` | `hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision, permissionDecisionReason?, updatedInput?, additionalContext? }`; `permissionDecision` enum is `allow`, `deny`, `ask` in the schema. Current docs say `ask`, `continue`, `stopReason`, and `suppressOutput` are not supported for this event and cause a failed hook that proceeds. `allow` + `updatedInput` rewrites the full argument object; only use `updatedInput` with `allow`. Legacy `{ "decision": "block", "reason": "..." }` is also accepted. |
| `PermissionRequest` | `hookSpecificOutput: { hookEventName: "PermissionRequest", decision: { behavior: "allow"|"deny", message? } }`. `deny` beats any allow; otherwise an allow bypasses the normal approval prompt; no decision leaves normal approval flow. Do not return `updatedInput`, `updatedPermissions`, or `interrupt`: they are reserved and fail closed. |
| `PostToolUse` | `hookSpecificOutput: { hookEventName: "PostToolUse", additionalContext? }`, top-level `systemMessage`, `decision:"block"` + `reason`, and `continue:false` are documented. Blocking replaces the model-visible result with feedback but cannot undo side effects. `updatedMCPToolOutput` and `suppressOutput` are parsed but unsupported. |
| `PreCompact` | Common output fields; `continue:false` stops before compacting. |
| `PostCompact` | Common output fields; `continue:false` stops after compacting. |
| `UserPromptSubmit` | Plain stdout becomes developer context; JSON common fields and `hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext? }`. Top-level `decision:"block"` + `reason`, or exit 2 with stderr, rejects the prompt. |
| `SubagentStop` | Common fields; `decision:"block"` + `reason` asks Codex to continue the subagent. `continue:false` takes precedence over continuation decisions. |
| `Stop` | Common fields; `decision:"block"` + `reason` makes a continuation prompt for Codex. `continue:false` takes precedence over other Stop continuation decisions. |
| `Interrupt` | Informational only. JSON can contain `systemMessage`; cannot prevent the interruption or restart the turn. Plain-text stdout is invalid. |

An active supported command/MCP hook can deny an operation. A `PermissionRequest` hook is distinct from `PreToolUse`: it runs only when Codex is about to ask for approval, while `PreToolUse` runs before supported tool operations. See [common output](https://developers.openai.com/codex/hooks#common-output-fields) and event sections in [Codex Hooks](https://developers.openai.com/codex/hooks).

**Schema/runtime discrepancy:** The generated `PreToolUse` output schema includes `permissionDecision: "ask"`, but the current prose docs say `"ask"` is parsed and unsupported, so Codex reports the hook as failed and proceeds with the tool call. The generated `PermissionRequest` schema includes `updatedInput`, `updatedPermissions`, and `interrupt`; it describes the first two as reserved without declaring a JSON Schema type, and says `interrupt` is a boolean. The docs/runtime say these fields are reserved and fail closed when present (for `interrupt`, when `true`). Sources: [PreToolUse output schema](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/pre-tool-use.command.output.schema.json), [PermissionRequest output schema](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/permission-request.command.output.schema.json), and [Codex PreToolUse output docs](https://developers.openai.com/codex/hooks#pretooluse).

### Codex exit codes, multiple-hook combination, and limitations

Hooks are synchronous by default. Matching command hooks for the same event are launched concurrently; they cannot prevent another matching command handler from starting. Set `async: true` for a background command hook. Up to eight background hooks run concurrently per session; later ones queue. They cannot block, approve, rewrite, or control the triggering operation, and unfinished ones are cancelled when the session ends. `SessionEnd` remains synchronous.

| Exit / result | Codex behavior |
| --- | --- |
| Exit `0`, no output | Success; normal operation continues. |
| Exit `0`, valid JSON | Event-specific supported fields are applied; unsupported fields can mark the hook failed. |
| Exit `2` with stderr | Blocking feedback for supported control events. `PreToolUse` blocks, `PermissionRequest` denies, `UserPromptSubmit` blocks the prompt, `PostToolUse` replaces the model-visible tool result with feedback, `Stop`/`SubagentStop` continues work. |
| Other nonzero / timeout / invalid output | Hook failure; generally does not block the action. The docs explicitly warn that errors/timeouts/malformed responses in managed remote MCP hooks can fail without blocking. `SessionEnd` is advisory; `Interrupt` is informational. |
| Background (`async:true`) | Informational output is delivered at the next safe point; no blocking/approval/rewrite regardless of exit code. |
| `mcp_tool` | Tool text is parsed like command-hook stdout; tool errors/unavailable servers do not block. Uses an existing MCP connection and runs synchronously. |

Decision combinations are event-specific. `PermissionRequest`: any deny wins, otherwise the last allow wins, otherwise normal approval applies. Current `PreToolUse` source combines blockers with `any`; if not blocked, competing `updatedInput` values resolve to the hook that finished last. The `PreToolUse` rewrite completion-order rule is in [official Rust source](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/events/pre_tool_use.rs), not the prose docs. See [runtime behavior](https://developers.openai.com/codex/hooks), [background hooks](https://developers.openai.com/codex/hooks#run-hooks-in-the-background), and [PermissionRequest source](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/events/permission_request.rs).

### Codex docs/source caveats

The official Codex docs page says the `main`-branch generated schemas may contain fields not yet in a current release, so the page is the release-behavior authority. Two checked-in schema details also differ from the current docs/source: the current docs and `SessionStartSource` Rust enum include `fork`, while the checked-in `session-start.command.input.schema.json` enum lists only `startup`, `resume`, `clear`, and `compact`; and the checked-in `pre-tool-use.command.input.schema.json` is a Bash-shaped sample (`tool_name` const `"Bash"`, `tool_input.command`) while the docs describe coverage of MCP and other local tools with arbitrary JSON arguments. Do not treat either generated fixture as a universal current wire schema without version/context. Sources: [Codex schema warning](https://developers.openai.com/codex/hooks#schemas), [`session_start.rs`](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/events/session_start.rs), [SessionStart schema fixture](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/session-start.command.input.schema.json), [PreToolUse schema fixture](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/pre-tool-use.command.input.schema.json).

The current Codex docs’ global decision-control table lists top-level `decision:"block"` for `PreCompact`, but the generated `pre-compact.command.output.schema.json` and current `output_parser::parse_pre_compact` accept only common output fields. The event-specific docs prose specifically documents `continue:false` for stopping compaction. This is a docs/schema/runtime discrepancy; the release-specific behavior should be confirmed against the target Codex version before treating `decision:"block"` as supported. Sources: [Codex decision-control docs](https://developers.openai.com/codex/hooks#common-output-fields), [PreCompact output schema](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/pre-compact.command.output.schema.json), [Codex output parser](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/engine/output_parser.rs).

## C. Emerging cross-tool standards

Evidence checked on 2026-10-19:

* **ACP is not a hook schema.** ACP v1 standardizes editor/client-to-agent JSON-RPC communication, session lifecycle, updates, permissions, and capabilities; its overview does not define a portable hook configuration/input/output contract. The ACP RFD [Agent Extensions via ACP Proxies](https://agentclientprotocol.com/rfds/proxy-chains) proposes proxies that can intercept/transform ACP messages, but it is an RFD/proposal, not a native cross-vendor lifecycle hook schema.
* **Agent Hooks Protocol (AHP) is a published Working Draft, not a stable compatibility commitment.** Its current draft is version `draft` and defines a vendor-neutral harness-to-runtime-middleware control interface over stdio/HTTP with advertised events/effects. The project page explicitly calls all current artifacts Working Drafts. Sources: [AHP status](https://agenthooksprotocol.com/) and [canonical draft spec](https://agenthooksprotocol.com/docs/spec/draft/).
* **MCP Interceptors are also a draft.** SEP-2624 proposes validators/mutators for MCP context operations; the official `modelcontextprotocol/experimental-ext-interceptors` repository says the work is experimental and not an accepted or official MCP extension. It targets MCP-side context operations rather than establishing the same native lifecycle hook contract as the vendor systems here. Sources: [SEP-2624](https://github.com/modelcontextprotocol/modelcontextprotocol/issues/2624), [experimental reference repository](https://github.com/modelcontextprotocol/experimental-ext-interceptors).
* **Agent Plugins 1.0.0 is published, but it does not standardize hooks.** Its portable component types are Agent Skills and MCP servers; client-specific behavior can live under a reverse-domain extension namespace. Sources: [Agent Plugins spec v1.0.0](https://agent-plugins.org/specification) and [client extensions](https://agent-plugins.org/plugin-authors/client-extensions).

## D. Comparison matrix

Cursor is included only as a comparison baseline; the detailed catalog should use [Cursor’s Hooks reference](https://cursor.com/docs/hooks). The Claude and Codex citations above are the sources for their columns.

### Event-name mapping

| Lifecycle concept | Cursor | Claude Code | Codex |
| --- | --- | --- | --- |
| New session / thread start | `sessionStart` | `SessionStart` | `SessionStart` |
| Session end | `sessionEnd` | `SessionEnd` | `SessionEnd` |
| User prompt before model | `beforeSubmitPrompt` | `UserPromptSubmit` | `UserPromptSubmit` |
| Prompt/command expansion | — | `UserPromptExpansion` | — |
| Generic before-tool | `preToolUse` | `PreToolUse` | `PreToolUse` |
| Shell-specific before/after | `beforeShellExecution` / `afterShellExecution` | Usually `PreToolUse` / `PostToolUse` matching `Bash` or `PowerShell` | `PreToolUse` / `PostToolUse` matching `Bash` |
| MCP-specific before/after | `beforeMCPExecution` / `afterMCPExecution` | Tool events matching `mcp__<server>__<tool>`; `mcp_tool` handler also available | `PreToolUse` / `PostToolUse` matching MCP tool name; `mcp_tool` handler also available |
| Read/edit | `beforeReadFile`, `afterFileEdit` | `PreToolUse` / `PostToolUse` matching `Read`, `Edit`, `Write` | `PreToolUse` / `PostToolUse` matching `apply_patch` / aliases; specialized reads depend on actual tool path |
| Tool result / failure | `postToolUse`, `postToolUseFailure` | `PostToolUse`, `PostToolUseFailure` | `PostToolUse`; no `PostToolUseFailure` event |
| Permission request | No separate event; permission output is on pre-execution hooks | `PermissionRequest` | `PermissionRequest` |
| Batch after parallel tools | — | `PostToolBatch` | — |
| Main agent stops | `stop` | `Stop` | `Stop` |
| API/request failure | — | `StopFailure` | — |
| Subagent lifecycle | `subagentStart`, `subagentStop` | `SubagentStart`, `SubagentStop` | `SubagentStart`, `SubagentStop` |
| Compaction | `preCompact` | `PreCompact`, `PostCompact` | `PreCompact`, `PostCompact` |
| Task/teammate lifecycle | — | `TaskCreated`, `TaskCompleted`, `TeammateIdle` | — |
| Configuration/instructions/cwd/directory/files | — | `ConfigChange`, `InstructionsLoaded`, `CwdChanged`, `DirectoryAdded`, `FileChanged` | — |
| Worktree lifecycle | — | `WorktreeCreate`, `WorktreeRemove` | — |
| Model switch | — | `PreModelSwitch`, `PostModelSwitch` | — |
| MCP elicitation | — | `Elicitation`, `ElicitationResult` | — |
| Interrupt | — | No documented `Interrupt` hook event | `Interrupt` |
| Cursor-specific surfaces | `beforeTabFileRead`, `afterTabFileEdit`, `workspaceOpen`, `afterAgentResponse`, `afterAgentThought` | No exact equivalent in settings-based hooks; `MessageDisplay` is not the same as final response/thought | No exact equivalent |

`PreToolUse`/`preToolUse` are not identical to a permission prompt event: they fire before an action and can also run when the host would not show an approval dialog. Cursor’s separate `beforeShellExecution`, `beforeMCPExecution`, and `beforeReadFile` surfaces further divide that gate by operation kind. Sources: [Cursor events](https://cursor.com/docs/hooks#hook-categories), [Claude events](https://code.claude.com/docs/en/hooks#hook-lifecycle), [Codex events](https://developers.openai.com/codex/hooks).

### Config shape and locations

| System | File shape | Locations and distribution |
| --- | --- | --- |
| Cursor | `{\"version\":1,\"hooks\":{\"<event>\":[{\"command\":...}]}}`; per-event definitions are flat, not matcher-group nested. `type:"prompt"` is also documented with a `prompt` field. | `~/.cursor/hooks.json`, `<project-root>/.cursor/hooks.json`, team/cloud-distributed hooks, enterprise-managed system files. Priority: Enterprise → Team → Project → User. |
| Claude Code | `hooks` setting maps event names to arrays of `{matcher?, hooks:[{type,...}]}` groups. No settings schema version field is documented. | `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json`, managed settings, plugin `hooks/hooks.json` or manifest, skill/agent frontmatter. Settings precedence: managed > `--settings` > local project > shared project > user, with hook lists merged. |
| Codex | `hooks.json` has optional top-level `description` plus `hooks` event maps to matcher groups; same shape can be inline under `[hooks]` in `config.toml` or managed `requirements.toml`. | `~/.codex/hooks.json`, `~/.codex/config.toml`, `<repo>/.codex/hooks.json`, `<repo>/.codex/config.toml`, managed requirements, and enabled plugin `hooks/hooks.json`/manifest. All matching hooks load; project layer requires trust. |

Sources: [Cursor configuration](https://cursor.com/docs/hooks#configuration), [Claude locations/precedence](https://code.claude.com/docs/en/hooks#hook-locations) and [settings precedence](https://code.claude.com/docs/en/settings#settings-precedence), [Codex locations/config](https://developers.openai.com/codex/hooks#where-codex-looks-for-hooks).

**Cursor schema caveat:** The same [Cursor Hooks reference](https://cursor.com/docs/hooks#configuration) calls `command` required in its per-script option table, but its [prompt-hook example](https://cursor.com/docs/hooks#prompt-based-hooks) contains `type:"prompt"` and `prompt` with no `command`. The docs do not reconcile this mismatch; the prompt example establishes the documented shape, but requiredness for the combined command/prompt union is not stated consistently.

### Input envelope

| System | Shared fields / envelope |
| --- | --- |
| Cursor | Common JSON has `conversation_id`, `generation_id`, `model`, optional `model_id`, optional `model_params` (`id`, `value` pairs), `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email` (`string|null`), and `transcript_path` (`string|null`). Event-specific payloads differ substantially. `workspaceOpen` omits conversation/generation/model/session/transcript fields. |
| Claude Code | Common fields are `session_id`, `prompt_id?`, `transcript_path`, `cwd`, `scratchpad_dir?`, `permission_mode?`, `effort?`, `hook_event_name`, `agent_id?`, and `agent_type?`. Tool events add `tool_name`, `tool_input`, IDs/results/errors; lifecycle events add their own fields. |
| Codex | Common fields usually include `session_id`, `transcript_path` (`string|null`), `cwd`, `hook_event_name`, and `model`; turn-scoped events add `turn_id`. `permission_mode` is present on the documented event subset. Event-specific data includes `source`, `prompt`, `tool_name`, `tool_input`, `tool_response`, `agent_type`, `trigger`, and stop fields. |

The field spelling and types above are literal from the sources; input values are not a common portable shape. Sources: [Cursor common input](https://cursor.com/docs/hooks#input-all-hooks), [Claude common input](https://code.claude.com/docs/en/hooks#common-input-fields), [Codex common input](https://developers.openai.com/codex/hooks#common-input-fields).

### Output envelopes and decision semantics

| System | Decision / output model |
| --- | --- |
| Cursor | Event-specific JSON. Permission hooks commonly return `permission:"allow"|"deny"` and messages; pre-tool example also documents `ask` in schema but says it is not enforced today. `updated_input` rewrites tool arguments; `additional_context`, `agent_message`, `user_message`, `followup_message`, session-start `env`, and `workspaceOpen.pluginPaths` serve different events. |
| Claude Code | Universal JSON keys include `continue`, `stopReason`, `systemMessage`, `terminalSequence`, and `suppressOutput` (accepted but documented as no-op). Event-specific top-level `decision:"block"`/`reason` or `hookSpecificOutput` provide allow/deny/ask/defer, argument rewrite, result replacement, context, elicitation responses, or display-only replacement. |
| Codex | Event-specific JSON; common keys are `continue`, `stopReason`, `systemMessage`, and parsed-but-unimplemented `suppressOutput`. Tool gate uses `hookSpecificOutput.permissionDecision`; permission prompt uses `hookSpecificOutput.decision.behavior`; other events use top-level `decision:"block"`/`reason` or `continue:false`. `additionalContext` is usually inside `hookSpecificOutput`; Codex does not currently implement tool-output replacement. |

Sources: [Cursor event schemas](https://cursor.com/docs/hooks#hook-events), [Claude JSON output](https://code.claude.com/docs/en/hooks#json-output), [Codex common output](https://developers.openai.com/codex/hooks#common-output-fields).

### Exit code conventions

| System | Exit `0` | Exit `2` | Other codes/failures |
| --- | --- | --- | --- |
| Cursor | Use JSON output; permission-hook invalid JSON/schema blocks. | Blocks, equivalent to `permission:"deny"`. | Fail-open by default; `failClosed:true` changes this. |
| Claude Code | Success; valid JSON fields apply; on some context events plain stdout is injected. | Blocks only where the event supports blocking; otherwise it can be feedback or ignored, as enumerated in the Claude event table above. | Normally non-blocking; command/HTTP `onFailure:"block"` changes failures/timeouts to block on supported events. |
| Codex | Success; supported event JSON applies; some lifecycle hooks accept plain stdout as context. | Control effect depends on event; e.g. PreToolUse blocks, PermissionRequest denies, PostToolUse feeds back after the action, SessionEnd does not steer. | Generally fail-open for action, with failure reported; async output cannot control. |

Sources: [Cursor exit behavior](https://cursor.com/docs/hooks#command-based-hooks), [Claude per-event exit behavior](https://code.claude.com/docs/en/hooks#exit-code-2-behavior-per-event), [Codex event output and background-hook behavior](https://developers.openai.com/codex/hooks).

### Matchers, execution types, timeouts, and ordering

| Dimension | Cursor | Claude Code | Codex |
| --- | --- | --- | --- |
| Matcher syntax | Regex string; `""` or `"*"` matches all. Tested against event-specific values (tool type, full shell command, subagent type, fixed event labels). | Exact names/alternatives for a restricted character set, else unanchored JavaScript regex. Some events use narrower rules or no matcher. Tool `if` is a second permission-rule filter. | Regex string; empty/`*`/omitted matches all. Exact alternatives are supported. Only nine current events use matchers; event-specific targets listed above. |
| Execution types | `command` (default) and `prompt` (LLM-evaluated). | `command`, `http`, `mcp_tool`, `prompt`, `agent`; event support varies. Agent hooks experimental. | `command` and `mcp_tool` active; `prompt`/`agent` parsed but skipped. `async:true` enables background command handlers. |
| Timeout | Seconds; per-script field says platform default. The docs do not give a numeric default. | Seconds: 600 command/http/MCP; 30 prompt; 60 agent; reduced defaults on UserPromptSubmit, model-switch and MessageDisplay events; SessionEnd budget is 1.5 seconds by default, max 60 seconds. | Seconds: 600 most hooks; SessionEnd and Interrupt default 1 second, max 3 seconds. |
| Ordering / combination | All matching source hooks run. Source priority Enterprise → Team → Project → User. `deny > ask > allow`; user/agent messages concatenate; other fields use last response in priority order (lower-priority source can overwrite a higher-priority value). Docs do not define same-source tie order. | All matching hooks run in parallel; duplicate handler across settings files is deduplicated, plugin/skill copies remain separate. Explicit decision precedence exists for PreToolUse and PreModelSwitch only; no universal output-field merge rule is documented. | All matching hooks load; same-event command handlers launch concurrently. PermissionRequest any deny wins, otherwise allow; PreToolUse blocking is any-block and current source chooses conflicting rewrites by completion order. |

Sources: [Cursor configuration/matcher options](https://cursor.com/docs/hooks#configuration), [Claude handler fields/matchers](https://code.claude.com/docs/en/hooks#hook-handler-fields) and [exit/timeouts](https://code.claude.com/docs/en/hooks#exit-code-output), [Codex matchers](https://developers.openai.com/codex/hooks#matcher-patterns) and [background hooks](https://developers.openai.com/codex/hooks#run-hooks-in-the-background).

### Semantically incompatible behaviors to account for explicitly

1. **Not all “pre-tool” hooks intercept the same seam.** Claude’s `PermissionRequest` runs only when a permission decision is needed; `PreToolUse` runs before tool calls generally. Cursor has separate shell, MCP, file-read, and generic tool hooks. Codex also splits `PreToolUse` from `PermissionRequest`.
2. **The input envelopes are not wire-compatible.** Cursor uses conversation/generation IDs and model metadata; Claude uses session/prompt IDs, cwd, permission mode, and `hook_event_name`; Codex uses `session_id`, optional/null transcript, and `turn_id` extensions. Event-specific shell/MCP/file payloads also differ.
3. **“Ask” is not equivalent.** Cursor has `ask` in some pre-hook output schemas but documents it as unenforced for `preToolUse`; Claude supports `ask` (and `defer`) for `PreToolUse`; Codex parses `ask` in PreToolUse’s schema but documents it as unsupported, and `PermissionRequest` only allows/denies or defers to the normal approval prompt.
4. **Post-operation effects differ.** Claude can replace all tool output with `updatedToolOutput`; Cursor documents `updated_mcp_tool_output` for MCP only; Codex parses an MCP-output replacement field but does not implement it. None of these post hooks can undo side effects that already occurred.
5. **Completion loops differ.** Cursor uses `followup_message` with a per-script `loop_limit`; Claude uses `decision:"block"`/`reason`, `additionalContext`, or `continue:false` plus loop protections; Codex Stop uses `decision:"block"`/`reason` as a continuation prompt and separately treats `continue:false` as a stopped hook run.
6. **Failure policy differs.** Cursor is fail-open by default with `failClosed`; Claude has event-specific exit-2 handling and optional `onFailure:"block"` for command/HTTP hooks; Codex’s supported denials are event-specific and background hooks cannot control.
7. **Event coverage is non-isomorphic.** Claude exposes notification, batch, task, teammate, config/instruction/cwd/file/worktree/model-switch/elicitation events; Codex adds top-level `Interrupt`; Cursor adds Tab and workspace UI lifecycle events and has no direct counterparts for most of the others.
8. **Async and model-based execution differ.** Cursor command/prompt; Claude command/HTTP/MCP/prompt/experimental agent, with command background/rewake; Codex command/MCP active, prompt/agent skipped, async command active. A shared `type` field cannot by itself imply shared execution semantics.

These are factual gaps/behavior differences observed in the cited sources, not a proposed abstraction.
