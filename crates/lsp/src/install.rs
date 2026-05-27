use anyhow::{Context, Result};
#[cfg(all(feature = "local_fs", unix))]
use async_fs::unix::PermissionsExt;
use serde::Deserialize;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use sha2::{Digest, Sha256};
        use std::path::PathBuf;
    }
}

use crate::language_server_candidate::LanguageServerMetadata;

const GITHUB_API_URL: &str = "https://api.github.com";

#[derive(Deserialize, Debug)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Deserialize, Debug)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

/// Finds a named asset in a release and returns its download URL and optional SHA256 digest.
fn resolve_asset(
    assets: Vec<GithubReleaseAsset>,
    asset_name: &str,
) -> Result<(String, Option<String>)> {
    let asset = assets
        .into_iter()
        .find(|a| a.name == asset_name)
        .with_context(|| format!("Asset '{asset_name}' not found in release"))?;

    // Strip the "sha256:" prefix from the digest if present
    let digest = asset
        .digest
        .map(|d| d.strip_prefix("sha256:").unwrap_or(&d).to_string());

    Ok((asset.browser_download_url, digest))
}

async fn fetch_latest_release_from_github(
    client: &http_client::Client,
    repo_owner: &str,
    repo_name: &str,
) -> Result<GithubRelease> {
    let url = format!(
        "{}/repos/{}/{}/releases/latest",
        GITHUB_API_URL, repo_owner, repo_name
    );

    let response = client
        .get(&url)
        // GitHub API recommends specifying these parameters in the header.
        // See: https://docs.github.com/en/rest/using-the-rest-api/getting-started-with-the-rest-api#user-agent
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "warp-terminal")
        .send()
        .await
        .context("Failed to fetch latest release from GitHub")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "GitHub API returned status {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        );
    }

    response
        .json()
        .await
        .context("Failed to parse GitHub release response")
}

/// Fetches the latest release metadata from a GitHub repository.
///
/// # Arguments
/// * `client` - The HTTP client to use for the request
/// * `repo_owner` - The owner of the GitHub repository (e.g. "rust-lang")
/// * `repo_name` - The name of the GitHub repository (e.g. "rust-analyzer")
/// * `asset_name` - The name of the asset to find in the release. If None, no asset
///   lookup is performed and url/digest will be None (useful for servers like gopls
///   that don't provide prebuilt binaries).
///
/// # Returns
/// A `LanguageServerMetadata` containing the version, optional download URL, and optional SHA256 digest.
pub async fn fetch_latest_metadata_from_github(
    client: &http_client::Client,
    repo_owner: &str,
    repo_name: &str,
    asset_name: Option<&str>,
) -> Result<LanguageServerMetadata> {
    let release = fetch_latest_release_from_github(client, repo_owner, repo_name).await?;

    // If an asset name is provided, look up the asset details
    let (download_url, digest) = if let Some(asset_name) = asset_name {
        let (url, digest) = resolve_asset(release.assets, asset_name)?;
        (Some(url), digest)
    } else {
        (None, None)
    };

    Ok(LanguageServerMetadata {
        version: release.tag_name,
        url: download_url,
        digest,
    })
}

/// Fetches the latest release metadata from a GitHub repository and resolves
/// an asset name dynamically based on the latest release tag.
///
/// This is useful for projects where asset names include the tag itself, e.g.
/// `clangd-mac-v21.0.0.zip`.
pub async fn fetch_latest_metadata_from_github_dynamic_asset<F>(
    client: &http_client::Client,
    repo_owner: &str,
    repo_name: &str,
    asset_name_for_tag: F,
) -> Result<LanguageServerMetadata>
where
    F: FnOnce(&str) -> String,
{
    let release = fetch_latest_release_from_github(client, repo_owner, repo_name).await?;
    let asset_name = asset_name_for_tag(&release.tag_name);
    let (url, digest) = resolve_asset(release.assets, &asset_name)?;

    Ok(LanguageServerMetadata {
        version: release.tag_name,
        url: Some(url),
        digest,
    })
}

/// The type of archive for a GitHub release asset.
#[cfg(feature = "local_fs")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// A gzip-compressed file (e.g., `rust-analyzer-aarch64-apple-darwin.gz`)
    Gz,
    /// A zip archive (e.g., `rust-analyzer-x86_64-pc-windows-msvc.zip`)
    Zip,
}

