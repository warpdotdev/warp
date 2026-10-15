//! Turns the attachments on a prompt into ACP prompt content.
//!
//! Shared-session viewers attach context in the `AgentAttachment` wire shape; the agent only
//! understands `ContentBlock`s. Plain text is inlined as its own text block. Uploaded files are
//! downloaded next to the harness working directory and then shaped by what the agent said it
//! accepts in its `promptCapabilities`: raster images go inline when it takes images, text files
//! go inline as embedded resources when it takes embedded context, and everything else is handed
//! over as a resource link, which every ACP agent accepts. Block references need the terminal's
//! block contents and are not forwarded yet.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use futures::io::AsyncReadExt as _;
use mime_guess::mime;
use session_sharing_protocol::common::AgentAttachment;
use url::Url;
use warp_core::safe_warn;

use super::protocol::{ContentBlock, EmbeddedResource, PromptCapabilities};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::attachment_utils::{
    DownloadedAttachment, download_task_file_attachments, resolve_agent_attachments,
};
use crate::server::server_api::ai::AIClient;
use crate::util::image::{
    MAX_IMAGE_SIZE_BYTES, MIME_SNIFF_BYTES, infer_mime_type, is_supported_image_mime_type,
};

/// Largest text file embedded inline in the prompt. Larger files are linked instead, so a
/// stray log or fixture does not consume the agent's context on its own.
pub(super) const MAX_EMBEDDED_TEXT_BYTES: usize = 1024 * 1024;

/// Largest image sent inline. Matches the native agent's cap, which already accounts for the
/// base64 inflation on the wire.
pub(super) const MAX_INLINE_IMAGE_BYTES: usize = MAX_IMAGE_SIZE_BYTES;

/// Downloads a prompt's uploaded files and shapes them for the agent.
pub(super) struct AttachmentResolver {
    ai_client: Arc<dyn AIClient>,
    http_client: Arc<http_client::Client>,
    /// Uploaded files are stored against the run's task, so without one there is nothing to
    /// download them from.
    task_id: Option<AmbientAgentTaskId>,
    download_dir: PathBuf,
    /// Learned at the handshake; until then (and for agents that advertise nothing) files are
    /// shaped for a baseline agent.
    prompt_capabilities: OnceLock<PromptCapabilities>,
}

impl AttachmentResolver {
    pub(super) fn new(
        ai_client: Arc<dyn AIClient>,
        http_client: Arc<http_client::Client>,
        task_id: Option<AmbientAgentTaskId>,
        download_dir: PathBuf,
    ) -> Self {
        Self {
            ai_client,
            http_client,
            task_id,
            download_dir,
            prompt_capabilities: OnceLock::new(),
        }
    }

    /// Records what the agent accepts. Only the first call takes effect; the capabilities are
    /// fixed for the life of the connection.
    pub(super) fn set_prompt_capabilities(&self, capabilities: PromptCapabilities) {
        if self.prompt_capabilities.set(capabilities).is_err() {
            log::warn!("ACP prompt capabilities were already recorded; keeping the first set");
        }
    }

    pub(super) fn prompt_capabilities(&self) -> PromptCapabilities {
        self.prompt_capabilities.get().copied().unwrap_or_default()
    }

    /// Builds the prompt content for `text` plus `attachments`. A file that cannot be downloaded
    /// is logged and left out rather than failing the turn; the text always goes through.
    pub(super) async fn prompt_content(
        &self,
        text: String,
        attachments: Vec<AgentAttachment>,
    ) -> Vec<ContentBlock> {
        let (block_ids, selected_text, file_downloads) = resolve_agent_attachments(attachments);
        if !block_ids.is_empty() {
            log::warn!(
                "Ignoring {} block reference(s) on an ACP follow-up; block contents are not \
                 forwarded to the agent yet",
                block_ids.len()
            );
        }
        let downloaded = match (self.task_id, file_downloads.is_empty()) {
            (_, true) => Vec::new(),
            (Some(task_id), false) => {
                download_task_file_attachments(
                    self.ai_client.clone(),
                    self.http_client.clone(),
                    task_id,
                    self.download_dir.clone(),
                    file_downloads,
                )
                .await
            }
            (None, false) => {
                log::warn!(
                    "Ignoring {} file attachment(s) on an ACP follow-up; the run has no task to \
                     download them from",
                    file_downloads.len()
                );
                Vec::new()
            }
        };
        self.prompt_content_for_files(text, selected_text, &downloaded)
            .await
    }

    /// Builds the prompt content for `text` plus files that are already on disk.
    pub(super) async fn prompt_content_for_files(
        &self,
        text: String,
        selected_text: Vec<String>,
        downloaded: &[DownloadedAttachment],
    ) -> Vec<ContentBlock> {
        let capabilities = self.prompt_capabilities();
        let mut files = Vec::with_capacity(downloaded.len());
        for file in downloaded {
            files.push(ResolvedFile::read(file, capabilities).await);
        }
        prompt_content(capabilities, text, selected_text, &files)
    }
}

/// A downloaded file with what shaping needs to know about it.
#[derive(Debug)]
pub(super) struct ResolvedFile {
    pub name: String,
    pub path: PathBuf,
    /// Sniffed from the leading bytes when they were read, otherwise guessed from the name.
    pub mime_type: String,
    /// `None` when the file was not read: the agent accepts nothing of its kind inline, it is
    /// too large to inline, or reading failed. Such a file travels as a link.
    pub contents: Option<Vec<u8>>,
}

