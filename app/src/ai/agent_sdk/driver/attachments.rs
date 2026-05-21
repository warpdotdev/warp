use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use base64::engine::general_purpose;
use base64::Engine;
use futures::future::join_all;
use futures::TryStreamExt as _;
use mime_guess::from_path;
use tokio::fs;
use tokio_util::io::StreamReader;
use warp_core::features::FeatureFlag;

use crate::ai::agent_sdk::retry::with_bounded_retry;
use crate::ai::ambient_agents::task::{AttachmentInput, TaskAttachment};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::attachment_utils::MAX_ATTACHMENT_SIZE_BYTES;
use crate::server::server_api::ai::AIClient;
use crate::server::server_api::presigned_upload::HttpStatusError;
use crate::server::server_api::ServerApi;
use crate::util::image::MIN_IMAGE_HEADER_SIZE;

/// Maximum number of file attachments for a cloud agent task.
pub const MAX_ATTACHMENT_COUNT_FOR_CLOUD_QUERY: usize = 25;

/// Fetches task attachments via GraphQL and downloads them to the filesystem.
/// Returns the attachments directory path if any attachments were downloaded,
/// so the caller can pass it to the server via `StartFromAmbientRunPrompt`.
///
/// `attachments_dir` is the per-session directory where files should be downloaded.
///
/// Makes a best-effort attempt to download all attachments.
/// Individual download failures are logged but don't cause the entire function to fail.
pub(crate) async fn fetch_and_download_attachments(
    ai_client: Arc<dyn AIClient>,
    http_client: Arc<ServerApi>,
    task_id: String,
    attachments_dir: PathBuf,
) -> anyhow::Result<Option<String>> {
    if !FeatureFlag::AmbientAgentsImageUpload.is_enabled() {
        return Ok(None);
    }

    let attachments = ai_client
        .get_task_attachments(task_id.clone())
        .await
        .context("Failed to fetch task attachments")?;

    log::info!("Fetched {} task attachments", attachments.len());

    if attachments.is_empty() {
        return Ok(None);
    }

    download_and_write_attachments(attachments, &attachments_dir, &http_client).await?;

    Ok(Some(attachments_dir.to_string_lossy().into_owned()))
}

/// Fetches handoff snapshot attachments for the active execution and downloads
/// them into `{attachments_dir}/handoff/{attachment_uuid}` so the runtime's
/// rehydration prompt references always point at a file that exists on disk.
///
/// Returns `Some(attachments_dir)` when at least one attachment wrote to disk, mirroring
/// the contract of the sibling [`fetch_and_download_attachments`]. Partial failures are
/// logged at WARN level inside this function; per-file errors are not surfaced to callers.
///
/// Fatal failures (listing the attachments, creating the handoff dir) return `Err`.
pub(crate) async fn fetch_and_download_handoff_snapshot_attachments(
    ai_client: Arc<dyn AIClient>,
    http_client: &http_client::Client,
    task_id: AmbientAgentTaskId,
    attachments_dir: PathBuf,
) -> anyhow::Result<Option<String>> {
    if !FeatureFlag::OzHandoff.is_enabled() {
        log::error!(
            "fetch_and_download_handoff_snapshot_attachments called with OzHandoff disabled; \
             call sites should gate on the flag before invoking"
        );
        return Ok(None);
    }

    let attachments = ai_client
        .get_handoff_snapshot_attachments(&task_id)
        .await
        .context("Failed to fetch handoff snapshot attachments")?;

    if attachments.is_empty() {
        return Ok(None);
    }

    let handoff_dir = attachments_dir.join("handoff");
    fs::create_dir_all(&handoff_dir)
        .await
        .context("Failed to create handoff attachments directory")?;

    let attempts = attachments.len();
    let download_futures = attachments.into_iter().map(|attachment| {
        let file_path = handoff_dir.join(&attachment.file_id);
        download_handoff_entry(attachment, file_path, http_client)
    });
    let results = join_all(download_futures).await;

    let mut succeeded: usize = 0;
    let mut failures: Vec<(String, String)> = Vec::new();
    for result in results {
        match result {
            Ok(()) => succeeded += 1,
            Err((filename, err)) => failures.push((filename, err)),
        }
    }

    if failures.is_empty() {
        log::info!("Handoff snapshot attachments: {succeeded}/{attempts} downloaded");
    } else {
        let detail = failures
            .iter()
            .map(|(filename, err)| format!("{filename}: {err}"))
            .collect::<Vec<_>>()
            .join("; ");
        log::warn!(
            "Handoff snapshot attachments: {succeeded}/{attempts} downloaded; {} failed ({detail})",
            failures.len()
        );
    }

    // Only surface the attachments dir if at least one file made it to disk. Passing a dir
    // with zero usable entries downstream would make the rehydration prompt reference a
    // phantom path.
    if succeeded == 0 {
        return Ok(None);
    }
    Ok(Some(attachments_dir.to_string_lossy().into_owned()))
}

