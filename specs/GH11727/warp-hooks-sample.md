# Sample Warp notification plugin for Grok Build

Install under `$GROK_HOME/hooks` (default `~/.grok/hooks`). Grok Build loads
`*.json` hook files from that directory. This sample is the on-disk form of the
same conceptual **Warp** notification plugin that Claude Code / Codex install
from marketplaces as `warp@claude-code-warp` / `warp@codex-warp`. It emits
Warp’s structured CLI-agent protocol (`OSC 777` title `warp://cli-agent`) so
Warp can latch rich status.

> **Note:** Warp’s one-click install writes these files automatically under
> `$GROK_HOME/hooks`. This sample is the source of truth for that plugin and for
> manual installs. First-class detection works without them (OSC 9 fallback
> still provides basic turn-complete signals).

## `~/.grok/hooks/warp-plugin.json`

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bin/warp-plugin.sh",
            "timeout": 5
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bin/warp-plugin.sh",
            "timeout": 5
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bin/warp-plugin.sh",
            "timeout": 5
          }
        ]
      }
    ],
    "StopFailure": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bin/warp-plugin.sh",
            "timeout": 5
          }
        ]
      }
    ]
  }
}
```

## `~/.grok/hooks/bin/warp-plugin.sh`

```bash
#!/usr/bin/env bash
# Warp notification plugin for Grok Build. Reads hook envelope JSON from stdin;
# emits Warp OSC 777 on stderr.
set -euo pipefail

payload="$(cat)"
hook_event="$(printf '%s' "$payload" | sed -n 's/.*"hookEventName"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
session_id="$(printf '%s' "$payload" | sed -n 's/.*"sessionId"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
cwd="$(printf '%s' "$payload" | sed -n 's/.*"cwd"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
prompt="$(printf '%s' "$payload" | sed -n 's/.*"prompt"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
error="$(printf '%s' "$payload" | sed -n 's/.*"error"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"

case "$hook_event" in
  session_start|SessionStart) event="session_start" ;;
  user_prompt_submit|UserPromptSubmit) event="prompt_submit" ;;
  stop|Stop) event="stop" ;;
  stop_failure|StopFailure) event="stop_failure" ;;
  *) exit 0 ;;
esac

# Prefer python for safe JSON if available.
if command -v python3 >/dev/null 2>&1; then
  body="$(HOOK_EVENT="$event" SESSION_ID="$session_id" CWD="$cwd" QUERY="$prompt" ERROR_TYPE="$error" python3 - <<'PY'
import json, os
body = {
  "v": 1,
  "agent": "grok",
  "event": os.environ["HOOK_EVENT"],
  "plugin_version": "1.0.0",
}
if os.environ.get("SESSION_ID"):
  body["session_id"] = os.environ["SESSION_ID"]
if os.environ.get("CWD"):
  body["cwd"] = os.environ["CWD"]
if os.environ.get("QUERY"):
  body["query"] = os.environ["QUERY"]
if os.environ.get("ERROR_TYPE"):
  body["error_type"] = os.environ["ERROR_TYPE"]
print(json.dumps(body, separators=(",", ":")))
PY
)"
else
  body="{\"v\":1,\"agent\":\"grok\",\"event\":\"$event\",\"plugin_version\":\"1.0.0\"}"
fi

# OSC 777;notify;<title>;<body> BEL  — Warp maps title/body to PluggableNotification.
printf '\033]777;notify;warp://cli-agent;%s\007' "$body" >&2
```

Make executable: `chmod +x ~/.grok/hooks/bin/warp-plugin.sh`.

Also written by auto-install: `~/.grok/hooks/warp-plugin.version` (semver string).