impl ResolvedFile {
    /// Reads the file only when `capabilities` and its size allow it to be sent inline, so a
    /// batch of large attachments that will only be linked is never held in memory.
    async fn read(file: &DownloadedAttachment, capabilities: PromptCapabilities) -> Self {
        let path = PathBuf::from(&file.file_path);
        let largest_cap = [
            capabilities.image.then_some(MAX_INLINE_IMAGE_BYTES),
            capabilities
                .embedded_context
                .then_some(MAX_EMBEDDED_TEXT_BYTES),
        ]
        .into_iter()
        .flatten()
        .max();
        let inlineable = match largest_cap {
            Some(largest_cap) => match read_if_inlineable(&path, largest_cap, capabilities).await {
                Ok(inlineable) => inlineable,
                Err(error) => {
                    safe_warn!(
                        safe: ("Failed to read an ACP attachment; sending it as a resource link"),
                        full: (
                            "Failed to read ACP attachment {}; sending it as a resource link: {error}",
                            file.file_name
                        )
                    );
                    None
                }
            },
            None => None,
        };
        let (mime_type, contents) = match inlineable {
            Some((mime_type, contents)) => (mime_type, Some(contents)),
            None => (infer_mime_type(&path, &[]), None),
        };
        Self {
            name: file.file_name.clone(),
            path,
            mime_type,
            contents,
        }
    }

    fn link(&self) -> ContentBlock {
        ContentBlock::ResourceLink {
            uri: file_uri(&self.path),
            name: Some(self.name.clone()),
        }
    }
}

/// Reads `path` when the agent accepts its kind inline and it fits that kind's cap, returning
/// its sniffed MIME type and bytes. `None` means it should be linked instead. The size is
/// checked against `largest_cap` before anything is read, and only the sniff prefix is read
/// before the kind's own cap is known; the final read is bounded in case the file grew.
async fn read_if_inlineable(
    path: &Path,
    largest_cap: usize,
    capabilities: PromptCapabilities,
) -> io::Result<Option<(String, Vec<u8>)>> {
    let len = async_fs::metadata(path).await?.len();
    if len > largest_cap as u64 {
        return Ok(None);
    }
    let mut file = async_fs::File::open(path).await?;
    let mut contents = Vec::new();
    (&mut file)
        .take(MIME_SNIFF_BYTES as u64)
        .read_to_end(&mut contents)
        .await?;
    let mime_type = infer_mime_type(path, &contents);
    let cap = if is_supported_image_mime_type(&mime_type) {
        capabilities.image.then_some(MAX_INLINE_IMAGE_BYTES)
    } else {
        capabilities
            .embedded_context
            .then_some(MAX_EMBEDDED_TEXT_BYTES)
    };
    let Some(cap) = cap.filter(|cap| len <= *cap as u64) else {
        return Ok(None);
    };
    file.take((cap + 1).saturating_sub(contents.len()) as u64)
        .read_to_end(&mut contents)
        .await?;
    if contents.len() > cap {
        return Ok(None);
    }
    Ok(Some((mime_type, contents)))
}

/// Orders the prompt as the user's text followed by each attachment, so agents that
/// concatenate text blocks read the instruction before its supporting context.
pub(super) fn prompt_content(
    capabilities: PromptCapabilities,
    text: String,
    selected_text: Vec<String>,
    files: &[ResolvedFile],
) -> Vec<ContentBlock> {
    let mut blocks = vec![ContentBlock::Text { text }];
    blocks.extend(selected_text.into_iter().map(|content| ContentBlock::Text {
        text: format!("<attached_text>\n{content}\n</attached_text>"),
    }));
    blocks.extend(files.iter().map(|file| file_block(capabilities, file)));
    blocks
}

/// The block a file travels as: inline when the agent accepts that kind of content and the
/// file is small enough, a resource link otherwise.
pub(super) fn file_block(capabilities: PromptCapabilities, file: &ResolvedFile) -> ContentBlock {
    let Some(contents) = &file.contents else {
        return file.link();
    };
    if is_supported_image_mime_type(&file.mime_type) {
        if capabilities.image && contents.len() <= MAX_INLINE_IMAGE_BYTES {
            return ContentBlock::Image {
                data: base64::engine::general_purpose::STANDARD.encode(contents),
                mime_type: file.mime_type.clone(),
            };
        }
        return file.link();
    }
    if !capabilities.embedded_context || contents.len() > MAX_EMBEDDED_TEXT_BYTES {
        return file.link();
    }
    let Ok(text) = std::str::from_utf8(contents) else {
        return file.link();
    };
    ContentBlock::Resource {
        resource: EmbeddedResource {
            uri: file_uri(&file.path),
            mime_type: Some(text_mime_type(&file.mime_type)),
            text: Some(text.to_owned()),
            blob: None,
        },
    }
}

/// A file whose bytes are valid UTF-8 but whose name gives no better hint is declared plain
/// text, so the agent does not treat an embedded `text` as opaque.
fn text_mime_type(mime_type: &str) -> String {
    if mime_type == mime::APPLICATION_OCTET_STREAM.as_ref() {
        mime::TEXT_PLAIN.to_string()
    } else {
        mime_type.to_owned()
    }
}

/// `Url::from_file_path` only accepts absolute paths; downloads always land under the absolute
/// harness working directory, so the fallback exists for callers handing in something else.
fn file_uri(path: &Path) -> String {
    Url::from_file_path(path)
        .map(String::from)
        .unwrap_or_else(|()| format!("file://{}", path.display()))
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
