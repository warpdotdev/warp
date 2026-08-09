use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "local_fs")]
use anyhow::Context;
use async_trait::async_trait;
#[cfg(feature = "local_fs")]
use command::r#async::Command;

use crate::CommandBuilder;
use crate::language_server_candidate::{LanguageServerCandidate, LanguageServerMetadata};
#[cfg(feature = "local_fs")]
use crate::supported_servers::CustomBinaryConfig;

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub struct SolidityLanguageServerCandidate {
    client: Arc<http_client::Client>,
}

impl SolidityLanguageServerCandidate {
    /// Path to the language server entrypoint relative to the install directory.
    #[cfg(feature = "local_fs")]
    const SERVER_PATH: &str = "node_modules/@nomicfoundation/solidity-language-server/out/index.js";

    pub fn new(client: Arc<http_client::Client>) -> Self {
        Self { client }
    }

    /// Finds the configuration for running the Solidity language server from our custom installation.
    ///
    /// Instead of running the wrapper script (which has a shebang requiring node in PATH),
    /// we run node directly with the packaged JS entrypoint.
    ///
    /// # Arguments
    /// * `path_env_var` - The PATH environment variable to use when checking for system node.
    #[cfg(feature = "local_fs")]
    pub async fn find_installed_binary_config(
        path_env_var: Option<&str>,
    ) -> Option<CustomBinaryConfig> {
        let install_dir = warp_core::paths::data_dir().join("solidity-language-server");
        let server_js = install_dir.join(Self::SERVER_PATH);

        if !server_js.is_file() {
            log::info!(
                "Solidity language server JS file not found at {}",
                server_js.display()
            );
            return None;
        }

        let node_binary = node_runtime::find_working_node_binary(path_env_var).await?;

        let mut cmd = Command::new(&node_binary);
        if let Some(path) = path_env_var {
            cmd.env("PATH", path);
        }
        cmd.arg(&server_js).arg("--version");
        match cmd.output().await {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout);
                log::info!(
                    "Verified Solidity language server installation: {}",
                    version.trim()
                );
            }
            Ok(output) => {
                log::warn!(
                    "Solidity language server version check failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return None;
            }
            Err(e) => {
                log::warn!(
                    "Failed to run Solidity language server version check: {}",
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
impl LanguageServerCandidate for SolidityLanguageServerCandidate {
    async fn should_suggest_for_repo(&self, path: &Path, _executor: &CommandBuilder) -> bool {
        // Check for common Solidity project indicators
        path.join("hardhat.config.js").exists()
            || path.join("hardhat.config.ts").exists()
            || path.join("foundry.toml").exists()
            || path.join("truffle.js").exists()
            || path.join("truffle-config.js").exists()
            || path.join("ape-config.yaml").exists()
    }

    async fn is_installed_in_data_dir(&self, executor: &CommandBuilder) -> bool {
        Self::find_installed_binary_config(executor.path_env_var())
            .await
            .is_some()
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        executor
            .command("nomicfoundation-solidity-language-server")
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
            "Installing @nomicfoundation/solidity-language-server version {}",
            metadata.version
        );

        let install_dir = warp_core::paths::data_dir().join("solidity-language-server");

        async_fs::create_dir_all(&install_dir)
            .await
            .context("Failed to create Solidity language server installation directory")?;

        let use_system_node = match executor.path_env_var() {
            Some(path) => node_runtime::detect_system_node(path).await.is_ok(),
            None => false,
        };

        let custom_node_paths = if use_system_node {
            log::info!("Using system Node.js for Solidity language server installation");
            None
        } else {
            log::info!("System Node.js not found or too old, installing custom Node.js");
            node_runtime::install_npm(&self.client).await?;
            Some((
                node_runtime::node_binary_path()?,
                node_runtime::npm_binary_path()?,
            ))
        };

        log::info!(
            "Installing @nomicfoundation/solidity-language-server@{} using npm",
            metadata.version
        );

        let mut cmd = if let Some((node_path, npm_path)) = &custom_node_paths {
            let mut c = executor.command(node_path);
            c.arg(npm_path);
            c
        } else {
            executor.command("npm")
        };

        cmd.arg("install")
            .arg("--ignore-scripts")
            .arg(format!(
                "@nomicfoundation/solidity-language-server@{}",
                metadata.version
            ))
            .current_dir(&install_dir);

        let output = cmd.output().await.context("Failed to run npm install")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Failed to install @nomicfoundation/solidity-language-server via npm: {}",
                stderr
            );
        }

        log::info!("@nomicfoundation/solidity-language-server installed successfully");
        Ok(())
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        let version = node_runtime::fetch_npm_package_version(
            &self.client,
            "@nomicfoundation/solidity-language-server",
        )
        .await
        .context(
            "Failed to fetch @nomicfoundation/solidity-language-server version from npm registry",
        )?;

        Ok(LanguageServerMetadata {
            version,
            url: None,
            digest: None,
        })
    }
}

#[async_trait]
#[cfg(not(feature = "local_fs"))]
impl LanguageServerCandidate for SolidityLanguageServerCandidate {
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
