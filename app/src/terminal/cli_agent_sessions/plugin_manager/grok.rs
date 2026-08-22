use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{env, fs, io};

use async_trait::async_trait;

use super::{
    CliAgentPluginManager, PluginInstallError, PluginInstructionStep, PluginInstructions,
    compare_versions,
};

/// Minimum Warp plugin version written to `warp-plugin.version` and reported
/// via rich OSC 777 `plugin_version`. Bump when the hooks plugin schema changes.
const MINIMUM_PLUGIN_VERSION: &str = "1.0.0";

/// On-disk names for the Warp notification plugin under `$GROK_HOME/hooks`.
/// Claude/Codex ship a marketplace package named `warp` (`warp@claude-code-warp`,
/// `warp@codex-warp`); Grok has no hosted package yet, so we materialize the same
/// conceptual plugin as hooks files with a `warp-plugin` prefix.
const HOOK_JSON_FILE: &str = "warp-plugin.json";
const PLUGIN_SCRIPT_REL: &str = "bin/warp-plugin.sh";
const VERSION_FILE: &str = "warp-plugin.version";

/// Hook config loaded by Grok Build from `$GROK_HOME/hooks/*.json`.
const HOOK_JSON: &str = r#"{
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
"#;

/// Plugin script: reads Grok hook envelope JSON from stdin; emits Warp OSC 777 on stderr.
/// `plugin_version` is substituted from [`MINIMUM_PLUGIN_VERSION`] at install time.
fn plugin_script() -> String {
    format!(
        r#"#!/usr/bin/env bash
# Warp notification plugin for Grok Build. Written by Warp's one-click install.
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

if command -v python3 >/dev/null 2>&1; then
  body="$(HOOK_EVENT="$event" SESSION_ID="$session_id" CWD="$cwd" QUERY="$prompt" ERROR_TYPE="$error" python3 - <<'PY'
import json, os
body = {{
  "v": 1,
  "agent": "grok",
  "event": os.environ["HOOK_EVENT"],
  "plugin_version": "{version}",
}}
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
  body="{{\"v\":1,\"agent\":\"grok\",\"event\":\"$event\",\"plugin_version\":\"{version}\"}}"
fi

# OSC 777;notify;<title>;<body> BEL — Warp maps title/body to PluggableNotification.
printf '\033]777;notify;warp://cli-agent;%s\007' "$body" >&2
"#,
        version = MINIMUM_PLUGIN_VERSION
    )
}

pub(super) struct GrokPluginManager;

#[async_trait]
impl CliAgentPluginManager for GrokPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        MINIMUM_PLUGIN_VERSION
    }

    fn can_auto_install(&self) -> bool {
        true
    }

    fn is_installed(&self) -> bool {
        let Ok(hooks_dir) = grok_hooks_dir() else {
            return false;
        };
        check_installed(&hooks_dir)
    }

    fn needs_update(&self) -> bool {
        let Ok(hooks_dir) = grok_hooks_dir() else {
            return false;
        };
        match installed_version(&hooks_dir) {
            Some(v) => compare_versions(&v, MINIMUM_PLUGIN_VERSION).is_lt(),
            // Present but missing version file: treat as outdated so reinstall refreshes.
            None => check_installed(&hooks_dir),
        }
    }

    async fn install(&self) -> Result<(), PluginInstallError> {
        install_plugin_files()
    }

    async fn update(&self) -> Result<(), PluginInstallError> {
        install_plugin_files()
    }

    fn install_success_message(&self) -> &'static str {
        "Warp plugin installed. Please restart Grok Build to activate."
    }

    fn update_success_message(&self) -> &'static str {
        "Warp plugin updated. Please restart Grok Build to activate."
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &UPDATE_INSTRUCTIONS
    }
}

