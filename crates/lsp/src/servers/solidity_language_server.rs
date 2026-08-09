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
#[cfg(feature = "local_fs")]
const SOLIDITY_MIN_NODE_VERSION: (u64, u64, u64) = (20, 9, 0);

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

        let node_binary = find_node_binary_for_solidity(path_env_var).await?;

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
        binary_exists_on_path(executor.path_env_var(), SOLIDITY_SERVER_BINARY_NAME)
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

        let use_system_node = system_node_meets_solidity_requirement(executor.path_env_var()).await;

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

/// Finds a Node.js binary that can run the Solidity language server.
///
/// Prefers Warp's pinned custom Node install, then falls back to system Node only when it
/// satisfies the package's `engines` requirement (`>=20.9.0`).
#[cfg(feature = "local_fs")]
async fn find_node_binary_for_solidity(path_env_var: Option<&str>) -> Option<std::path::PathBuf> {
    if let Ok(custom_node) = node_runtime::node_binary_path()
        && custom_node.is_file()
    {
        log::info!(
            "Using custom node installation for Solidity language server at {}",
            custom_node.display()
        );
        return Some(custom_node);
    }

    if system_node_meets_solidity_requirement(path_env_var).await {
        log::info!("Using system node for Solidity language server");
        return Some(std::path::PathBuf::from("node"));
    }

    None
}

/// Returns true when system Node.js is available and meets the Solidity server's engine requirement.
#[cfg(feature = "local_fs")]
async fn system_node_meets_solidity_requirement(path_env_var: Option<&str>) -> bool {
    let Some(path) = path_env_var else {
        return false;
    };

    // Still require Warp's generic minimum first so we share the same PATH resolution path.
    if node_runtime::detect_system_node(path).await.is_err() {
        return false;
    }

    let mut cmd = command::r#async::Command::new("node");
    cmd.env("PATH", path).arg("--version");
    match cmd.output().await {
        Ok(output) if output.status.success() => {
            let version_str = String::from_utf8_lossy(&output.stdout);
            let version_str = version_str.trim().trim_start_matches('v');
            match parse_semver_tuple(version_str) {
                Some(version) if version >= SOLIDITY_MIN_NODE_VERSION => {
                    log::info!(
                        "System Node.js {} meets Solidity language server requirement (>= {}.{}.{})",
                        version_str,
                        SOLIDITY_MIN_NODE_VERSION.0,
                        SOLIDITY_MIN_NODE_VERSION.1,
                        SOLIDITY_MIN_NODE_VERSION.2
                    );
                    true
                }
                Some(_) => {
                    log::info!(
                        "System Node.js {} is below Solidity language server requirement (>= {}.{}.{})",
                        version_str,
                        SOLIDITY_MIN_NODE_VERSION.0,
                        SOLIDITY_MIN_NODE_VERSION.1,
                        SOLIDITY_MIN_NODE_VERSION.2
                    );
                    false
                }
                None => {
                    log::warn!("Failed to parse system Node.js version: {version_str}");
                    false
                }
            }
        }
        Ok(output) => {
            log::warn!(
                "node --version failed while checking Solidity requirement: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            false
        }
        Err(e) => {
            log::warn!("Failed to run node --version for Solidity requirement check: {e}");
            false
        }
    }
}

/// Returns true if `binary_name` exists somewhere on PATH.
///
/// The Solidity language server only accepts LSP transport flags such as `--stdio`,
/// so we cannot probe it with `--version` the way we do for other Node-based servers.
#[cfg(feature = "local_fs")]
fn binary_exists_on_path(path_env_var: Option<&str>, binary_name: &str) -> bool {
    let Some(path_env_var) = path_env_var else {
        return false;
    };

    for dir in std::env::split_paths(path_env_var) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return true;
        }

        #[cfg(windows)]
        {
            for extension in ["cmd", "bat", "exe"] {
                let windows_candidate = dir.join(format!("{binary_name}.{extension}"));
                if windows_candidate.is_file() {
                    return true;
                }
            }
        }
    }

    false
}

#[cfg(feature = "local_fs")]
fn parse_semver_tuple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("0")
        .parse()
        .ok()?;
    Some((major, minor, patch))
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