/// Downloads task attachments from presigned URLs and writes them to the filesystem.
/// Downloads are performed concurrently using `join_all`.
/// Makes a best-effort attempt to download all attachments, logging warnings for failures.
/// The filename is already formatted by the server with UUID prefix (e.g., "uuid_filename.png").
async fn download_and_write_attachments(
    attachments: Vec<TaskAttachment>,
    attachment_dir: &Path,
    http_client: &ServerApi,
) -> anyhow::Result<()> {
    fs::create_dir_all(attachment_dir)
        .await
        .context("Failed to create attachments directory")?;
    log::info!(
        "Created attachments directory at: {}",
        attachment_dir.display()
    );

    let http = http_client.http_client();
    let download_futures = attachments
        .into_iter()
        .map(|attachment| download_task_attachment(attachment, attachment_dir, http));
    let results = join_all(download_futures).await;

    let mut successful = 0;
    let mut failed = 0;
    for result in results {
        match result {
            Ok(()) => successful += 1,
            Err(_) => failed += 1,
        }
    }

    log::info!("Attachment download complete: {successful} successful, {failed} failed");

    Ok(())
}

/// Download a single task attachment into `attachment_dir/<sanitized filename>`.
///
/// Delegates to [`download_attachment`] so transient failures retry on the shared schedule.
async fn download_task_attachment(
    attachment: TaskAttachment,
    attachment_dir: &Path,
    http_client: &http_client::Client,
) -> anyhow::Result<()> {
    let safe_filename = Path::new(&attachment.filename)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid filename for file_id={}", attachment.file_id))?
        .to_string();

    let file_path = attachment_dir.join(&safe_filename);
    log::info!(
        "Downloading attachment: {} -> {}",
        attachment.filename,
        file_path.display()
    );

    download_attachment(http_client, &attachment.download_url, &file_path).await?;

    log::info!("Successfully wrote attachment to: {}", file_path.display());
    Ok(())
}

/// Download a single handoff attachment into `file_path`, mapping failure to
/// `(filename, error_message)` so the aggregator in
/// [`fetch_and_download_handoff_snapshot_attachments`] can log and count per-file outcomes.
async fn download_handoff_entry(
    attachment: TaskAttachment,
    file_path: PathBuf,
    http_client: &http_client::Client,
) -> Result<(), (String, String)> {
    // Factor `file_id` and `download_url` out before the retry closure so `attachment` is fully
    // consumed up-front. The closure borrows the two fields it needs as references.
    let TaskAttachment {
        file_id,
        download_url,
        ..
    } = attachment;
    download_attachment(http_client, &download_url, &file_path)
        .await
        .map_err(|e| (file_id, format!("{e:#}")))
}

/// Shared download primitive: GET `download_url`, write the body to `file_path`, and retry
/// transient HTTP failures on the shared bounded-backoff schedule. Non-2xx responses surface
/// an [`HttpStatusError`] so the retry classifier can decide whether to retry.
async fn download_attachment(
    http_client: &http_client::Client,
    download_url: &str,
    file_path: &Path,
) -> anyhow::Result<()> {
    let operation = format!("download attachment '{}'", file_path.display());
    with_bounded_retry(&operation, || async {
        let response = http_client
            .get(download_url)
            .send()
            .await
            .context("Failed to send download request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::Error::new(HttpStatusError {
                status: status.as_u16(),
                body: body.clone(),
            })
            .context(format!("Download failed with status {status}: {body}")));
        }

        // Stream the response body directly to disk instead of buffering the full payload
        // in memory.
        let mut file = fs::File::create(file_path)
            .await
            .context("Failed to create file")?;
        let mut response_stream =
            StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
        tokio::io::copy(&mut response_stream, &mut file)
            .await
            .context("Failed to write file")?;

        Ok(())
    })
    .await
}

/// Process a file attachment for ambient agent upload.
/// Returns AttachmentInput with base64-encoded data.
/// All file types share the same 10MB size limit.
pub fn process_attachment(
    attachment_path: &PathBuf,
    index: usize,
) -> anyhow::Result<AttachmentInput> {
    let file_bytes = std::fs::read(attachment_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read attachment file '{}': {e}",
            attachment_path.display()
        )
    })?;

    // Detect MIME type from file data using infer crate, fall back to file extension
    let mime_type = if file_bytes.len() >= MIN_IMAGE_HEADER_SIZE {
        infer::get(&file_bytes).map(|kind| kind.mime_type().to_string())
    } else {
        None
    };

    // If infer couldn't detect, fall back to file extension
    let mime_type = mime_type.unwrap_or_else(|| {
        from_path(attachment_path)
            .first_or_octet_stream()
            .to_string()
    });

    if file_bytes.len() > MAX_ATTACHMENT_SIZE_BYTES {
        return Err(anyhow::anyhow!(
            "File is too large ({}MB). Maximum size is 10MB.",
            file_bytes.len() / (1024 * 1024)
        ));
    }

    let base64_data = general_purpose::STANDARD.encode(&file_bytes);

    let file_name = attachment_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("task_attachment_{index}"));

    Ok(AttachmentInput {
        file_name,
        mime_type,
        data: base64_data,
    })
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
