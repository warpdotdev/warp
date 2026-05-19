use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use blocking::unblock;
use flate2::read::GzDecoder;
use futures::AsyncWriteExt as _;
use futures::TryStreamExt as _;
use http_client::StatusCode;

use remote_server::setup::RemotePlatform;
use remote_server::transport::Error;

const REMOTE_SERVER_TARBALL_CACHE_FILE_NAME: &str = "oz.tar.gz";

const REMOTE_SERVER_TARBALL_DOWNLOAD_ATTEMPTS: usize = 3;
// The local SCP fallback download can run over slow or captive networks. Match
// the install-script timeout so slow client-side downloads have the same budget
// as remote-host downloads.
const REMOTE_SERVER_TARBALL_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

// Keep retry backoff short because retries only cover transient HTTP failures;
// the longer timeout above handles slow successful downloads.
const REMOTE_SERVER_TARBALL_DOWNLOAD_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Exit codes where SCP fallback would not help because the failure is on the
/// remote host itself, not a network/download issue.
pub(super) fn should_try_install(error: &Error) -> bool {
    !matches!(error, Error::ScriptFailed { exit_code, .. } if *exit_code == 2)
}

/// Installs the remote server via SCP fallback.
///
/// The tarball is downloaded or reused from the local cache first, then uploaded
/// to the remote host and passed to the install script as an already-downloaded
/// archive. This avoids requiring the remote host to download the tarball itself.
pub(super) async fn install(socket_path: &Path) -> Result<(), Error> {
    let platform = super::super::detect_remote_platform(socket_path).await?;

    let client_tarball_path = cached_remote_server_tarball(&platform)
        .await
        .map_err(|source| Error::ClientDownloadFailed { source })?;
    let timeout = remote_server::setup::SCP_INSTALL_TIMEOUT;
    let install_dir = remote_server::setup::remote_server_dir();
    let remote_tarball_name = format!("oz-upload-{}.tar.gz", uuid::Uuid::new_v4());
    let remote_tarball_path = format!("{install_dir}/{remote_tarball_name}");

    // The normal install script creates this directory before downloading, but
    // SCP fallback can run after a failure that happened before that point.
    // Ensure the destination exists before uploading the staged tarball.
    let mkdir_output = remote_server::ssh::run_ssh_command(
        socket_path,
        &format!("mkdir -p {install_dir}"),
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await
    .map_err(Error::from)?;
    if !mkdir_output.status.success() {
        let code = mkdir_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr).to_string();
        return Err(Error::ScriptFailed {
            exit_code: code,
            stderr,
        });
    }

    log::info!("Uploading tarball to remote at {remote_tarball_path}");
    remote_server::ssh::scp_upload(
        socket_path,
        &client_tarball_path,
        &remote_tarball_path,
        timeout,
    )
    .await
    .map_err(Error::Other)?;

    log::info!("Running extraction via install script with tarball at {remote_tarball_path}");
    let script = remote_server::setup::install_script(Some(&remote_tarball_path));

    let output = remote_server::ssh::run_ssh_script(socket_path, &script, timeout)
        .await
        .map_err(Error::from)?;
    if output.status.success() {
        Ok(())
    } else {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(Error::ScriptFailed {
            exit_code: code,
            stderr,
        })
    }
}

fn remote_server_tarball_cache_root() -> PathBuf {
    warp_core::paths::cache_dir()
        .join("remote-server")
        .join("tarballs")
}

fn current_remote_server_tarball_cache_version() -> &'static str {
    remote_server::setup::remote_server_artifact_version()
}

fn remote_server_tarball_cache_path(platform: &RemotePlatform) -> PathBuf {
    remote_server_tarball_cache_root()
        .join(current_remote_server_tarball_cache_version())
        .join(format!(
            "{}-{}",
            platform.os.as_str(),
            platform.arch.as_str()
        ))
        .join(REMOTE_SERVER_TARBALL_CACHE_FILE_NAME)
}

