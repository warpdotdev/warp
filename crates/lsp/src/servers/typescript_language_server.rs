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
pub struct TypeScriptLanguageServerCandidate {
    client: Arc<http_client::Client>,
}

impl TypeScriptLanguageServerCandidate {
    /// Path to the new langserver JS file (v4.0.0+) relative to the install directory.
    #[cfg(feature = "local_fs")]
    const NEW_SERVER_PATH: &str = "node_modules/typescript-language-server/lib/cli.mjs";

    /// Path to the old langserver JS file (pre-4.0.0) relative to the install directory.
    #[cfg(feature = "local_fs")]
    const OLD_SERVER_PATH: &str = "node_modules/typescript-language-server/lib/cli.js";

    pub fn new(client: Arc<http_client::Client>) -> Self {
        Self { client }
    }

    /// Finds the configuration for running typescript-language-server from our custom installation.
    ///
    /// Instead of running the wrapper script (which has a shebang requiring node in PATH),
    /// we run node directly with the CLI JS file. This is the same pattern used by Zed.
    ///
    /// # Arguments
    /// * `path_env_var` - The PATH environment variable to use when checking for system node.
    #[cfg(feature = "local_fs")]
    pub async fn find_installed_binary_config(
        path_env_var: Option<&str>,
    ) -> Option<CustomBinaryConfig> {
        let install_dir = warp_core::paths::data_dir().join("typescript-language-server");

        // Check for the JS file - prefer new path (cli.mjs) over old path (cli.js)
        let server_js = {
            let new_path = install_dir.join(Self::NEW_SERVER_PATH);
            if new_path.is_file() {
                new_path
            } else {
                let old_path = install_dir.join(Self::OLD_SERVER_PATH);
                if old_path.is_file() {
                    old_path
                } else {
                    log::info!(
                        "typescript-language-server JS file not found at {} or {}",
                        new_path.display(),
                        old_path.display()
                    );
                    return None;
                }
            }
        };

        // Try to find a working node binary - first custom, then system
        let node_binary = node_runtime::find_working_node_binary(path_env_var).await?;

        // Verify the installation works by running `node cli.mjs --version`
        let mut cmd = Command::new(&node_binary);
        // Propagate PATH so "node" (bare name) resolves when using system node
        if let Some(path) = path_env_var {
            cmd.env("PATH", path);
        }
        cmd.arg(&server_js).arg("--version");
        match cmd.output().await {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout);
                log::info!(
                    "Verified typescript-language-server installation: {}",
                    version.trim()
                );
            }
            Ok(output) => {
                log::warn!(
                    "typescript-language-server version check failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return None;
            }
            Err(e) => {
                log::warn!(
                    "Failed to run typescript-language-server version check: {}",
                    e
                );
                return None;
            }
        }

        Some(CustomBinaryConfig {
            binary_path: node_binary,
            prepend_args: vec![server_js.to_string_lossy().to_string()],
        })
    }
}

#[async_trait]
#[cfg(feature = "local_fs")]
impl LanguageServerCandidate for TypeScriptLanguageServerCandidate {
    async fn should_suggest_for_repo(&self, path: &Path, _executor: &CommandBuilder) -> bool {
        // Check for common JavaScript/TypeScript project indicators
        path.join("package.json").exists()
            || path.join("tsconfig.json").exists()
            || path.join("jsconfig.json").exists()
    }

    async fn is_installed_in_data_dir(&self, executor: &CommandBuilder) -> bool {
        Self::find_installed_binary_config(executor.path_env_var())
            .await
            .is_some()
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        executor
            .command("typescript-language-server")
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
        log::info!(
            "Installing typescript-language-server version {}",
            metadata.version
        );

        let install_dir = warp_core::paths::data_dir().join("typescript-language-server");

        // Create the installation directory
        async_fs::create_dir_all(&install_dir)
            .await
            .context("Failed to create typescript-language-server installation directory")?;

        // First, check if system node is available and meets requirements
        let use_system_node = match executor.path_env_var() {
            Some(path) => node_runtime::detect_system_node(path).await.is_ok(),
            None => false,
        };

        let custom_node_paths = if use_system_node {
            log::info!("Using system Node.js for typescript-language-server installation");
            None
        } else {
            log::info!("System Node.js not found or too old, installing custom Node.js");
            node_runtime::install_npm(&self.client).await?;
            Some((
                node_runtime::node_binary_path()?,
                node_runtime::npm_binary_path()?,
            ))
        };

        // Install typescript-language-server and typescript using npm
        // typescript is a peer dependency required for the language server to work
        log::info!(
            "Installing typescript-language-server@{} using npm",
            metadata.version
        );

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
            .arg(format!("typescript-language-server@{}", metadata.version))
            .arg("typescript")
            .current_dir(&install_dir);

        let output = cmd.output().await.context("Failed to run npm install")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Failed to install typescript-language-server via npm: {}",
                stderr
            );
        }

        log::info!("typescript-language-server installed successfully");
        Ok(())
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        let version =
            node_runtime::fetch_npm_package_version(&self.client, "typescript-language-server")
                .await
                .context("Failed to fetch typescript-language-server version from npm registry")?;

        Ok(LanguageServerMetadata {
            version,
            url: None, // npm packages don't have direct download URLs
            digest: None,
        })
    }
}

#[async_trait]
#[cfg(not(feature = "local_fs"))]
impl LanguageServerCandidate for TypeScriptLanguageServerCandidate {
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
