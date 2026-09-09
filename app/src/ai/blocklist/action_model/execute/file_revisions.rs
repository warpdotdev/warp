use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::ai::agent::conversation::AIConversationId;

const MAX_REVISION_READ_BYTES: u32 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileRevision {
    Present {
        last_modified: Option<SystemTime>,
        content_digest: [u8; 32],
    },
    Missing,
}

impl FileRevision {
    pub(super) fn present(content: impl AsRef<[u8]>, last_modified: Option<SystemTime>) -> Self {
        Self::Present {
            last_modified,
            content_digest: Sha256::digest(content).into(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct FileRevisionTracker {
    revisions: Arc<Mutex<HashMap<AIConversationId, HashMap<String, FileRevision>>>>,
}

impl FileRevisionTracker {
    pub fn expected_revisions(
        &self,
        conversation_id: AIConversationId,
        paths: impl IntoIterator<Item = String>,
    ) -> HashMap<String, FileRevision> {
        let revisions = self.revisions.lock();
        let Some(conversation_revisions) = revisions.get(&conversation_id) else {
            return HashMap::new();
        };

        paths
            .into_iter()
            .filter_map(|path| {
                conversation_revisions
                    .get(&path)
                    .copied()
                    .map(|revision| (path, revision))
            })
            .collect()
    }

    pub fn record_revisions(
        &self,
        conversation_id: AIConversationId,
        revisions_to_record: impl IntoIterator<Item = (String, FileRevision)>,
    ) {
        self.revisions
            .lock()
            .entry(conversation_id)
            .or_default()
            .extend(revisions_to_record);
    }

    pub fn record_missing_files(
        &self,
        conversation_id: AIConversationId,
        paths: impl IntoIterator<Item = String>,
    ) {
        self.record_revisions(
            conversation_id,
            paths.into_iter().map(|path| (path, FileRevision::Missing)),
        );
    }
}

pub(super) fn read_local_revision(path: &str) -> std::io::Result<FileRevision> {
    match std::fs::read(path) {
        Ok(content) => {
            let last_modified = std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok();
            Ok(FileRevision::present(content, last_modified))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileRevision::Missing),
        Err(error) => Err(error),
    }
}

pub(super) fn read_local_revisions(
    paths: impl IntoIterator<Item = String>,
) -> Vec<(String, FileRevision)> {
    paths
        .into_iter()
        .filter_map(|path| {
            read_local_revision(&path)
                .ok()
                .map(|revision| (path, revision))
        })
        .collect()
}

pub(super) async fn read_remote_revisions(
    handle: &remote_server::manager::HostRequestHandle,
    paths: &[String],
) -> Vec<(String, FileRevision)> {
    if paths.is_empty() {
        return Vec::new();
    }

    let request = remote_server::proto::ReadFileContextRequest {
        files: paths
            .iter()
            .map(|path| remote_server::proto::ReadFileContextFile {
                path: path.clone(),
                line_ranges: vec![],
            })
            .collect(),
        max_file_bytes: Some(MAX_REVISION_READ_BYTES),
        max_batch_bytes: None,
    };
    let Ok(response) = handle.read_file_context(request).await else {
        return Vec::new();
    };

    response
        .file_contexts
        .into_iter()
        .filter_map(|context| {
            let path = context.file_name.clone();
            revision_from_remote_context(context).map(|revision| (path, revision))
        })
        .chain(response.failed_files.into_iter().filter_map(|failed| {
            let is_missing = failed
                .error
                .is_some_and(|error| is_missing_error(&error.message));
            is_missing.then_some((failed.path, FileRevision::Missing))
        }))
        .collect()
}

pub(super) fn revision_from_remote_context(
    context: remote_server::proto::FileContextProto,
) -> Option<FileRevision> {
    if context.line_range_start.is_some() || context.line_range_end.is_some() {
        return None;
    }

    let last_modified = context
        .last_modified_epoch_millis
        .map(|millis| std::time::UNIX_EPOCH + std::time::Duration::from_millis(millis));
    let revision = match context.content {
        Some(remote_server::proto::file_context_proto::Content::TextContent(content)) => {
            FileRevision::present(content, last_modified)
        }
        Some(remote_server::proto::file_context_proto::Content::BinaryContent(content)) => {
            FileRevision::present(content, last_modified)
        }
        None => FileRevision::present([], last_modified),
    };
    Some(revision)
}

fn is_missing_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("not found") || message.contains("does not exist")
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "file_revisions_tests.rs"]
mod tests;