async fn is_valid_cached_tarball(path: &Path) -> bool {
    let metadata_is_valid = async_fs::metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
    if !metadata_is_valid {
        return false;
    }

    let path = path.to_path_buf();
    unblock(move || validate_gzip_tarball(&path)).await.is_ok()
}

fn validate_gzip_tarball(path: &Path) -> anyhow::Result<()> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open remote-server tarball '{}'", path.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .with_context(|| format!("Failed to read tar entries from '{}'", path.display()))?;
    let mut entry_count = 0;

    for entry in entries {
        let mut entry =
            entry.with_context(|| format!("Failed to read tar entry from '{}'", path.display()))?;
        std::io::copy(&mut entry, &mut std::io::sink())
            .with_context(|| format!("Failed to validate tar entry from '{}'", path.display()))?;
        entry_count += 1;
    }

    anyhow::ensure!(
        entry_count > 0,
        "Remote-server tarball '{}' contained no entries",
        path.display()
    );

    Ok(())
}

/// Returns a local tarball for the remote platform.
///
/// Reuses an existing cached tarball when available; otherwise downloads the
/// tarball into the cache and returns the newly cached path.
async fn cached_remote_server_tarball(platform: &RemotePlatform) -> anyhow::Result<PathBuf> {
    let cache_path = remote_server_tarball_cache_path(platform);
    let url = remote_server::setup::download_tarball_url(platform);
    cached_remote_server_tarball_from(&url, &cache_path).await
}

async fn cached_remote_server_tarball_from(
    url: &str,
    cache_path: &Path,
) -> anyhow::Result<PathBuf> {
    if is_valid_cached_tarball(cache_path).await {
        log::info!(
            "Using cached remote-server tarball at {}",
            cache_path.display()
        );
        return Ok(cache_path.to_path_buf());
    }

    if async_fs::metadata(cache_path).await.is_ok() {
        log::warn!(
            "Discarding invalid cached remote-server tarball at {}",
            cache_path.display()
        );
        let _ = async_fs::remove_file(cache_path).await;
    }

    log::info!(
        "Downloading remote-server tarball from {url} into cache at {}",
        cache_path.display()
    );
    download_remote_server_tarball_to_cache(url, cache_path).await?;
    Ok(cache_path.to_path_buf())
}

async fn download_remote_server_tarball_to_cache(
    url: &str,
    cache_path: &Path,
) -> anyhow::Result<()> {
    let parent = cache_path
        .parent()
        .context("remote-server tarball cache path has no parent directory")?;
    async_fs::create_dir_all(parent).await.with_context(|| {
        format!(
            "Failed to create remote-server tarball cache directory '{}'",
            parent.display()
        )
    })?;
    let temp_dir = parent.join(".tmp");
    async_fs::create_dir_all(&temp_dir).await.with_context(|| {
        format!(
            "Failed to create remote-server tarball cache temp directory '{}'",
            temp_dir.display()
        )
    })?;

    let http_client = http_client::Client::new();
    let mut last_retryable_error = None;

    for attempt in 1..=REMOTE_SERVER_TARBALL_DOWNLOAD_ATTEMPTS {
        // Download into a fresh unique temp path for every attempt so a failed
        // or partial response body can never be reused by a later retry.
        let temp_path = temp_dir.join(format!(
            ".{REMOTE_SERVER_TARBALL_CACHE_FILE_NAME}.{}.tmp",
            uuid::Uuid::new_v4()
        ));

        let attempt_result =
            download_remote_server_tarball_attempt(&http_client, url, cache_path, &temp_path).await;
        if attempt_result.is_err() {
            let _ = async_fs::remove_file(&temp_path).await;
        }

        match attempt_result {
            Ok(()) => return Ok(()),
            Err(DownloadAttemptError::Permanent(e)) => return Err(e),
            Err(DownloadAttemptError::Retryable(e)) => {
                last_retryable_error = Some(e);
                if attempt < REMOTE_SERVER_TARBALL_DOWNLOAD_ATTEMPTS {
                    log::warn!("Remote-server tarball download attempt {attempt} failed; retrying");
                    tokio::time::sleep(REMOTE_SERVER_TARBALL_DOWNLOAD_RETRY_DELAY).await;
                }
            }
        }
    }

    Err(last_retryable_error
        .unwrap_or_else(|| anyhow::anyhow!("Remote-server tarball download failed without an error"))
        .context(format!(
            "Remote-server tarball client download failed after {REMOTE_SERVER_TARBALL_DOWNLOAD_ATTEMPTS} attempts"
        )))
}

