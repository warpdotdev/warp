# Warp Muse Code hooks sample

Research notes for a **future agent-owned plugin** (e.g. `warpdotdev/muse-code-warp`).
**Not in scope for the Warp client PR that adds `CLIAgent::Muse`.** Warp must not
write these files from `plugin_manager` (that is the Grok in-client hook writer
reviewers already stripped).

Verified against **Muse Code 1.2.1 (1.2.1-R2847.1)** on 2026-09-14 with isolated
`XDG_CONFIG_HOME` and live `muse exec` (echo + Meta provider).

## What actually fires

| Location | Fires? |
|----------|--------|
| `<workspace>/.muse/hooks.json` (HookConfig) | yes, with `--trust-workspace` / trusted workspace |
| `settings.json` `"managed_hooks_path"` → HookConfig file | yes (user-level, no extra hook trust step) |
| `settings.json` `"hooks": { "Stop": ... }` | no |
| `settings.json` `"hooks": { "hooks": { "Stop": ... } }` | no |
| `~/.config/muse/hooks.json` sibling file | no |
| `muse plugins …` | `plugins are not available in this build` |

## HookConfig file

`command` is a **string** (absolute path). Stdin is one JSON object.

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "/absolute/path/to/on-session-start.sh" }] }
    ],
    "UserPromptSubmit": [
      { "hooks": [{ "type": "command", "command": "/absolute/path/to/on-prompt.sh" }] }
    ],
    "PostToolUse": [
      { "hooks": [{ "type": "command", "command": "/absolute/path/to/on-tool.sh" }] }
    ],
    "PostToolUseFailure": [
      { "hooks": [{ "type": "command", "command": "/absolute/path/to/on-tool-failure.sh" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "/absolute/path/to/on-stop.sh" }] }
    ],
    "Notification": [
      { "hooks": [{ "type": "command", "command": "/absolute/path/to/on-notification.sh" }] }
    ],
    "PermissionRequest": [
      { "hooks": [{ "type": "command", "command": "/absolute/path/to/on-permission.sh" }] }
    ],
    "SessionEnd": [
      { "hooks": [{ "type": "command", "command": "/absolute/path/to/on-session-end.sh" }] }
    ]
  }
}
```

## settings.json pointer

Preserve unrelated keys. Set `managed_hooks_path` only when unset or already Warp’s file.

```json
{
  "schema_version": 1,
  "managed_hooks_path": "/Users/you/.config/muse/warp-plugin/hooks.json"
}
```

## Stdin payloads observed

Common fields: `hook_event_name`, `session_id`, `cwd`, `transcript_path`,
`model`, `model_provider`, `permission_mode`.

| Event | Extra fields |
|-------|----------------|
| `SessionStart` | `source` (`startup`) |
| `UserPromptSubmit` | `prompt`, `turn_id` |
| `PreToolUse` | `tool_name`, `tool_input`, `tool_use_id`, `turn_id` |
| `PostToolUse` | same as Pre + `tool_response` |
| `PostToolUseFailure` | same as Pre + `error`, `duration_ms`, `is_interrupt` |
| `Stop` | `stop_hook_active`, `last_assistant_message`, `turn_id` |
| `SessionEnd` | `reason` (e.g. `other`) |

Hook processes get a **cleared environment**. Use `/bin/sh` and absolute paths.
`python3` was not available to a hook until we stopped depending on PATH.

`DuplicateHookSource` rejects two handlers that share the same command argv —
give each event its own script (or a distinct wrapper) even if they share a
helper.
