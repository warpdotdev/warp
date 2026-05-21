use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "local_fs")]
use crate::install::{fetch_latest_metadata_from_github, install_from_github, AssetKind};
use crate::language_server_candidate::{LanguageServerCandidate, LanguageServerMetadata};
use crate::CommandBuilder;
use async_trait::async_trait;

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub struct RustAnalyzerCandidate {
    client: Arc<http_client::Client>,
}

/// Returns the rust-analyzer asset name for the current platform.
///
/// Asset names follow the pattern: rust-analyzer-{arch}-{vendor}-{os}.{ext}
/// e.g. rust-analyzer-aarch64-apple-darwin.gz, rust-analyzer-x86_64-unknown-linux-gnu.gz
#[cfg(feature = "local_fs")]
fn asset_name() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "rust-analyzer-aarch64-apple-darwin.gz"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "rust-analyzer-x86_64-apple-darwin.gz"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "rust-analyzer-x86_64-unknown-linux-gnu.gz"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "rust-analyzer-aarch64-unknown-linux-gnu.gz"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "rust-analyzer-x86_64-pc-windows-msvc.zip"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "rust-analyzer-aarch64-pc-windows-msvc.zip"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
    )))]
    {
        todo!("Unsupported platform for rust-analyzer")
    }
}

#[cfg(feature = "local_fs")]
const SERVER_NAME: &str = "rust-analyzer";

impl RustAnalyzerCandidate {
    pub fn new(client: Arc<http_client::Client>) -> Self {
        Self { client }
    }

    /// Finds the path to an installed rust-analyzer binary in the data directory.
    ///
    /// Returns the path to the first working binary found (verified by running `--help`).
    #[cfg(feature = "local_fs")]
    pub async fn find_installed_binary_in_data_dir() -> Option<std::path::PathBuf> {
        use tokio::process::Command;

        let install_dir = warp_core::paths::data_dir().join(SERVER_NAME);
        if !install_dir.exists() {
            return None;
        }

        // Check if any version directory contains a working binary
        let binary_name = if cfg!(windows) {
            format!("{}.exe", SERVER_NAME)
        } else {
            SERVER_NAME.to_string()
        };

        let Ok(entries) = std::fs::read_dir(&install_dir) else {
            return None;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let binary_path = path.join(&binary_name);
                if binary_path.is_file() {
                    // Verify the binary works by running --help
                    let mut cmd = Command::new(&binary_path);
                    cmd.arg("--help");
                    if cmd
                        .output()
                        .await
                        .map(|output| output.status.success())
                        .unwrap_or(false)
                    {
                        return Some(binary_path);
                    }
                }
            }
        }

        None
    }
}

#[async_trait]
#[cfg(feature = "local_fs")]
impl LanguageServerCandidate for RustAnalyzerCandidate {
    async fn should_suggest_for_repo(&self, path: &Path, _executor: &CommandBuilder) -> bool {
        path.join("Cargo.toml").exists()
    }

    async fn is_installed_in_data_dir(&self, _executor: &CommandBuilder) -> bool {
        Self::find_installed_binary_in_data_dir().await.is_some()
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        executor
            .command(SERVER_NAME)
            .arg("--help")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn install(
        &self,
        metadata: LanguageServerMetadata,
        _executor: &CommandBuilder,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !cfg!(target_os = "freebsd"),
            "rust-analyzer is not auto-installable on FreeBSD: upstream \
             GitHub releases publish no FreeBSD asset. Install it via \
             `rustup component add rust-analyzer` or `pkg install \
             rust-analyzer` and warp will pick it up off PATH."
        );
        let asset_kind = AssetKind::from_filename(asset_name()).ok_or_else(|| {
            anyhow::anyhow!("Unsupported archive format for asset: {}", asset_name())
        })?;
        install_from_github(&self.client, &metadata, SERVER_NAME, asset_kind, None).await?;
        Ok(())
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        anyhow::ensure!(
            !cfg!(target_os = "freebsd"),
            "rust-analyzer release metadata is unavailable on FreeBSD: \
             upstream GitHub releases publish no FreeBSD asset."
        );
        fetch_latest_metadata_from_github(
            &self.client,
            "rust-lang",
            "rust-analyzer",
            Some(asset_name()),
        )
        .await
    }
}

#[async_trait]
#[cfg(not(feature = "local_fs"))]
impl LanguageServerCandidate for RustAnalyzerCandidate {
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
