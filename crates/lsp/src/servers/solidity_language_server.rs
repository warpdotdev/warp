use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "local_fs")]
use anyhow::Context;
use async_trait::async_trait;

use crate::CommandBuilder;
use crate::language_server_candidate::{LanguageServerCandidate, LanguageServerMetadata};
#[cfg(feature = "local_fs")]
use crate::supported_servers::CustomBinaryConfig;

const SERVER_NAME: &str = "nomicfoundation-solidity-language-server";
const NPM_PACKAGE_NAME: &str = "@nomicfoundation/solidity-language-server";

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub struct SolidityLanguageServerCandidate {
    client: Arc<http_client::Client>,
}

impl SolidityLanguageServerCandidate {
    #[cfg(feature = "local_fs")]
    const SERVER_JS_PATH: &str =
        "node_modules/@nomicfoundation/solidity-language-server/out/index.js";

    pub fn new(client: Arc<http_client::Client>) -> Self {
        Self { client }
    }

    #[cfg(feature = "local_fs")]
    pub async fn find_installed_binary_config(
        path_env_var: Option<&str>,
    ) -> Option<CustomBinaryConfig> {
        let server_js = install_dir().join(Self::SERVER_JS_PATH);

        if !server_js.is_file() {
            log::info!(
                "Solidity language server JS file not found at {}",
                server_js.display()
            );
            return None;
        }

        let node_binary = node_runtime::find_working_node_binary(path_env_var).await?;

        Some(CustomBinaryConfig {
            binary_path: node_binary,
            prepend_args: vec![server_js.to_string_lossy().to_string()],
        })
    }
}

#[cfg(feature = "local_fs")]
fn install_dir() -> PathBuf {
    warp_core::paths::data_dir().join(SERVER_NAME)
}

#[cfg(feature = "local_fs")]
fn is_solidity_project_marker(path: &Path) -> bool {
    [
        "hardhat.config.js",
        "hardhat.config.ts",
        "foundry.toml",
        "truffle-config.js",
        "truffle.js",
        "remappings.txt",
    ]
    .iter()
    .any(|marker| path.join(marker).exists())
}

#[cfg(feature = "local_fs")]
fn contains_solidity_file(path: &Path, max_depth: usize) -> bool {
    if max_depth == 0 {
        return false;
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };

    for entry in entries.flatten() {
        let file_path = entry.path();
        if file_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, ".git" | "node_modules" | "target" | "dist"))
        {
            continue;
        }

        if file_path.is_file()
            && file_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "sol")
        {
            return true;
        }

        if file_path.is_dir() && contains_solidity_file(&file_path, max_depth - 1) {
            return true;
        }
    }

    false
}

#[cfg(feature = "local_fs")]
fn binary_exists_on_path(binary_name: &str, path_env_var: Option<&str>) -> bool {
    let path = path_env_var
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"));

    let Some(path) = path else {
        return false;
    };

    let candidates: Vec<String> = if cfg!(windows) {
        let mut candidates = vec![format!("{binary_name}.cmd"), format!("{binary_name}.bat")];
        if binary_name.ends_with(".exe") {
            candidates.push(binary_name.to_string());
        } else {
            candidates.push(format!("{binary_name}.exe"));
        }
        candidates
    } else {
        vec![binary_name.to_string()]
    };

    std::env::split_paths(&path).any(|dir| {
        candidates.iter().any(|candidate| {
            let path = dir.join(candidate);
            path.is_file()
        })
    })
}

#[async_trait]
#[cfg(feature = "local_fs")]
impl LanguageServerCandidate for SolidityLanguageServerCandidate {
    async fn should_suggest_for_repo(&self, path: &Path, _executor: &CommandBuilder) -> bool {
        is_solidity_project_marker(path) || contains_solidity_file(path, 4)
    }

    async fn is_installed_in_data_dir(&self, executor: &CommandBuilder) -> bool {
        Self::find_installed_binary_config(executor.path_env_var())
            .await
            .is_some()
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        binary_exists_on_path(SERVER_NAME, executor.path_env_var())
    }

    async fn install(
        &self,
        metadata: LanguageServerMetadata,
        executor: &CommandBuilder,
    ) -> anyhow::Result<()> {
        log::info!(
            "Installing {} version {}",
            NPM_PACKAGE_NAME,
            metadata.version
        );

        let install_dir = install_dir();

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

        let mut cmd = if let Some((node_path, npm_path)) = &custom_node_paths {
            let mut c = executor.command(node_path);
            c.arg(npm_path);
            c
        } else {
            executor.command("npm")
        };

        cmd.arg("install")
            .arg("--ignore-scripts")
            .arg(format!("{}@{}", NPM_PACKAGE_NAME, metadata.version))
            .current_dir(&install_dir);

        let output = cmd.output().await.context("Failed to run npm install")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Failed to install Solidity language server via npm: {}",
                stderr
            );
        }

        let server_js = install_dir.join(Self::SERVER_JS_PATH);
        if !server_js.is_file() {
            anyhow::bail!(
                "Solidity language server installed but JS entrypoint was not found at {}",
                server_js.display()
            );
        }

        log::info!("Solidity language server installed successfully");
        Ok(())
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        let version = node_runtime::fetch_npm_package_version(&self.client, NPM_PACKAGE_NAME)
            .await
            .context("Failed to fetch Solidity language server version from npm registry")?;

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
