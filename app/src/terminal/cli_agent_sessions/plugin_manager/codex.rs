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
use crate::features::FeatureFlag;
use crate::terminal::model::session::LocalCommandExecutor;
use crate::terminal::shell::ShellType;

const PLUGIN_NAME: &str = "warp";
const PLUGIN_KEY: &str = "warp@codex-warp";
const MARKETPLACE_REPO: &str = "warpdotdev/codex-warp";
const MARKETPLACE_NAME: &str = "codex-warp";

const PLATFORM_PLUGIN_NAME: &str = "orchestration";
const PLATFORM_PLUGIN_KEY: &str = "orchestration@codex-warp";

const CODEX_CONFIG_DIR: &str = ".codex";
const CODEX_HOME_ENV: &str = "CODEX_HOME";

// Keep in sync with the plugin version in warpdotdev/codex-warp.
const MINIMUM_PLUGIN_VERSION: &str = "0.4.0";
// Keep in sync with the orchestration plugin version in warpdotdev/codex-warp.
const MINIMUM_PLATFORM_PLUGIN_VERSION: &str = "0.4.0";

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

    /// Ensures the codex-warp marketplace is registered, while preserving a
    /// non-Git/local marketplace override. If the marketplace is already a
    /// Git repo, upgrade it; if it is a non-Git source, leave it alone; otherwise
    /// add it from the canonical repository.
    async fn ensure_marketplace(&self, log: &mut String) -> Result<(), PluginInstallError> {
        match codex_home_dir()
            .ok()
            .and_then(|dir| codex_warp_marketplace_config(&dir))
        {
            Some(config) if config.is_git() => {
                self.run_logged(&["plugin", "marketplace", "upgrade", MARKETPLACE_NAME], log)
                    .await
            }
            Some(_) => Ok(()),
            None => {
                self.run_logged(&["plugin", "marketplace", "add", MARKETPLACE_REPO], log)
                    .await
            }
        }
    }
}

