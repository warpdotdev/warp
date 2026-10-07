use std::path::Path;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
#[cfg(feature = "local_fs")]
use command::r#async::Command;

use crate::CommandBuilder;
#[cfg(feature = "local_fs")]
use crate::install::{
    AssetKind, fetch_latest_metadata_from_github_dynamic_asset, install_from_github,
};
use crate::language_server_candidate::{LanguageServerCandidate, LanguageServerMetadata};

#[cfg(feature = "local_fs")]
const SERVER_NAME: &str = "clangd";

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub struct ClangdCandidate {
    client: Arc<http_client::Client>,
}

impl ClangdCandidate {
    pub fn new(client: Arc<http_client::Client>) -> Self {
        Self { client }
    }

    #[cfg(feature = "local_fs")]
    pub async fn find_installed_binary_in_data_dir() -> Option<PathBuf> {
        let install_root = warp_core::paths::data_dir().join(SERVER_NAME);
        if !install_root.is_dir() {
            return None;
        }

        let Ok(entries) = std::fs::read_dir(&install_root) else {
            return None;
        };

        for entry in entries.flatten() {
            let version_dir = entry.path();
            if !version_dir.is_dir() {
                continue;
            }

            let Some(binary_path) = find_binary_in_dir(&version_dir) else {
                continue;
            };

            if binary_is_working(&binary_path).await {
                return Some(binary_path);
            }
        }

        None
    }
}

#[cfg(feature = "local_fs")]
fn asset_os_suffix() -> anyhow::Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => Ok("mac"),
        ("linux", "x86_64") => Ok("linux"),
        ("windows", "x86_64") => Ok("windows"),
        (os, arch) => anyhow::bail!("Unsupported platform for clangd: {os}/{arch}"),
    }
}

#[cfg(feature = "local_fs")]
fn is_c_or_cpp_extension(extension: &str) -> bool {
    matches!(
        extension,
        "c" | "C" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx" | "H"
    )
}

#[cfg(feature = "local_fs")]
async fn binary_is_working(binary_path: &Path) -> bool {
    let mut command = Command::new(binary_path);
    command.arg("--version");
    command
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(feature = "local_fs")]
fn is_bin_clangd_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    let expected_name = if cfg!(windows) {
        "clangd.exe"
    } else {
        "clangd"
    };

    if file_name != expected_name {
        return false;
    }

    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("bin")
}

#[cfg(feature = "local_fs")]
fn find_binary_in_dir(root: &Path) -> Option<PathBuf> {
    let mut directories = vec![root.to_path_buf()];

    while let Some(dir) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
                continue;
            }

            if is_bin_clangd_path(&path) {
                return Some(path);
            }
        }
    }

    None
}

#[async_trait]
#[cfg(feature = "local_fs")]
impl LanguageServerCandidate for ClangdCandidate {
    async fn should_suggest_for_repo(&self, path: &Path, _executor: &CommandBuilder) -> bool {
        let repo_markers = [
            "compile_commands.json",
            "compile_flags.txt",
            ".clangd",
            "CMakeLists.txt",
        ];

        if repo_markers.iter().any(|marker| path.join(marker).exists()) {
            return true;
        }

        let Ok(entries) = std::fs::read_dir(path) else {
            return false;
        };

        entries.flatten().any(|entry| {
            let file_path = entry.path();
            file_path.is_file()
                && file_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(is_c_or_cpp_extension)
        })
    }

    async fn is_installed_in_data_dir(&self, _executor: &CommandBuilder) -> bool {
        Self::find_installed_binary_in_data_dir().await.is_some()
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        executor
            .command(SERVER_NAME)
            .arg("--version")
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    async fn install(
        &self,
        metadata: LanguageServerMetadata,
        _executor: &CommandBuilder,
    ) -> anyhow::Result<()> {
        let binary_path = install_from_github(
            &self.client,
            &metadata,
            SERVER_NAME,
            AssetKind::Zip,
            Some(find_binary_in_dir),
        )
        .await?;

        // Verify the installed binary works
        if !binary_is_working(&binary_path).await {
            anyhow::bail!(
                "Installed clangd binary at {} failed version check",
                binary_path.display()
            );
        }

        Ok(())
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        let os_suffix = asset_os_suffix()?;

        fetch_latest_metadata_from_github_dynamic_asset(
            &self.client,
            "clangd",
            "clangd",
            move |tag| format!("clangd-{os_suffix}-{tag}.zip"),
        )
        .await
    }
}

#[async_trait]
#[cfg(not(feature = "local_fs"))]
impl LanguageServerCandidate for ClangdCandidate {
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
