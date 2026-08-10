use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "local_fs")]
use anyhow::Context;
use async_trait::async_trait;

use crate::CommandBuilder;
use crate::language_server_candidate::{LanguageServerCandidate, LanguageServerMetadata};
#[cfg(feature = "local_fs")]
use crate::supported_servers::CustomBinaryConfig;

#[cfg(feature = "local_fs")]
const SOLIDITY_SERVER_BINARY_NAME: &str = "nomicfoundation-solidity-language-server";

/// Minimum Node.js version required by `@nomicfoundation/solidity-language-server`.
///
/// Matches the package's published `engines.node` field. Node itself does not enforce
/// engines, but Warp gates PATH installs and system-Node selection on this floor so a
/// stale global install cannot block the managed-install fallback.
#[cfg(feature = "local_fs")]
const SOLIDITY_MIN_NODE_VERSION: node_runtime::Version = node_runtime::Version::new(20, 9, 0);

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
    /// The published entrypoint only accepts LSP transport flags such as `--stdio`; it does
    /// not implement `--version`. Custom-install verification therefore checks that the JS
    /// entrypoint exists and that a usable Node binary is available.
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

        let node_binary = node_runtime::find_working_node_binary_with_min(
            path_env_var,
            SOLIDITY_MIN_NODE_VERSION,
        )
        .await?;

        log::info!(
            "Found Solidity language server JS file at {}",
            server_js.display()
        );

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
        // The published server only accepts LSP transport flags such as `--stdio`, so we
        // cannot probe it with `--version` the way we do for pyright/typescript-language-server.
        // PATH must still only win when the wrapper is actually runnable: the binary has to
        // exist/be executable, and system Node must satisfy the package engines floor.
        // Otherwise Warp would skip the managed-install fallback for a broken global install.
        if !executable_exists_on_path(executor.path_env_var(), SOLIDITY_SERVER_BINARY_NAME) {
            return false;
        }

        let Some(path) = executor.path_env_var() else {
            return false;
        };

        match node_runtime::detect_system_node_with_min(path, SOLIDITY_MIN_NODE_VERSION).await {
            Ok(()) => true,
            Err(err) => {
                log::info!(
                    "Ignoring PATH install of {SOLIDITY_SERVER_BINARY_NAME} because system Node does not meet >= {SOLIDITY_MIN_NODE_VERSION}: {err}"
                );
                false
            }
        }
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
            Some(path) => {
                node_runtime::detect_system_node_with_min(path, SOLIDITY_MIN_NODE_VERSION)
                    .await
                    .is_ok()
            }
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

/// Returns true if an executable named `binary_name` exists somewhere on PATH.
///
/// The Solidity language server only accepts LSP transport flags such as `--stdio`,
/// so we cannot probe it with `--version` the way we do for other Node-based servers.
#[cfg(feature = "local_fs")]
fn executable_exists_on_path(path_env_var: Option<&str>, binary_name: &str) -> bool {
    let Some(path_env_var) = path_env_var else {
        return false;
    };

    for dir in std::env::split_paths(path_env_var) {
        let candidate = dir.join(binary_name);
        if is_executable_file(&candidate) {
            return true;
        }

        #[cfg(windows)]
        {
            for extension in ["cmd", "bat", "exe"] {
                let windows_candidate = dir.join(format!("{binary_name}.{extension}"));
                if is_executable_file(&windows_candidate) {
                    return true;
                }
            }
        }
    }

    false
}

#[cfg(feature = "local_fs")]
fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        path.is_file()
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

#[cfg(all(test, feature = "local_fs", unix))]
#[path = "solidity_language_server_tests.rs"]
mod tests;
