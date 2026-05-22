use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "local_fs")]
use anyhow::Context;
use async_trait::async_trait;
#[cfg(feature = "local_fs")]
use command::r#async::Command;

use crate::language_server_candidate::{LanguageServerCandidate, LanguageServerMetadata};
#[cfg(feature = "local_fs")]
use crate::supported_servers::CustomBinaryConfig;
use crate::CommandBuilder;

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub struct PyrightCandidate {
    client: Arc<http_client::Client>,
}

impl PyrightCandidate {
    /// Path to the langserver JS file relative to the pyright install directory.
    #[cfg(feature = "local_fs")]
    const LANGSERVER_JS_PATH: &str = "node_modules/pyright/langserver.index.js";

    /// Path to the pyright CLI JS file (used for version checks).
    #[cfg(feature = "local_fs")]
    const PYRIGHT_CLI_PATH: &str = "node_modules/pyright/dist/pyright.js";

    pub fn new(client: Arc<http_client::Client>) -> Self {
        Self { client }
    }

    /// Finds the configuration for running pyright from our custom installation.
    ///
    /// Instead of running the `pyright-langserver` wrapper script (which has a shebang
    /// requiring node in PATH), we run node directly with the langserver.index.js file.
    /// This is the same pattern used by Zed.
    ///
    /// Pyright can be installed with either system node or our custom node. This function
    /// checks for both cases:
    /// 1. First tries our custom node installation
    /// 2. Falls back to system node if custom node isn't available
    ///
    /// # Arguments
    /// * `path_env_var` - The PATH environment variable to use when checking for system node.
    #[cfg(feature = "local_fs")]
    pub async fn find_installed_binary_config(
        path_env_var: Option<&str>,
    ) -> Option<CustomBinaryConfig> {
        let install_dir = warp_core::paths::data_dir().join("pyright");
        let langserver_js = install_dir.join(Self::LANGSERVER_JS_PATH);

        // Check if the JS file exists
        if !langserver_js.is_file() {
            log::info!(
                "Pyright langserver.index.js not found at {}",
                langserver_js.display()
            );
            return None;
        }

        // Try to find a working node binary - first custom, then system
        let node_binary = node_runtime::find_working_node_binary(path_env_var).await?;

        // Verify the pyright installation works by running `node pyright.js --version`
        let pyright_cli = install_dir.join(Self::PYRIGHT_CLI_PATH);
        if pyright_cli.is_file() {
            let mut cmd = Command::new(&node_binary);
            // Propagate PATH so "node" (bare name) resolves when using system node
            if let Some(path) = path_env_var {
                cmd.env("PATH", path);
            }
            cmd.arg(&pyright_cli).arg("--version");
            match cmd.output().await {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout);
                    log::info!("Verified pyright installation: {}", version.trim());
                }
                Ok(output) => {
                    log::warn!(
                        "Pyright version check failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return None;
                }
                Err(e) => {
                    log::warn!("Failed to run pyright version check: {}", e);
                    return None;
                }
            }
        } else {
            log::warn!(
                "Pyright CLI not found at {}, skipping version check",
                pyright_cli.display()
            );
            // Still proceed - the langserver.index.js exists, installation might still work
        }

        Some(CustomBinaryConfig {
            binary_path: node_binary,
            prepend_args: vec![langserver_js.to_string_lossy().to_string()],
        })
    }
}

#[async_trait]
#[cfg(feature = "local_fs")]
impl LanguageServerCandidate for PyrightCandidate {
    async fn should_suggest_for_repo(&self, path: &Path, _executor: &CommandBuilder) -> bool {
        // Check for common Python project indicators
        path.join("pyproject.toml").exists()
            || path.join("setup.py").exists()
            || path.join("requirements.txt").exists()
            || path.join("Pipfile").exists()
    }

    async fn is_installed_in_data_dir(&self, executor: &CommandBuilder) -> bool {
        Self::find_installed_binary_config(executor.path_env_var())
            .await
            .is_some()
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        executor
            .command("pyright-langserver")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn install(
        &self,
        metadata: LanguageServerMetadata,
        executor: &CommandBuilder,
    ) -> anyhow::Result<()> {
        log::info!("Installing pyright version {}", metadata.version);

        let install_dir = warp_core::paths::data_dir().join("pyright");

        // Create the installation directory
        async_fs::create_dir_all(&install_dir)
            .await
            .context("Failed to create pyright installation directory")?;

        // First, check if system node is available and meets requirements
        let use_system_node = match executor.path_env_var() {
            Some(path) => node_runtime::detect_system_node(path).await.is_ok(),
            None => false,
        };

        let custom_node_paths = if use_system_node {
            log::info!("Using system Node.js for pyright installation");
            None
        } else {
            log::info!("System Node.js not found or too old, installing custom Node.js");
            node_runtime::install_npm(&self.client).await?;
            Some((
                node_runtime::node_binary_path()?,
                node_runtime::npm_binary_path()?,
            ))
        };

        // Install pyright using npm
        log::info!("Installing pyright@{} using npm", metadata.version);

        // Build the npm install command:
        // - System node: run `npm` directly (it's on PATH)
        // - Custom node: run `node <npm_path>` to avoid relying on shebang resolution
        let mut cmd = if let Some((node_path, npm_path)) = &custom_node_paths {
            let mut c = executor.command(node_path);
            c.arg(npm_path);
            c
        } else {
            executor.command("npm")
        };

        cmd.arg("install")
            .arg("--ignore-scripts")
            .arg(format!("pyright@{}", metadata.version))
            .current_dir(&install_dir);

        let output = cmd.output().await.context("Failed to run npm install")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to install pyright via npm: {}", stderr);
        }

        log::info!("Pyright installed successfully");
        Ok(())
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        let version = node_runtime::fetch_npm_package_version(&self.client, "pyright")
            .await
            .context("Failed to fetch pyright version from npm registry")?;

        Ok(LanguageServerMetadata {
            version,
            url: None, // npm packages don't have direct download URLs
            digest: None,
        })
    }
}

#[async_trait]
#[cfg(not(feature = "local_fs"))]
impl LanguageServerCandidate for PyrightCandidate {
    async fn should_suggest_for_repo(&self, _path: &Path, _executor: &CommandBuilder) -> bool {
        false
    }

    async fn is_installed_in_data_dir(&self, _executor: &CommandBuilder) -> bool {
        false
    }

    async fn is_installed_on_path(&self, _executor: &CommandBuilder) -> bool {
        false
    }

    async fn install(
        &self,
        _metadata: LanguageServerMetadata,
        _executor: &CommandBuilder,
    ) -> anyhow::Result<()> {
        todo!()
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        todo!()
    }
}