#[async_trait]
impl CliAgentPluginManager for CodexPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        if FeatureFlag::CodexPlugin.is_enabled() {
            MINIMUM_PLUGIN_VERSION
        } else {
            "0.0.0"
        }
    }

    fn can_auto_install(&self) -> bool {
        FeatureFlag::CodexPlugin.is_enabled()
    }

    fn is_installed(&self) -> bool {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return false;
        }
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        check_installed(&codex_dir)
    }

    fn needs_update(&self) -> bool {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return false;
        }
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        if codex_warp_marketplace_config(&codex_dir).is_some_and(|config| !config.is_git()) {
            return false;
        }
        plugin_needs_update(&codex_dir, PLUGIN_NAME, PLUGIN_KEY, MINIMUM_PLUGIN_VERSION)
    }

    fn is_platform_plugin_installed(&self) -> bool {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return false;
        }
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        check_platform_plugin_installed(&codex_dir)
    }

    fn platform_plugin_needs_update(&self) -> bool {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return false;
        }
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        if codex_warp_marketplace_config(&codex_dir).is_some_and(|config| !config.is_git()) {
            return false;
        }
        plugin_needs_update(
            &codex_dir,
            PLATFORM_PLUGIN_NAME,
            PLATFORM_PLUGIN_KEY,
            MINIMUM_PLATFORM_PLUGIN_VERSION,
        )
    }

    fn has_local_marketplace_override(&self) -> bool {
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        codex_warp_marketplace_config(&codex_dir).is_some_and(|config| !config.is_git())
    }

    async fn install(&self) -> Result<(), PluginInstallError> {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return Ok(());
        }
        log::info!("[PLUGIN_INSTALL] updating codex plugin");
        let mut log = String::new();
        ensure_codex_home_dir()?;
        self.ensure_marketplace(&mut log).await?;
        self.run_logged(&["plugin", "add", PLUGIN_KEY], &mut log)
            .await?;
        Ok(())
    }

    async fn update(&self) -> Result<(), PluginInstallError> {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return Ok(());
        }
        let mut log = String::new();
        ensure_codex_home_dir()?;
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
        if FeatureFlag::CodexPlugin.is_enabled() {
            &PLUGIN_INSTALL_INSTRUCTIONS
        } else {
            &NATIVE_INSTALL_INSTRUCTIONS
        }
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        if FeatureFlag::CodexPlugin.is_enabled() {
            &PLUGIN_UPDATE_INSTRUCTIONS
        } else {
            &EMPTY_INSTRUCTIONS
        }
    }

    fn supports_update(&self) -> bool {
        FeatureFlag::CodexPlugin.is_enabled()
    }

    async fn install_platform_plugin(&self) -> Result<(), PluginInstallError> {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return Ok(());
        }
        let mut log = String::new();
        ensure_codex_home_dir()?;
        self.ensure_marketplace(&mut log).await?;
        self.run_logged(&["plugin", "add", PLATFORM_PLUGIN_KEY], &mut log)
            .await?;
        let updated = codex_home_dir()
            .ok()
            .map(|dir| platform_plugin_version_is_current(&dir))
            .unwrap_or(false);
        if !updated {
            log.push_str("Post-install version check: platform plugin is still outdated\n");
            return Err(PluginInstallError {
                message: "Platform plugin installation did not take effect".to_owned(),
                log,
            });
        }
        Ok(())
    }

    async fn update_platform_plugin(&self) -> Result<(), PluginInstallError> {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return Ok(());
        }
        let mut log = String::new();
        ensure_codex_home_dir()?;
        self.run_logged(
            &["plugin", "marketplace", "upgrade", MARKETPLACE_NAME],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "add", PLATFORM_PLUGIN_KEY], &mut log)
            .await?;
        let updated = codex_home_dir()
            .ok()
            .map(|dir| platform_plugin_version_is_current(&dir))
            .unwrap_or(false);
        if !updated {
            log.push_str("Post-update version check: platform plugin is still outdated\n");
            return Err(PluginInstallError {
                message: "Platform plugin update did not take effect".to_owned(),
                log,
            });
        }
        Ok(())
    }
}

static PLUGIN_INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> =
    LazyLock::new(|| PluginInstructions {
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

static NATIVE_INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
        title: "Enable Warp Notifications for Codex",
        subtitle: "Update Codex to the latest version, then enable in-focus notifications so Warp can display them while you work.",
        steps: &[
            PluginInstructionStep {
                description: "Update Codex to the latest version.",
                command: "",
                executable: false,
                link: Some("https://developers.openai.com/codex/cli#upgrade"),
            },
            PluginInstructionStep {
                description: "Set the notification condition to \"always\" in your Codex config. Open or create ~/.codex/config.toml and add:",
                command: "[tui]\nnotification_condition = \"always\"",
                executable: false,
                link: None,
            },
        ],
        post_install_notes: &["Restart Codex to apply the changes."],
    }
});

static EMPTY_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "",
    subtitle: "",
    steps: &[],
    post_install_notes: &[],
});

static PLUGIN_UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
        title: "Update Warp Plugin for Codex",
        subtitle: "Run the following commands, then restart Codex.",
        steps: &[
            PluginInstructionStep {
                description: "Upgrade the marketplace",
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
        post_install_notes: &[
            "Restart Codex to activate the update.",
            "If this fails because codex-warp is not configured as a Git marketplace, remove and re-add the marketplace.",
        ],
    }
});

fn check_installed(codex_dir: &Path) -> bool {
    check_plugin_enabled(codex_dir, PLUGIN_KEY)
}

fn check_platform_plugin_installed(codex_dir: &Path) -> bool {
    check_plugin_enabled(codex_dir, PLATFORM_PLUGIN_KEY)
}

