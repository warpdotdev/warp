use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{fs, io};

use async_trait::async_trait;
use serde_json::Value;

use super::{
    compare_versions, run_cli_command_logged, CliAgentPluginManager, PluginInstallError,
    PluginInstructionStep, PluginInstructions,
};
use crate::terminal::model::session::LocalCommandExecutor;
use crate::terminal::shell::ShellType;

const EXTENSION_REPO: &str = "https://github.com/warpdotdev/gemini-cli-warp";
const EXTENSION_NAME: &str = "gemini-warp";

// Keep in sync with the plugin version in warpdotdev/gemini-warp.
const MINIMUM_PLUGIN_VERSION: &str = "1.0.0";

pub(super) struct GeminiPluginManager {
    executor: LocalCommandExecutor,
    path_env_var: Option<String>,
}

impl GeminiPluginManager {
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
        run_cli_command_logged("gemini", args, &self.executor, env_vars, log).await
    }
}

#[async_trait]
impl CliAgentPluginManager for GeminiPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        MINIMUM_PLUGIN_VERSION
    }

    fn can_auto_install(&self) -> bool {
        true
    }

    fn is_installed(&self) -> bool {
        let Ok(extensions_dir) = gemini_extensions_dir() else {
            return false;
        };
        check_installed(&extensions_dir)
    }

    fn needs_update(&self) -> bool {
        let Ok(extensions_dir) = gemini_extensions_dir() else {
            return false;
        };
        match installed_version(&extensions_dir) {
            Some(v) => compare_versions(&v, MINIMUM_PLUGIN_VERSION).is_lt(),
            // No version field means very old or malformed extension.
            None => check_installed(&extensions_dir),
        }
    }

    async fn install(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(
            &["extensions", "install", EXTENSION_REPO, "--consent"],
            &mut log,
        )
        .await?;
        Ok(())
    }

    async fn update(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(&["extensions", "update", EXTENSION_NAME], &mut log)
            .await?;

        // Sanity check: verify the on-disk version actually changed.
        let still_outdated = gemini_extensions_dir()
            .ok()
            .and_then(|dir| installed_version(&dir))
            .map(|v| compare_versions(&v, MINIMUM_PLUGIN_VERSION).is_lt())
            .unwrap_or(true);
        if still_outdated {
            log.push_str("Post-update version check: plugin is still outdated\n");
            return Err(PluginInstallError {
                message: i18n::t("terminal.plugin_instructions.gemini.error.update_no_effect"),
                log,
            });
        }
        Ok(())
    }

    fn install_success_message(&self) -> String {
        i18n::t("terminal.plugin_instructions.gemini.success.installed_restart")
    }

    fn update_success_message(&self) -> String {
        i18n::t("terminal.plugin_instructions.gemini.success.updated_restart")
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &UPDATE_INSTRUCTIONS
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "terminal.plugin_instructions.gemini.install.title",
    subtitle: "terminal.plugin_instructions.gemini.install.subtitle",
    steps: &[PluginInstructionStep {
        description: "terminal.plugin_instructions.gemini.install.step.install_extension",
        command:
            "gemini extensions install https://github.com/warpdotdev/gemini-cli-warp --consent",
        executable: true,
        link: None,
    }],
    post_install_notes: &["terminal.plugin_instructions.gemini.install.note.restart"],
});

static UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "terminal.plugin_instructions.gemini.update.title",
    subtitle: "terminal.plugin_instructions.gemini.install.subtitle",
    steps: &[PluginInstructionStep {
        description: "terminal.plugin_instructions.gemini.update.step.update_extension",
        command: "gemini extensions update gemini-warp",
        executable: true,
        link: None,
    }],
    post_install_notes: &["terminal.plugin_instructions.gemini.update.note.restart"],
});

fn check_installed(extensions_dir: &Path) -> bool {
    let manifest_path = extensions_dir
        .join(EXTENSION_NAME)
        .join("gemini-extension.json");
    let Ok(contents) = fs::read_to_string(manifest_path) else {
        return false;
    };
    serde_json::from_str::<Value>(&contents).is_ok()
}

/// Reads the installed version string for the Warp extension, if present.
fn installed_version(extensions_dir: &Path) -> Option<String> {
    let manifest_path = extensions_dir
        .join(EXTENSION_NAME)
        .join("gemini-extension.json");
    let contents = fs::read_to_string(manifest_path).ok()?;
    let parsed: Value = serde_json::from_str(&contents).ok()?;
    parsed.get("version")?.as_str().map(|s| s.to_owned())
}

/// Returns the path to `~/.gemini/extensions`.
fn gemini_extensions_dir() -> io::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".gemini").join("extensions"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                i18n::t("terminal.plugin_instructions.error.home_directory_not_found"),
            )
        })
}

#[cfg(test)]
#[path = "gemini_tests.rs"]
mod tests;
