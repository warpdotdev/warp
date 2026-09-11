//! Shared utilities for file-attachment handling (download, filename sanitization,
//! and building attachment maps).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use warp_errors::report_error;

/// Max file attachment size is 10 MB.
pub(crate) const MAX_ATTACHMENT_SIZE_BYTES: usize = 10 * 1024 * 1024;

use crate::ai::agent::AIAgentAttachment;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::AIClient;

/// Returns the per-session directory for downloading file attachments,
/// based on the agent's working directory.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn attachments_download_dir(working_dir: &Path) -> PathBuf {
    working_dir.join(".warp").join("attachments")
}

/// Extracts the filename component from a path, stripping any directory prefixes.
pub(crate) fn sanitize_filename(raw: &str) -> &str {
    Path::new(raw)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(raw)
}

/// A downloaded file attachment with its resolved path on disk.
pub(crate) struct DownloadedAttachment {
    /// The UUID of the attachment.
    pub file_id: String,
    /// The sanitized display name.
    pub file_name: String,
    /// The full resolved path on disk where the file was downloaded.
    pub file_path: String,
}

/// Builds a `HashMap<String, AIAgentAttachment>` keyed by (deduplicated) filename
/// from a list of successfully downloaded attachments.
pub(crate) fn build_file_attachment_map(
    downloads: &[DownloadedAttachment],
) -> HashMap<String, AIAgentAttachment> {
    let mut map = HashMap::new();
    for download in downloads {
        let mut key = download.file_name.clone();
        if map.contains_key(&key) {
            let mut suffix = 1;
            loop {
                key = format!("{} ({suffix})", download.file_name);
                if !map.contains_key(&key) {
                    break;
                }
                suffix += 1;
            }
        }
        map.insert(
            key,
            AIAgentAttachment::FilePathReference {
                file_id: download.file_id.clone(),
                file_name: download.file_name.clone(),
                file_path: download.file_path.clone(),
            },
        );
    }
    map
}

/// Downloads a file from `url` and writes it to `dest`. Returns the number of bytes written.
pub(crate) async fn download_file(
    client: &http_client::Client,
    url: &str,
    dest: &Path,
) -> anyhow::Result<usize> {
    let bytes: bytes::Bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    async_fs::write(dest, &bytes).await?;
    Ok(bytes.len())
}

/// Downloads `file_downloads` (attachment id, display name pairs) from `task_id`'s task
/// attachments into `dest_dir`, best-effort: a failure looking up download URLs, creating
/// `dest_dir`, or downloading an individual file is logged and skipped rather than aborting the
/// whole batch. Callers attaching the result to a prompt should send it regardless of whether
/// this returns every attachment, some, or none -- a download failure is never a reason to drop
/// the prompt itself.
pub(crate) async fn download_task_file_attachments(
    ai_client: Arc<dyn AIClient>,
    http_client: Arc<http_client::Client>,
    task_id: AmbientAgentTaskId,
    dest_dir: PathBuf,
    file_downloads: Vec<(String, String)>,
) -> Vec<DownloadedAttachment> {
    let attachment_ids: Vec<String> = file_downloads.iter().map(|(id, _)| id.clone()).collect();
    let download_urls = match ai_client
        .download_task_attachments(&task_id, &attachment_ids)
        .await
    {
        Ok(resp) => resp
            .attachments
            .into_iter()
            .map(|att| (att.attachment_id, att.download_url))
            .collect::<HashMap<_, _>>(),
        Err(e) => {
            report_error!(
                e.context("Failed to get download URLs for task"),
                extra: { "task_id" => %task_id }
            );
            return Vec::new();
        }
    };

    if let Err(e) = async_fs::create_dir_all(&dest_dir).await {
        report_error!(anyhow::Error::new(e).context("Failed to create attachments directory"));
        return Vec::new();
    }

    let mut downloaded = Vec::new();
    for (attachment_id, file_name) in &file_downloads {
        let Some(url) = download_urls.get(attachment_id) else {
            log::warn!("No download URL for attachment {attachment_id}");
            continue;
        };
        let safe_name = sanitize_filename(file_name).to_owned();
        let dest = dest_dir.join(format!("{attachment_id}_{safe_name}"));
        match download_file(&http_client, url, &dest).await {
            Ok(_) => downloaded.push(DownloadedAttachment {
                file_id: attachment_id.clone(),
                file_name: safe_name,
                file_path: dest.to_string_lossy().into_owned(),
            }),
            Err(e) => {
                report_error!(
                    e.context("Failed to download attachment"),
                    extra: { "file_name" => %safe_name }
                );
            }
        }
    }
    downloaded
}

#[cfg(test)]
#[path = "attachment_utils_tests.rs"]
mod tests;