/// Whether `config.toml` marks the given plugin key as enabled.
fn check_plugin_enabled(codex_dir: &Path, plugin_key: &str) -> bool {
    let config_path = codex_dir.join("config.toml");
    let Ok(contents) = fs::read_to_string(config_path) else {
        return false;
    };
    let Ok(parsed) = contents.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    parsed
        .get("plugins")
        .and_then(|plugins| plugins.get(plugin_key))
        .and_then(|plugin| plugin.get("enabled"))
        .and_then(|enabled| enabled.as_bool())
        .unwrap_or(false)
}

/// Reads the latest cached Warp plugin version, if present.
fn installed_version(codex_dir: &Path) -> Option<String> {
    installed_plugin_version(codex_dir, PLUGIN_NAME)
}

/// Reads the latest cached orchestration plugin version, if present.
fn installed_platform_plugin_version(codex_dir: &Path) -> Option<String> {
    installed_plugin_version(codex_dir, PLATFORM_PLUGIN_NAME)
}

fn platform_plugin_version_is_current(codex_dir: &Path) -> bool {
    installed_platform_plugin_version(codex_dir)
        .map(|v| !compare_versions(&v, MINIMUM_PLATFORM_PLUGIN_VERSION).is_lt())
        .unwrap_or(false)
}

/// Reads the latest cached version for `plugin_name` from
/// `plugins/cache/codex-warp/<plugin_name>/<version>/.codex-plugin/plugin.json`.
fn installed_plugin_version(codex_dir: &Path, plugin_name: &str) -> Option<String> {
    let cache_dir = codex_dir
        .join("plugins")
        .join("cache")
        .join(MARKETPLACE_NAME)
        .join(plugin_name);
    let entries = fs::read_dir(cache_dir).ok()?;
    let mut latest: Option<String> = None;
    for entry in entries.flatten() {
        let manifest_path = entry.path().join(".codex-plugin").join("plugin.json");
        let Some(version) = plugin_manifest_version(manifest_path) else {
            continue;
        };
        if latest
            .as_deref()
            .map(|current| compare_versions(&version, current).is_gt())
            .unwrap_or(true)
        {
            latest = Some(version);
        }
    }
    latest
}

fn plugin_manifest_version(manifest_path: impl AsRef<Path>) -> Option<String> {
    let contents = fs::read_to_string(manifest_path).ok()?;
    let parsed = serde_json::from_str::<Value>(&contents).ok()?;
    parsed
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

fn plugin_needs_update(
    codex_dir: &Path,
    plugin_name: &str,
    plugin_key: &str,
    minimum_version: &str,
) -> bool {
    if !check_plugin_enabled(codex_dir, plugin_key) {
        return false;
    }
    match installed_plugin_version(codex_dir, plugin_name) {
        Some(v) => compare_versions(&v, minimum_version).is_lt(),
        // No version field means very old plugin.
        None => true,
    }
}

struct CodexWarpMarketplaceConfig {
    source_type: Option<String>,
}

impl CodexWarpMarketplaceConfig {
    fn is_git(&self) -> bool {
        self.source_type.as_deref() == Some("git")
    }
}

fn codex_warp_marketplace_config(codex_dir: &Path) -> Option<CodexWarpMarketplaceConfig> {
    let config_path = codex_dir.join("config.toml");
    let contents = fs::read_to_string(config_path).ok()?;
    let parsed = contents.parse::<toml_edit::DocumentMut>().ok()?;
    let marketplace = parsed.get("marketplaces")?.get(MARKETPLACE_NAME)?;
    Some(CodexWarpMarketplaceConfig {
        source_type: marketplace
            .get("source_type")
            .and_then(|source_type| source_type.as_str())
            .map(str::to_owned),
    })
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

/// Creates the resolved Codex home directory if it does not yet exist.
/// The Codex CLI expects `CODEX_HOME` to exist before running plugin commands, we need
/// this for self-hosted direct backend workers.
fn ensure_codex_home_dir() -> io::Result<PathBuf> {
    let dir = codex_home_dir()?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
