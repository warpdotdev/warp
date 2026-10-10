//! Turns the attachments on a follow-up prompt into ACP prompt content.
//!
//! Shared-session viewers attach context in the `AgentAttachment` wire shape; the agent only
//! understands `ContentBlock`s. Plain text is inlined as its own text block, and uploaded files
//! are downloaded next to the harness working directory and handed over as resource links, which
//! every ACP agent accepts (unlike inline images, which agents opt into through
//! `promptCapabilities`). Block references need the terminal's block contents and are not
//! forwarded yet.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use session_sharing_protocol::common::AgentAttachment;
use url::Url;

use super::protocol::ContentBlock;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::attachment_utils::{
    DownloadedAttachment, download_task_file_attachments, resolve_agent_attachments,
};
use crate::server::server_api::ai::AIClient;

/// Downloads a follow-up's uploaded files so they can be referenced from the prompt.
pub(super) struct AttachmentResolver {
    ai_client: Arc<dyn AIClient>,
    http_client: Arc<http_client::Client>,
    /// Uploaded files are stored against the run's task, so without one there is nothing to
    /// download them from.
    task_id: Option<AmbientAgentTaskId>,
    download_dir: PathBuf,
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
        }
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
        prompt_content(text, selected_text, &downloaded)
    }
}

/// Orders the prompt as the user's text followed by each attachment, so agents that
/// concatenate text blocks read the instruction before its supporting context.
pub(super) fn prompt_content(
    text: String,
    selected_text: Vec<String>,
    downloaded: &[DownloadedAttachment],
) -> Vec<ContentBlock> {
    let mut blocks = vec![ContentBlock::Text { text }];
    blocks.extend(selected_text.into_iter().map(|content| ContentBlock::Text {
        text: format!("<attached_text>\n{content}\n</attached_text>"),
    }));
    blocks.extend(downloaded.iter().map(|file| ContentBlock::ResourceLink {
        uri: file_uri(Path::new(&file.file_path)),
        name: Some(file.file_name.clone()),
    }));
    blocks
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
