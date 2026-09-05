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
            "command": "bin/warp-plugin.sh SessionStart",
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
            "command": "bin/warp-plugin.sh UserPromptSubmit",
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
            "command": "bin/warp-plugin.sh Stop",
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
            "command": "bin/warp-plugin.sh StopFailure",
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

payload="$(</dev/stdin)"
hook_event="${1:-}"

case "$hook_event" in
  session_start|SessionStart) event="session_start" ;;
  user_prompt_submit|UserPromptSubmit) event="prompt_submit" ;;
  stop|Stop) event="stop" ;;
  stop_failure|StopFailure) event="stop_failure" ;;
  *) exit 0 ;;
esac

if command -v python3 >/dev/null 2>&1; then
  body="$(printf '%s' "$payload" | HOOK_EVENT="$event" python3 -c '
import json
import os
import sys

payload = json.load(sys.stdin)
body = {
    "v": 1,
    "agent": "grok",
    "event": os.environ["HOOK_EVENT"],
    "plugin_version": "1.0.0",
}
for source, target in (
    ("sessionId", "session_id"),
    ("cwd", "cwd"),
    ("prompt", "query"),
    ("error", "error_type"),
):
    value = payload.get(source)
    if isinstance(value, str) and value:
        body[target] = value
print(json.dumps(body, separators=(",", ":")))
')"
else
  body="{\"v\":1,\"agent\":\"grok\",\"event\":\"$event\",\"plugin_version\":\"1.0.0\"}"
fi

# OSC 777;notify;<title>;<body> BEL  — Warp maps title/body to PluggableNotification.
printf '\033]777;notify;warp://cli-agent;%s\007' "$body" >&2
```

Make executable: `chmod +x ~/.grok/hooks/bin/warp-plugin.sh`.

Also written by auto-install: `~/.grok/hooks/warp-plugin.version` (semver string).