#[cfg(feature = "local_fs")]
impl AssetKind {
    /// Determines the asset kind from a file name based on its extension.
    pub fn from_filename(filename: &str) -> Option<Self> {
        if filename.ends_with(".gz") && !filename.ends_with(".tar.gz") {
            Some(AssetKind::Gz)
        } else if filename.ends_with(".zip") {
            Some(AssetKind::Zip)
        } else {
            None
        }
    }
}

/// Downloads and installs a language server binary from GitHub.
///
/// This function:
/// 1. Downloads the binary from the provided URL
/// 2. Verifies the SHA256 checksum if provided
/// 3. Extracts/decompresses the archive based on its type
/// 4. Makes the binary executable (on Unix systems)
///
/// # Arguments
/// * `client` - The HTTP client to use for downloading
/// * `metadata` - The server metadata containing version, URL, and optional digest
/// * `server_name` - The name of the server (e.g., "rust-analyzer") used for the destination path
/// * `asset_kind` - The type of archive (Gz or Zip)
/// * `binary_finder` - Optional callback to locate the binary after extraction. When provided,
///   the full archive is extracted (no filter) and this function is called to find the binary.
///   When `None`, a name-based filter is used during extraction and the binary is assumed to be
///   at `{install_dir}/{server_name}`.
///
/// # Returns
/// The path to the installed binary on success.
#[cfg(feature = "local_fs")]
pub async fn install_from_github(
    client: &http_client::Client,
    metadata: &LanguageServerMetadata,
    server_name: &str,
    asset_kind: AssetKind,
    binary_finder: Option<fn(&std::path::Path) -> Option<PathBuf>>,
) -> Result<PathBuf> {
    let url = metadata
        .url
        .as_ref()
        .context("No download URL provided in metadata")?;

    // Create the destination directory: {data_dir}/{server_name}/{version}
    // If it already exists, remove it first to ensure a clean installation
    let install_dir = warp_core::paths::data_dir()
        .join(server_name)
        .join(&metadata.version);
    if install_dir.exists() {
        async_fs::remove_dir_all(&install_dir)
            .await
            .with_context(|| {
                format!(
                    "Failed to remove existing install directory: {:?}",
                    install_dir
                )
            })?;
    }
    async_fs::create_dir_all(&install_dir)
        .await
        .with_context(|| format!("Failed to create install directory: {:?}", install_dir))?;

    // The binary name is the server name (with .exe on Windows)
    let binary_name = if cfg!(windows) {
        format!("{server_name}.exe")
    } else {
        server_name.to_string()
    };

    // Download the file
    log::info!("Downloading {server_name} from {url}");
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to download from {url}"))?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Download failed with status {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        );
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    // Verify checksum if provided
    if let Some(expected_digest) = &metadata.digest {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual_digest = format!("{:x}", hasher.finalize());

        if actual_digest != *expected_digest {
            anyhow::bail!(
                "SHA256 checksum mismatch for {server_name}. Expected: {expected_digest}, Got: {actual_digest}"
            );
        }
        log::info!("Checksum verified for {server_name}");
    }

    // Extract the archive based on type
    match asset_kind {
        AssetKind::Gz => {
            let binary_path = install_dir.join(&binary_name);
            node_runtime::extract_gz(&bytes, &binary_path).await?;
        }
        AssetKind::Zip => {
            if binary_finder.is_some() {
                // Extract the full archive when using a custom binary finder
                let no_filter: Option<fn(&str) -> bool> = None;
                node_runtime::extract_zip(&bytes, &install_dir, no_filter).await?;
            } else {
                // Use a filter to extract only the specific binary we need
                let binary_name_clone = binary_name.clone();
                node_runtime::extract_zip(
                    &bytes,
                    &install_dir,
                    Some(move |file_name: &str| {
                        file_name.ends_with(&binary_name_clone)
                            || file_name.ends_with(&format!("/{binary_name_clone}"))
                    }),
                )
                .await?;
            }
        }
    }

    // Locate the binary
    let binary_path = if let Some(finder) = binary_finder {
        finder(&install_dir)
            .with_context(|| format!("Failed to locate {server_name} binary after extraction"))?
    } else {
        install_dir.join(&binary_name)
    };

    // Make the binary executable on Unix systems
    #[cfg(unix)]
    {
        let mut perms = async_fs::metadata(&binary_path)
            .await
            .with_context(|| format!("Failed to get metadata for {:?}", binary_path))?
            .permissions();
        perms.set_mode(0o755);
        async_fs::set_permissions(&binary_path, perms)
            .await
            .with_context(|| format!("Failed to set permissions for {:?}", binary_path))?;
    }

    log::info!("Successfully installed {server_name} to {:?}", binary_path);
    Ok(binary_path)
}
