use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{env, fs, io};

use async_trait::async_trait;
use serde_json::Value;

use super::{
    compare_versions, run_cli_command_logged, CliAgentPluginManager, PluginInstallError,
    PluginInstructionStep, PluginInstructions,
};
use crate::terminal::model::session::LocalCommandExecutor;
use crate::terminal::shell::ShellType;

const PLUGIN_KEY: &str = "warp@codex-warp";
const MARKETPLACE_REPO: &str = "warpdotdev/codex-warp";
const MARKETPLACE_NAME: &str = "codex-warp";

const PLATFORM_PLUGIN_KEY: &str = "orchestration@codex-warp";

const CODEX_CONFIG_DIR: &str = ".codex";
const CODEX_HOME_ENV: &str = "CODEX_HOME";

// Keep in sync with the plugin version in warpdotdev/codex-warp.
const MINIMUM_PLUGIN_VERSION: &str = "0.4.0";

pub(super) struct CodexPluginManager {
    executor: LocalCommandExecutor,
    path_env_var: Option<String>,
}

impl CodexPluginManager {
    pub(super) fn new(
        shell_path: Option<PathBuf>,
        shell_type: Option<ShellType>,
        path_env_var: Option<String>,
    ) -> Self {
        let shell_type = shell_type.unwrap_or(ShellType::Bash);
        Self {
            executor: LocalCommandExecutor::new(shell_path, shell_type),
            path_env_var,
        }
    }

    async fn run_logged(&self, args: &[&str], log: &mut String) -> Result<(), PluginInstallError> {
        let env_vars = self
            .path_env_var
            .as_deref()
            .map(|path| HashMap::from([("PATH".to_owned(), path.to_owned())]));
        run_cli_command_logged("codex", args, &self.executor, env_vars, log).await
    }
}

#[async_trait]
impl CliAgentPluginManager for CodexPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        MINIMUM_PLUGIN_VERSION
    }

    fn can_auto_install(&self) -> bool {
        true
    }

    fn is_installed(&self) -> bool {
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        check_installed(&codex_dir)
    }

    fn needs_update(&self) -> bool {
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        match installed_version(&codex_dir) {
            Some(v) => compare_versions(&v, MINIMUM_PLUGIN_VERSION).is_lt(),
            None => check_installed(&codex_dir),
        }
    }

    async fn install(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(
            &["plugin", "marketplace", "add", MARKETPLACE_REPO],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "add", PLUGIN_KEY], &mut log)
            .await?;
        Ok(())
    }

    async fn update(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(
            &["plugin", "marketplace", "upgrade", MARKETPLACE_NAME],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "add", PLUGIN_KEY], &mut log)
            .await?;

        let still_outdated = codex_home_dir()
            .ok()
            .and_then(|dir| installed_version(&dir))
            .map(|v| compare_versions(&v, MINIMUM_PLUGIN_VERSION).is_lt())
            .unwrap_or(true);
        if still_outdated {
            log.push_str("Post-update version check: plugin is still outdated\n");
            return Err(PluginInstallError {
                message: "Plugin update did not take effect".to_owned(),
                log,
            });
        }
        Ok(())
    }

    fn install_success_message(&self) -> &'static str {
        "Warp plugin installed. Please restart Codex to activate."
    }

    fn update_success_message(&self) -> &'static str {
        "Warp plugin updated. Please restart Codex to activate."
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &UPDATE_INSTRUCTIONS
    }

    async fn install_platform_plugin(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(
            &["plugin", "marketplace", "add", MARKETPLACE_REPO],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "add", PLATFORM_PLUGIN_KEY], &mut log)
            .await?;
        Ok(())
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "Install Warp Plugin for Codex",
    subtitle: "Run the following commands, then restart Codex.",
    steps: &[
        PluginInstructionStep {
            description: "Add the Warp plugin marketplace repository",
            command: "codex plugin marketplace add warpdotdev/codex-warp",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: "Install the Warp plugin",
            command: "codex plugin add warp@codex-warp",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: &["Restart Codex to activate the plugin."],
});

static UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "Update Warp Plugin for Codex",
    subtitle: "Run the following commands, then restart Codex.",
    steps: &[
        PluginInstructionStep {
            description: "Upgrade the Warp plugin marketplace",
            command: "codex plugin marketplace upgrade codex-warp",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: "Reinstall the Warp plugin",
            command: "codex plugin add warp@codex-warp",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: &["Restart Codex to activate the update."],
});

fn check_installed(codex_dir: &Path) -> bool {
    let config_path = codex_dir.join("config.toml");
    let Ok(contents) = fs::read_to_string(config_path) else {
        return false;
    };
    let Ok(parsed) = contents.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    parsed
        .get("plugins")
        .and_then(|plugins| plugins.get(PLUGIN_KEY))
        .and_then(|plugin| plugin.get("enabled"))
        .and_then(|enabled| enabled.as_bool())
        .unwrap_or(false)
}

/// Reads the latest cached Warp plugin version, if present.
fn installed_version(codex_dir: &Path) -> Option<String> {
    let cache_dir = codex_dir
        .join("plugins")
        .join("cache")
        .join(MARKETPLACE_NAME)
        .join("warp");
    let entries = fs::read_dir(cache_dir).ok()?;
    let mut latest: Option<String> = None;
    for entry in entries.flatten() {
        let manifest_path = entry.path().join(".codex-plugin").join("plugin.json");
        let Ok(contents) = fs::read_to_string(manifest_path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(&contents) else {
            continue;
        };
        let Some(version) = parsed.get("version").and_then(|v| v.as_str()) else {
            continue;
        };
        if latest
            .as_deref()
            .map(|current| compare_versions(version, current).is_gt())
            .unwrap_or(true)
        {
            latest = Some(version.to_owned());
        }
    }
    latest
}

/// Checks `CODEX_HOME` first, falls back to `~/.codex`.
fn codex_home_dir() -> io::Result<PathBuf> {
    if let Ok(codex_home) = env::var(CODEX_HOME_ENV) {
        if !codex_home.is_empty() {
            return Ok(PathBuf::from(codex_home));
        }
    }
    dirs::home_dir()
        .map(|home| home.join(CODEX_CONFIG_DIR))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not determine home directory",
            )
        })
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
