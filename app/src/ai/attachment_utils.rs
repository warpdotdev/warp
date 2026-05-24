//! Shared utilities for file-attachment handling (download, filename sanitization,
//! and building attachment maps).
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Max file attachment size is 10 MB.
pub(crate) const MAX_ATTACHMENT_SIZE_BYTES: usize = 10 * 1024 * 1024;

use crate::ai::agent::AIAgentAttachment;

/// Returns the per-session directory for downloading file attachments,
/// based on the agent's working directory.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn attachments_download_dir(working_dir: &Path) -> PathBuf {
    working_dir.join(".black").join("attachments")
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