fn install_plugin_files() -> Result<(), PluginInstallError> {
    let mut log = String::new();
    let hooks_dir = grok_hooks_dir().map_err(|err| {
        log.push_str(&format!("error resolving Grok hooks dir: {err}\n"));
        PluginInstallError {
            message: "could not determine Grok home directory".to_owned(),
            log: log.clone(),
        }
    })?;

    log.push_str(&format!(
        "Installing Warp plugin into {}\n",
        hooks_dir.display()
    ));

    let bin_dir = hooks_dir.join("bin");
    fs::create_dir_all(&bin_dir).map_err(|err| {
        log.push_str(&format!("mkdir {}: {err}\n", bin_dir.display()));
        PluginInstallError {
            message: format!("failed to create {}", bin_dir.display()),
            log: log.clone(),
        }
    })?;
    log.push_str(&format!("created {}\n", bin_dir.display()));

    let json_path = hooks_dir.join(HOOK_JSON_FILE);
    fs::write(&json_path, HOOK_JSON).map_err(|err| {
        log.push_str(&format!("write {}: {err}\n", json_path.display()));
        PluginInstallError {
            message: format!("failed to write {}", json_path.display()),
            log: log.clone(),
        }
    })?;
    log.push_str(&format!("wrote {}\n", json_path.display()));

    let script_path = hooks_dir.join(PLUGIN_SCRIPT_REL);
    fs::write(&script_path, plugin_script()).map_err(|err| {
        log.push_str(&format!("write {}: {err}\n", script_path.display()));
        PluginInstallError {
            message: format!("failed to write {}", script_path.display()),
            log: log.clone(),
        }
    })?;
    log.push_str(&format!("wrote {}\n", script_path.display()));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)
            .map_err(|err| {
                log.push_str(&format!("stat {}: {err}\n", script_path.display()));
                PluginInstallError {
                    message: format!("failed to set permissions on {}", script_path.display()),
                    log: log.clone(),
                }
            })?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).map_err(|err| {
            log.push_str(&format!("chmod {}: {err}\n", script_path.display()));
            PluginInstallError {
                message: format!("failed to make {} executable", script_path.display()),
                log: log.clone(),
            }
        })?;
        log.push_str(&format!("chmod 755 {}\n", script_path.display()));
    }

    let version_path = hooks_dir.join(VERSION_FILE);
    fs::write(&version_path, format!("{MINIMUM_PLUGIN_VERSION}\n")).map_err(|err| {
        log.push_str(&format!("write {}: {err}\n", version_path.display()));
        PluginInstallError {
            message: format!("failed to write {}", version_path.display()),
            log: log.clone(),
        }
    })?;
    log.push_str(&format!(
        "wrote {} ({MINIMUM_PLUGIN_VERSION})\n",
        version_path.display()
    ));

    Ok(())
}

fn check_installed(hooks_dir: &Path) -> bool {
    hooks_dir.join(HOOK_JSON_FILE).is_file() && hooks_dir.join(PLUGIN_SCRIPT_REL).is_file()
}

fn installed_version(hooks_dir: &Path) -> Option<String> {
    let contents = fs::read_to_string(hooks_dir.join(VERSION_FILE)).ok()?;
    let version = contents.trim();
    if version.is_empty() {
        None
    } else {
        Some(version.to_owned())
    }
}

/// Returns `$GROK_HOME/hooks` when set, otherwise `~/.grok/hooks`.
fn grok_hooks_dir() -> io::Result<PathBuf> {
    if let Ok(home) = env::var("GROK_HOME") {
        let path = PathBuf::from(home);
        if !path.as_os_str().is_empty() {
            return Ok(path.join("hooks"));
        }
    }
    dirs::home_dir()
        .map(|home| home.join(".grok").join("hooks"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not determine home directory",
            )
        })
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "Install Warp Plugin for Grok Build",
    subtitle: "Add the Warp plugin so Grok Build can report session status to Warp. \
               Grok Build already works with Warp's toolbar and rich input without this step.",
    steps: &[
        PluginInstructionStep {
            description: "Ensure Grok Build is installed (default path on macOS/Linux):",
            command: "~/.grok/bin/grok",
            executable: false,
            link: Some("https://x.ai/cli"),
        },
        PluginInstructionStep {
            description: "Create the user hooks directory if it does not exist:",
            command: "mkdir -p ~/.grok/hooks/bin",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: "Add the Warp plugin hook JSON (auto-install writes this for you):",
            command: "~/.grok/hooks/warp-plugin.json",
            executable: false,
            link: Some("https://github.com/warpdotdev/warp/issues/11727"),
        },
        PluginInstructionStep {
            description: "Restart the Grok Build session (exit and run `grok` again) so the plugin loads.",
            command: "grok",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: &[
        "Without the plugin, Warp still detects Grok Build and offers the toolbar, rich input, and image paste.",
        "Native OSC 9 notifications from Grok Build provide basic turn-complete signals before the Warp plugin is installed.",
    ],
});

static UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "Update Warp Plugin for Grok Build",
    subtitle: "Refresh the Warp plugin under ~/.grok/hooks if a newer version is available.",
    steps: &[
        PluginInstructionStep {
            description: "Replace the Warp plugin files under:",
            command: "~/.grok/hooks/",
            executable: false,
            link: Some("https://github.com/warpdotdev/warp/issues/11727"),
        },
        PluginInstructionStep {
            description: "Restart the Grok Build session to load the updated plugin.",
            command: "grok",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: &["Run `grok` again after updating the plugin."],
});

#[cfg(test)]
#[path = "grok_tests.rs"]
mod tests;
