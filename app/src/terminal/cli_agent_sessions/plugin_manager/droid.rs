use std::sync::LazyLock;

use async_trait::async_trait;

use super::{CliAgentPluginManager, PluginInstructionStep, PluginInstructions};

const MINIMUM_PLUGIN_VERSION: &str = "1.0.0";

pub(super) struct DroidPluginManager;

#[async_trait]
impl CliAgentPluginManager for DroidPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        MINIMUM_PLUGIN_VERSION
    }

    fn can_auto_install(&self) -> bool {
        false
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn supports_update(&self) -> bool {
        false
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
        title: "Install Warp Hooks for Droid",
        subtitle: "Create a Droid hook that emits Warp CLI agent events, then add it to your global Droid hooks config.",
        steps: &[
            PluginInstructionStep {
                description: "Create the hook script",
                command: r#"mkdir -p "$HOME/.factory/hooks" && cat > "$HOME/.factory/hooks/warp-notify.sh" <<'EOF'
#!/bin/sh
set -eu

PLUGIN_VERSION="1.0.0"

# Only emit Warp notifications when Droid is running inside Warp.
[ -n "${WARP_CLI_AGENT_PROTOCOL_VERSION:-}" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

input="$(cat)"
[ -n "$input" ] || exit 0

hook_event="$(printf '%s' "$input" | jq -r '.hook_event_name // .hookEventName // empty' 2>/dev/null || true)"

case "$hook_event" in
  SessionStart)
    event="session_start"
    ;;
  UserPromptSubmit)
    event="prompt_submit"
    ;;
  Stop)
    stop_hook_active="$(printf '%s' "$input" | jq -r '.stop_hook_active // .stopHookActive // false' 2>/dev/null || true)"
    [ "$stop_hook_active" = "true" ] && exit 0
    event="stop"
    ;;
  Notification)
    message="$(printf '%s' "$input" | jq -r '.message // empty' 2>/dev/null || true)"
    if printf '%s' "$message" | grep -Eiq 'permission|approval|approve'; then
      event="permission_request"
    else
      event="question_asked"
    fi
    ;;
  PostToolUse)
    event="tool_complete"
    ;;
  *)
    exit 0
    ;;
esac

warp_protocol="${WARP_CLI_AGENT_PROTOCOL_VERSION:-1}"
case "$warp_protocol" in
  ''|*[!0-9]*)
    protocol_version=1
    ;;
  *)
    if [ "$warp_protocol" -lt 1 ] || [ "$warp_protocol" -gt 1 ]; then
      protocol_version=1
    else
      protocol_version="$warp_protocol"
    fi
    ;;
esac

payload="$(printf '%s' "$input" | jq -c \
  --arg event "$event" \
  --arg plugin_version "$PLUGIN_VERSION" \
  --argjson v "$protocol_version" '
    (.cwd // env.FACTORY_PROJECT_DIR // "") as $cwd
    | {
        v: $v,
        agent: "droid",
        event: $event,
        session_id: ((.session_id // "") | tostring),
        cwd: ($cwd | tostring),
        project: (($cwd | tostring | split("/") | map(select(length > 0)) | last) // ""),
        plugin_version: $plugin_version
      }
    + (if (.transcript_path? // null) != null then {transcript_path: (.transcript_path | tostring)} else {} end)
    + (if $event == "prompt_submit" then {query: ((.prompt // "") | tostring | .[0:200])} else {} end)
    + (if $event == "permission_request" or $event == "question_asked" then {summary: ((.message // "Droid needs your attention") | tostring)} else {} end)
    + (if $event == "tool_complete" then {tool_name: ((.tool_name // "") | tostring), tool_input: (.tool_input // {})} else {} end)
  ')"

title="warp://cli-agent"
{ printf '\033]777;notify;%s;%s\007' "$title" "$payload" > /dev/tty; } 2>/dev/null || true
EOF
chmod +x "$HOME/.factory/hooks/warp-notify.sh""#,
                executable: true,
                link: None,
            },
            PluginInstructionStep {
                description: "Add the hook to ~/.factory/hooks.json. If you already have hooks configured, merge these event entries into the top-level object.",
                command: r#"{
  "SessionStart": [
    {
      "hooks": [
        {
          "type": "command",
          "command": "$HOME/.factory/hooks/warp-notify.sh",
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
          "command": "$HOME/.factory/hooks/warp-notify.sh",
          "timeout": 5
        }
      ]
    }
  ],
  "Notification": [
    {
      "hooks": [
        {
          "type": "command",
          "command": "$HOME/.factory/hooks/warp-notify.sh",
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
          "command": "$HOME/.factory/hooks/warp-notify.sh",
          "timeout": 5
        }
      ]
    }
  ],
  "PostToolUse": [
    {
      "hooks": [
        {
          "type": "command",
          "command": "$HOME/.factory/hooks/warp-notify.sh",
          "timeout": 5
        }
      ]
    }
  ]
}"#,
                executable: false,
                link: Some("https://docs.factory.ai/cli/configuration/hooks-guide"),
            },
        ],
        post_install_notes: &[
            "Restart Droid to activate the hooks.",
            "The hook requires jq. If your Droid version requires literal absolute paths in hook commands, replace $HOME with your home directory in ~/.factory/hooks.json.",
        ],
    }
});

#[cfg(test)]
#[path = "droid_tests.rs"]
mod tests;