async fn download_remote_server_tarball_attempt(
    http_client: &http_client::Client,
    url: &str,
    cache_path: &Path,
    temp_path: &Path,
) -> Result<(), DownloadAttemptError> {
    download_remote_server_tarball_internal(http_client, url, temp_path).await?;
    if !is_valid_cached_tarball(temp_path).await {
        return Err(DownloadAttemptError::Retryable(anyhow::anyhow!(
            "Downloaded remote-server tarball from {url} was not a valid gzip/tar archive"
        )));
    }

    publish_remote_server_tarball_cache(temp_path, cache_path).await
}

enum DownloadAttemptError {
    Retryable(anyhow::Error),
    Permanent(anyhow::Error),
}
async fn publish_remote_server_tarball_cache(
    temp_path: &Path,
    cache_path: &Path,
) -> Result<(), DownloadAttemptError> {
    if is_valid_cached_tarball(cache_path).await {
        let _ = async_fs::remove_file(temp_path).await;
        return Ok(());
    }

    // Publish the validated temp file to the shared cache path. If another
    // concurrent fallback populated the cache after the check above, that valid
    // cache hit is good enough for this install, so discard our temp file.
    match async_fs::rename(temp_path, cache_path).await {
        Ok(()) => Ok(()),
        Err(e) if is_valid_cached_tarball(cache_path).await => {
            let _ = async_fs::remove_file(temp_path).await;
            Ok(())
        }
        Err(e) => {
            let _ = async_fs::remove_file(temp_path).await;
            Err(DownloadAttemptError::Permanent(anyhow::anyhow!(
                "Failed to move remote-server tarball into cache at '{}': {e}",
                cache_path.display()
            )))
        }
    }
}

async fn download_remote_server_tarball_internal(
    http_client: &http_client::Client,
    url: &str,
    temp_path: &Path,
) -> Result<(), DownloadAttemptError> {
    let response = http_client
        .get(url)
        .timeout(REMOTE_SERVER_TARBALL_DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            DownloadAttemptError::Retryable(anyhow::anyhow!(
                "Failed to download remote-server tarball from {url}: {e}"
            ))
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let error =
            anyhow::anyhow!("Remote-server tarball download failed with status {status}: {body}");
        return if is_retryable_download_status(status) {
            Err(DownloadAttemptError::Retryable(error))
        } else {
            Err(DownloadAttemptError::Permanent(error))
        };
    }

    let mut file = async_fs::File::create(temp_path).await.map_err(|e| {
        DownloadAttemptError::Permanent(anyhow::anyhow!(
            "Failed to create remote-server tarball cache file '{}': {e}",
            temp_path.display()
        ))
    })?;
    let mut bytes_stream = response.bytes_stream();
    while let Some(chunk) = bytes_stream.try_next().await.map_err(|e| {
        DownloadAttemptError::Retryable(anyhow::anyhow!(
            "Failed to read remote-server tarball response body from {url}: {e}"
        ))
    })? {
        file.write_all(&chunk).await.map_err(|e| {
            DownloadAttemptError::Permanent(anyhow::anyhow!(
                "Failed to write remote-server tarball cache file '{}': {e}",
                temp_path.display()
            ))
        })?;
    }
    file.sync_data().await.map_err(|e| {
        DownloadAttemptError::Permanent(anyhow::anyhow!(
            "Failed to sync remote-server tarball cache file '{}': {e}",
            temp_path.display()
        ))
    })?;

    Ok(())
}

fn is_retryable_download_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
}

#[cfg(test)]
#[path = "scp_fallback_tests.rs"]
mod tests;
